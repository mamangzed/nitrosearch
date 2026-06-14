//! Segment Manager - orchestrates segment lifecycle and merging
//!
//! Responsibilities:
//! - Manage multiple immutable segments
//! - Decide when to create new segments
//! - Merge small segments in background
//! - Provide unified search interface across all segments

use crate::segment::{Segment, SegmentError};
use crate::wal::WalWriter;
use nitro_core::Document;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

#[derive(Error, Debug)]
pub enum SegmentManagerError {
    #[error("Segment error: {0}")]
    Segment(#[from] SegmentError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Segment not found: {0}")]
    NotFound(String),
    #[error("Merge error: {0}")]
    MergeError(String),
}

/// Configuration for merge policy
#[derive(Debug, Clone)]
pub struct MergePolicy {
    /// Minimum number of segments before triggering merge
    pub min_merge_segments: usize,
    /// Maximum number of segments before forcing merge
    pub max_merge_segments: usize,
    /// Target segment size in MB
    pub target_segment_size_mb: u64,
    /// Merge factor (how many segments to merge at once)
    pub merge_factor: usize,
}

impl Default for MergePolicy {
    fn default() -> Self {
        Self {
            min_merge_segments: 5,
            max_merge_segments: 20,
            target_segment_size_mb: 512,
            merge_factor: 10,
        }
    }
}

/// Manages multiple segments and handles merging
pub struct SegmentManager {
    /// Directory where segments are stored
    data_dir: PathBuf,

    /// All active segments (read-mostly)
    segments: RwLock<Vec<Arc<Segment>>>,

    /// WAL for crash recovery
    #[allow(dead_code)]
    wal: RwLock<WalWriter>,

    /// Merge policy configuration
    merge_policy: MergePolicy,

    /// Background merge worker handle
    merge_handle: RwLock<Option<JoinHandle<()>>>,

    /// Background flush worker handle
    flush_handle: RwLock<Option<JoinHandle<()>>>,

    /// Shutdown signal for background tasks
    shutdown_tx: broadcast::Sender<()>,

    /// Current segment ID counter
    next_segment_id: RwLock<u64>,

    /// Buffer for documents before flush
    buffer: RwLock<HashMap<String, (Document, Vec<String>)>>,

    /// Flush threshold (number of documents)
    flush_threshold: usize,

    /// Flush interval (seconds)
    flush_interval_secs: u64,

    /// Memory limit (bytes)
    memory_limit_bytes: usize,
}

impl SegmentManager {
    /// Create a new segment manager
    pub fn new(data_dir: PathBuf, merge_policy: MergePolicy) -> Result<Self, SegmentManagerError> {
        fs::create_dir_all(&data_dir)?;

        let wal_path = data_dir.join("wal.log");
        let wal = WalWriter::open(&wal_path)
            .map_err(|e| SegmentManagerError::MergeError(e.to_string()))?;

        let (shutdown_tx, _) = broadcast::channel(16);

        let manager = Self {
            data_dir,
            segments: RwLock::new(Vec::new()),
            wal: RwLock::new(wal),
            merge_policy,
            merge_handle: RwLock::new(None),
            flush_handle: RwLock::new(None),
            shutdown_tx,
            next_segment_id: RwLock::new(1),
            buffer: RwLock::new(HashMap::new()),
            flush_threshold: 10000,
            flush_interval_secs: 5,
            memory_limit_bytes: 1024 * 1024 * 1024, // 1GB
        };

        // Load existing segments
        manager.load_segments()?;

        Ok(manager)
    }

    /// Load all segments from disk
    fn load_segments(&self) -> Result<(), SegmentManagerError> {
        let mut segments = Vec::new();
        let mut max_id = 0u64;

        for entry in fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir()
                && path
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("segment_")
            {
                match Segment::open(path.clone()) {
                    Ok(segment) => {
                        let segment_id = segment.id();
                        if segment_id > max_id {
                            max_id = segment_id;
                        }
                        info!("Loaded segment {}", segment_id);
                        segments.push(Arc::new(segment));
                    }
                    Err(e) => {
                        warn!("Failed to load segment {:?}: {}", path, e);
                    }
                }
            }
        }

        // Sort segments by ID (oldest first)
        segments.sort_by_key(|s| s.id());

        *self.next_segment_id.write().unwrap() = max_id + 1;
        *self.segments.write().unwrap() = segments;

        Ok(())
    }

    /// Get all active segments
    pub fn segments(&self) -> Vec<Arc<Segment>> {
        self.segments.read().unwrap().clone()
    }

    /// Get total document count across all segments
    pub fn total_doc_count(&self) -> u64 {
        self.segments().iter().map(|s| s.doc_count()).sum()
    }

    /// Add a new segment
    pub fn add_segment(&self, segment: Segment) -> Result<(), SegmentManagerError> {
        let segment_id = segment.id();
        info!("Adding new segment {}", segment_id);

        self.segments.write().unwrap().push(Arc::new(segment));

        // Check if merge is needed
        self.maybe_trigger_merge()?;

        Ok(())
    }

    /// Get next segment ID
    pub fn next_segment_id(&self) -> u64 {
        let mut id = self.next_segment_id.write().unwrap();
        let current = *id;
        *id += 1;
        current
    }

    /// Check if merge should be triggered based on policy
    fn maybe_trigger_merge(&self) -> Result<(), SegmentManagerError> {
        let segments = self.segments.read().unwrap();
        let segment_count = segments.len();

        if segment_count >= self.merge_policy.max_merge_segments {
            warn!(
                "Segment count {} exceeded max {}, forcing merge",
                segment_count, self.merge_policy.max_merge_segments
            );
            drop(segments);
            self.trigger_merge()?;
        } else if segment_count >= self.merge_policy.min_merge_segments {
            debug!(
                "Segment count {} reached threshold {}, scheduling merge",
                segment_count, self.merge_policy.min_merge_segments
            );
            drop(segments);
            self.trigger_merge()?;
        }

        Ok(())
    }

    /// Trigger a merge operation
    fn trigger_merge(&self) -> Result<(), SegmentManagerError> {
        let segments = self.segments();

        if segments.len() < 2 {
            return Ok(());
        }

        // Select segments to merge (smallest first, up to merge_factor)
        let mut candidates: Vec<_> = segments
            .iter()
            .filter(|s| s.doc_count() > 0)
            .cloned()
            .collect();

        candidates.sort_by_key(|s| s.doc_count());
        candidates.truncate(self.merge_policy.merge_factor);

        if candidates.len() < 2 {
            return Ok(());
        }

        info!("Merging {} segments", candidates.len());

        // Create new merged segment
        let new_segment_id = self.next_segment_id();
        let new_segment_path = self.data_dir.join(format!("segment_{}", new_segment_id));

        let merged = Self::merge_segments(&candidates, new_segment_id, new_segment_path)?;

        // Replace old segments with merged one
        let old_ids: Vec<_> = candidates.iter().map(|s| s.id()).collect();

        {
            let mut segments = self.segments.write().unwrap();
            segments.retain(|s| !old_ids.contains(&s.id()));
            segments.push(Arc::new(merged));
        }

        // Clean up old segment directories
        for old_id in old_ids {
            let old_path = self.data_dir.join(format!("segment_{}", old_id));
            if old_path.exists() {
                fs::remove_dir_all(&old_path)?;
                info!("Removed old segment {}", old_id);
            }
        }

        Ok(())
    }

    /// Merge multiple segments into one
    fn merge_segments(
        segments: &[Arc<Segment>],
        new_id: u64,
        new_path: PathBuf,
    ) -> Result<Segment, SegmentManagerError> {
        // Collect all terms and postings from all segments
        let mut all_terms: HashMap<String, Vec<u32>> = HashMap::new();
        let mut all_docs: HashMap<u32, Vec<u8>> = HashMap::new();

        for segment in segments {
            // Get all terms from this segment
            let terms = segment.all_terms();

            for term in terms {
                let postings = segment
                    .search_term(&term)?
                    .ok_or_else(|| SegmentError::NotFound(format!("Term not found: {}", term)))?;

                all_terms.entry(term).or_default().extend(postings);
            }

            // Get all documents (excluding deleted)
            let doc_ids = segment.all_doc_ids();
            for doc_id in doc_ids {
                if !segment.is_deleted(doc_id) {
                    if let Some(doc_data) = segment.get_document(doc_id)? {
                        // Use new sequential ID
                        let new_doc_id = all_docs.len() as u32 + 1;
                        all_docs.insert(new_doc_id, doc_data);
                    }
                }
            }
        }

        // Build new segment
        let mut builder = Segment::builder(new_id, new_path);

        // Add all terms with remapped doc IDs
        for (term, old_doc_ids) in all_terms {
            let mut positions = Vec::new();
            let mut new_doc_ids = Vec::new();

            // Simple remapping: maintain order
            for (i, _) in old_doc_ids.iter().enumerate() {
                new_doc_ids.push(i as u32 + 1);
                positions.push(vec![]); // Positions would need more complex remapping
            }

            builder.add_posting(term, new_doc_ids, positions);
        }

        // Add all documents
        for (doc_id, data) in all_docs {
            // Use doc_id as string for mapping
            builder.add_stored_doc(doc_id, doc_id.to_string(), data);
        }

        let segment = builder.build()?;
        info!(
            "Created merged segment {} with {} docs",
            new_id,
            segment.doc_count()
        );

        Ok(segment)
    }

    /// Start background merge worker
    pub fn start_merge_worker(&self) -> Result<(), SegmentManagerError> {
        let mut handle = self.merge_handle.write().unwrap();

        if handle.is_some() {
            return Ok(()); // Already running
        }

        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let _data_dir = self.data_dir.clone();

        let task = tokio::spawn(async move {
            info!("Background merge worker started");

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Background merge worker shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(60)) => {
                        // Periodic merge check would go here
                        debug!("Periodic merge check");
                    }
                }
            }
        });

        *handle = Some(task);
        Ok(())
    }

    /// Stop background merge worker
    pub fn stop_merge_worker(&self) {
        let _ = self.shutdown_tx.send(());

        if let Some(handle) = self.merge_handle.write().unwrap().take() {
            handle.abort();
        }
    }

    /// Start background flush worker
    pub fn start_flush_worker(&self) -> Result<(), SegmentManagerError> {
        let mut handle = self.flush_handle.write().unwrap();

        if handle.is_some() {
            return Ok(()); // Already running
        }

        let shutdown_rx = self.shutdown_tx.subscribe();
        let _flush_threshold = self.flush_threshold;
        let flush_interval = self.flush_interval_secs;
        let _memory_limit = self.memory_limit_bytes;

        let task = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(flush_interval));
            let mut shutdown_rx = shutdown_rx;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Check if buffer needs flushing
                        // For now, just a placeholder
                        debug!("Periodic flush check");
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Background flush worker shutting down");
                        break;
                    }
                }
            }
        });

        *handle = Some(task);
        Ok(())
    }

    /// Stop all background workers
    pub fn stop_workers(&self) {
        self.stop_merge_worker();

        let _ = self.shutdown_tx.send(());

        if let Some(handle) = self.flush_handle.write().unwrap().take() {
            handle.abort();
        }
    }

    /// Get buffer for documents before flush
    pub fn buffer(&self) -> &RwLock<HashMap<String, (Document, Vec<String>)>> {
        &self.buffer
    }

    /// Get segments lock
    pub fn segments_lock(&self) -> &RwLock<Vec<Arc<Segment>>> {
        &self.segments
    }

    /// Get data directory
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Batch insert multiple documents efficiently
    pub fn batch_insert(
        &self,
        docs: Vec<(Document, Vec<String>)>,
    ) -> Result<(), SegmentManagerError> {
        if docs.is_empty() {
            return Ok(());
        }

        let mut buffer = self.buffer.write().unwrap();

        // Reserve capacity to avoid reallocations
        buffer.reserve(docs.len());

        // Batch insert into buffer
        for (doc, tokens) in docs {
            buffer.insert(doc.id.clone(), (doc, tokens));
        }

        let buffer_size = buffer.len();
        debug!("Batch insert completed, buffer size: {}", buffer_size);

        // Check if we should trigger a flush
        if buffer_size >= self.flush_threshold {
            info!(
                "Buffer reached threshold ({} docs), triggering flush",
                buffer_size
            );
            drop(buffer); // Release lock before flush
            self.flush_buffer()?;
        }

        Ok(())
    }

    /// Flush buffer to a new segment
    pub fn flush_buffer(&self) -> Result<(), SegmentManagerError> {
        let mut buffer = self.buffer.write().unwrap();

        if buffer.is_empty() {
            return Ok(());
        }

        let segment_id = self.next_segment_id();
        let segment_path = self.data_dir.join(format!("segment_{}", segment_id));

        info!(
            "Flushing {} documents to segment {}",
            buffer.len(),
            segment_id
        );

        // Build segment from buffer
        let mut builder = crate::segment::SegmentBuilder::new(segment_id, segment_path);

        // Collect all documents and build inverted index
        let mut term_postings: std::collections::HashMap<String, Vec<u32>> =
            std::collections::HashMap::new();

        for (doc_num, (doc_id, (doc, tokens))) in buffer.drain().enumerate() {
            let doc_num = doc_num as u32;

            // Serialize and store document (use JSON + zstd compression)
            let doc_json = serde_json::to_vec(&doc).unwrap_or_default();
            let doc_bytes = zstd::encode_all(&doc_json[..], 3).unwrap_or_default();
            builder.add_stored_doc(doc_num, doc_id, doc_bytes);

            // Build postings for each term
            for token in tokens {
                term_postings.entry(token).or_default().push(doc_num);
            }
        }

        // Add postings to segment
        for (term, mut doc_ids) in term_postings {
            // Sort doc IDs for delta encoding efficiency
            doc_ids.sort_unstable();
            doc_ids.dedup();
            builder.add_posting(term, doc_ids, vec![]);
        }

        // Build segment on disk
        let segment = builder.build()?;

        // Add segment to manager
        drop(buffer); // Release lock before adding segment
        self.add_segment(segment)?;

        info!("Segment {} flushed successfully", segment_id);
        Ok(())
    }

    /// Get current buffer size
    pub fn buffer_size(&self) -> usize {
        self.buffer.read().unwrap().len()
    }

    /// Check if buffer should be flushed
    pub fn should_flush(&self) -> bool {
        self.buffer.read().unwrap().len() >= self.flush_threshold
    }
}

impl Drop for SegmentManager {
    fn drop(&mut self) {
        self.stop_workers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::SegmentBuilder;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_segment_manager_basic() {
        let temp_dir = TempDir::new().unwrap();
        let manager =
            SegmentManager::new(temp_dir.path().to_path_buf(), MergePolicy::default()).unwrap();

        assert_eq!(manager.segments().len(), 0);
        assert_eq!(manager.total_doc_count(), 0);
    }

    #[tokio::test]
    async fn test_add_segment() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SegmentManager::new(
            temp_dir.path().to_path_buf(),
            MergePolicy {
                min_merge_segments: 100, // High threshold to avoid auto-merge
                ..Default::default()
            },
        )
        .unwrap();

        // Create a test segment
        let segment_path = temp_dir.path().join("segment_1");
        let mut builder = SegmentBuilder::new(1, segment_path);
        builder.add_posting("test".to_string(), vec![1], vec![vec![]]);
        builder.add_stored_doc(1, "doc1".to_string(), b"doc1".to_vec());
        let segment = builder.build().unwrap();

        manager.add_segment(segment).unwrap();

        assert_eq!(manager.segments().len(), 1);
        assert_eq!(manager.total_doc_count(), 1);
    }
}
