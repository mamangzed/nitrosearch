//! Immutable disk-backed segment storage
//!
//! Segments are immutable after creation and memory-mapped for efficient reads.
//! Each segment contains:
//! - Term dictionary (sorted for binary search)
//! - Compressed posting lists (delta + VarInt)
//! - Stored document fields
//! - Segment metadata (doc count, field statistics)
//! - Deleted document bitmap (RoaringBitmap)

use crate::compression::{delta_decode, delta_encode};
use crate::mmap::MmapReader;
#[cfg(unix)]
#[cfg(unix)]
use memmap2::Advice;
use roaring::RoaringBitmap;
use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SegmentError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Mmap error: {0}")]
    Mmap(#[from] crate::mmap::MmapError),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Segment not found: {0}")]
    NotFound(String),
    #[error("Invalid segment format")]
    InvalidFormat,
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Segment metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SegmentMeta {
    pub id: u64,
    pub doc_count: u64,
    pub term_count: u64,
    pub total_field_lengths: HashMap<String, u64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Term posting list
#[derive(Debug, Clone)]
pub struct PostingList {
    pub term: String,
    pub doc_ids: Vec<u32>,
    pub positions: Vec<Vec<u32>>, // positions per doc
}

/// Immutable segment stored on disk
pub struct Segment {
    meta: SegmentMeta,
    path: PathBuf,

    // Memory-mapped files for zero-copy access
    #[allow(dead_code)]
    terms_mmap: Option<MmapReader>,
    postings_mmap: Option<MmapReader>,
    stored_mmap: Option<MmapReader>,

    // Deleted documents (mutable)
    deleted: RoaringBitmap,

    // Term dictionary for fast lookup
    term_index: HashMap<String, TermInfo>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TermInfo {
    term: String,
    doc_freq: u64,
    postings_offset: u64,
    postings_len: u64,
}

impl Segment {
    /// Create a new segment builder
    pub fn builder(id: u64, path: PathBuf) -> SegmentBuilder {
        SegmentBuilder::new(id, path)
    }

    /// Open an existing segment from disk
    pub fn open(path: PathBuf) -> Result<Self, SegmentError> {
        let meta_path = path.join("meta.json");
        let meta_data = fs::read_to_string(&meta_path)?;
        let meta: SegmentMeta = serde_json::from_str(&meta_data)
            .map_err(|e| SegmentError::Serialization(e.to_string()))?;

        let terms_path = path.join("terms.bin");
        let postings_path = path.join("postings.bin");
        let stored_path = path.join("stored.bin");
        let deleted_path = path.join("deleted.bitmap");

        // Memory-map the files
        let terms_mmap = if terms_path.exists() {
            Some(MmapReader::open(&terms_path)?)
        } else {
            None
        };

        let postings_mmap = if postings_path.exists() {
            Some(MmapReader::open(&postings_path)?)
        } else {
            None
        };

        let stored_mmap = if stored_path.exists() {
            Some(MmapReader::open(&stored_path)?)
        } else {
            None
        };

        // Load deleted bitmap
        let deleted = if deleted_path.exists() {
            let data = fs::read(&deleted_path)?;
            RoaringBitmap::deserialize_from(&data[..])
                .map_err(|e| SegmentError::Serialization(e.to_string()))?
        } else {
            RoaringBitmap::new()
        };

        // Build term index
        let term_index = if let Some(ref mmap) = terms_mmap {
            Self::load_term_index(mmap)?
        } else {
            HashMap::new()
        };

        // Advise OS about access pattern (Unix only)
        #[cfg(unix)]
        {
            if let Some(ref mmap) = terms_mmap {
                mmap.advise(Advice::Random)?;
            }
            if let Some(ref mmap) = postings_mmap {
                mmap.advise(Advice::Sequential)?;
            }
            if let Some(ref mmap) = stored_mmap {
                mmap.advise(Advice::Random)?;
            }
        }

        Ok(Self {
            meta,
            path,
            terms_mmap,
            postings_mmap,
            stored_mmap,
            deleted,
            term_index,
        })
    }

    fn load_term_index(mmap: &MmapReader) -> Result<HashMap<String, TermInfo>, SegmentError> {
        let size = mmap.len();
        if size < 8 {
            return Ok(HashMap::new());
        }

        // Read term count from end of file
        let term_count = mmap.read_u64(size - 8)? as usize;

        // Read term index entries
        let mut index = HashMap::new();
        let mut offset = 0;

        for _ in 0..term_count {
            // Read term length
            let term_len = mmap.read_u32(offset)? as usize;
            offset += 4;

            // Read term string
            let term_bytes = mmap.read_bytes(offset, term_len)?;
            let term = String::from_utf8_lossy(term_bytes).to_string();
            offset += term_len;

            // Read doc_freq
            let doc_freq = mmap.read_u64(offset)?;
            offset += 8;

            // Read postings offset and length
            let postings_offset = mmap.read_u64(offset)?;
            offset += 8;

            let postings_len = mmap.read_u64(offset)?;
            offset += 8;

            index.insert(
                term.clone(),
                TermInfo {
                    term,
                    doc_freq,
                    postings_offset,
                    postings_len,
                },
            );
        }

        Ok(index)
    }

    pub fn id(&self) -> u64 {
        self.meta.id
    }

    pub fn doc_count(&self) -> u64 {
        self.meta.doc_count - self.deleted.len()
    }

    pub fn term_count(&self) -> u64 {
        self.meta.term_count
    }

    pub fn total_field_length(&self, field: &str) -> u64 {
        self.meta
            .total_field_lengths
            .get(field)
            .copied()
            .unwrap_or(0)
    }

    /// Search for a term and return its posting list
    pub fn search_term(&self, term: &str) -> Result<Option<Vec<u32>>, SegmentError> {
        tracing::debug!("Segment search_term: looking for '{}'", term);
        let term_info = match self.term_index.get(term) {
            Some(info) => {
                tracing::debug!(
                    "Found term '{}' in index, offset: {}, len: {}",
                    term,
                    info.postings_offset,
                    info.postings_len
                );
                info
            }
            None => {
                tracing::debug!("Term '{}' NOT found in segment index", term);
                return Ok(None);
            }
        };

        let postings_mmap = match &self.postings_mmap {
            Some(mmap) => mmap,
            None => return Ok(None),
        };

        // Read compressed posting list
        let compressed = postings_mmap.read_bytes(
            term_info.postings_offset as usize,
            term_info.postings_len as usize,
        )?;

        // Decode posting list
        let doc_ids = delta_decode(compressed);

        // Filter out deleted documents
        let doc_ids: Vec<u32> = doc_ids
            .into_iter()
            .filter(|&doc_id| !self.deleted.contains(doc_id))
            .collect();

        Ok(Some(doc_ids))
    }

    /// Mark a document as deleted
    pub fn delete_document(&mut self, doc_id: u32) {
        self.deleted.insert(doc_id);
    }

    /// Check if a document is deleted
    pub fn is_deleted(&self, doc_id: u32) -> bool {
        self.deleted.contains(doc_id)
    }

    /// Get all terms in this segment
    pub fn all_terms(&self) -> Vec<String> {
        self.term_index.keys().cloned().collect()
    }

    /// Get all document IDs in this segment (excluding deleted)
    pub fn all_doc_ids(&self) -> Vec<u32> {
        let mut ids = Vec::new();
        let mmap = match &self.stored_mmap {
            Some(m) => m,
            None => return ids,
        };
        let len = mmap.len();
        let mut offset = 0;
        while offset + 8 <= len {
            if let Ok(doc_id) = mmap.read_u32(offset) {
                if let Ok(data_len) = mmap.read_u32(offset + 4) {
                    ids.push(doc_id);
                    offset += 8 + data_len as usize;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        ids
    }

    /// Get a document by ID
    pub fn get_document(&self, doc_id: u32) -> Result<Option<Vec<u8>>, SegmentError> {
        let mmap = match &self.stored_mmap {
            Some(m) => m,
            None => return Ok(None),
        };
        let len = mmap.len();
        let mut offset = 0;
        while offset + 8 <= len {
            let id = mmap.read_u32(offset)?;
            let data_len = mmap.read_u32(offset + 4)? as usize;
            if id == doc_id {
                let data = mmap.read_bytes(offset + 8, data_len)?;
                return Ok(Some(data.to_vec()));
            }
            offset += 8 + data_len;
        }
        Ok(None)
    }

    /// Persist deleted bitmap to disk
    pub fn persist_deletes(&self) -> Result<(), SegmentError> {
        let deleted_path = self.path.join("deleted.bitmap");
        let mut data = Vec::new();
        self.deleted
            .serialize_into(&mut data)
            .map_err(|e| SegmentError::Serialization(e.to_string()))?;
        fs::write(deleted_path, data)?;
        Ok(())
    }

    /// Get the path to this segment
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Builder for creating new segments
pub struct SegmentBuilder {
    id: u64,
    path: PathBuf,
    postings: Vec<PostingList>,
    stored_docs: Vec<(u32, Vec<u8>)>,
    field_lengths: HashMap<String, u64>,
    doc_id_mapping: HashMap<String, u32>, // doc_id string -> doc_num u32
}

impl SegmentBuilder {
    pub fn new(id: u64, path: PathBuf) -> Self {
        Self {
            id,
            path,
            postings: Vec::new(),
            stored_docs: Vec::new(),
            field_lengths: HashMap::new(),
            doc_id_mapping: HashMap::new(),
        }
    }

    /// Add a posting list
    pub fn add_posting(&mut self, term: String, doc_ids: Vec<u32>, positions: Vec<Vec<u32>>) {
        self.postings.push(PostingList {
            term,
            doc_ids,
            positions,
        });
    }

    /// Add a stored document with original doc_id mapping
    pub fn add_stored_doc(&mut self, doc_id: u32, doc_id_str: String, data: Vec<u8>) {
        self.stored_docs.push((doc_id, data));
        self.doc_id_mapping.insert(doc_id_str, doc_id);
    }

    /// Add field length statistic
    pub fn add_field_length(&mut self, field: &str, length: u64) {
        *self.field_lengths.entry(field.to_string()).or_insert(0) += length;
    }

    /// Build and write segment to disk
    pub fn build(self) -> Result<Segment, SegmentError> {
        fs::create_dir_all(&self.path)?;

        let _meta = self.write_segment()?;
        Segment::open(self.path)
    }

    fn write_segment(&self) -> Result<SegmentMeta, SegmentError> {
        // Write postings
        let postings_path = self.path.join("postings.bin");
        let mut postings_file = BufWriter::new(fs::File::create(&postings_path)?);
        let mut postings_offsets: HashMap<String, (u64, u64)> = HashMap::new();

        let mut offset = 0u64;
        for posting in &self.postings {
            let compressed = delta_encode(&posting.doc_ids);
            postings_file.write_all(&compressed)?;

            postings_offsets.insert(posting.term.clone(), (offset, compressed.len() as u64));

            offset += compressed.len() as u64;
        }
        postings_file.flush()?;

        // Write stored documents
        let stored_path = self.path.join("stored.bin");
        let mut stored_file = BufWriter::new(fs::File::create(&stored_path)?);

        for (doc_id, data) in &self.stored_docs {
            stored_file.write_all(&doc_id.to_le_bytes())?;
            stored_file.write_all(&(data.len() as u32).to_le_bytes())?;
            stored_file.write_all(data)?;
        }
        stored_file.flush()?;

        // Write term dictionary with index
        let terms_path = self.path.join("terms.bin");
        let mut terms_file = BufWriter::new(fs::File::create(&terms_path)?);

        for posting in &self.postings {
            let (postings_offset, postings_len) = postings_offsets[&posting.term];

            // Write term length
            terms_file.write_all(&(posting.term.len() as u32).to_le_bytes())?;
            // Write term
            terms_file.write_all(posting.term.as_bytes())?;
            // Write doc_freq
            terms_file.write_all(&(posting.doc_ids.len() as u64).to_le_bytes())?;
            // Write postings offset and length
            terms_file.write_all(&postings_offset.to_le_bytes())?;
            terms_file.write_all(&postings_len.to_le_bytes())?;
        }

        // Write term count at end
        terms_file.write_all(&(self.postings.len() as u64).to_le_bytes())?;
        terms_file.flush()?;

        // Create metadata
        let meta = SegmentMeta {
            id: self.id,
            doc_count: self.stored_docs.len() as u64,
            term_count: self.postings.len() as u64,
            total_field_lengths: self.field_lengths.clone(),
            created_at: chrono::Utc::now(),
        };

        let meta_path = self.path.join("meta.json");
        fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

        Ok(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_segment_builder() {
        let temp_dir = TempDir::new().unwrap();
        let segment_path = temp_dir.path().join("segment_001");

        let mut builder = Segment::builder(1, segment_path.clone());

        builder.add_posting(
            "hello".to_string(),
            vec![1, 2, 3],
            vec![vec![0], vec![0], vec![0]],
        );

        builder.add_stored_doc(1, "doc1".to_string(), b"doc1 data".to_vec());
        builder.add_stored_doc(2, "doc2".to_string(), b"doc2 data".to_vec());
        builder.add_stored_doc(3, "doc3".to_string(), b"doc3 data".to_vec());

        builder.add_field_length("title", 100);

        let segment = builder.build().unwrap();

        assert_eq!(segment.id(), 1);
        assert_eq!(segment.doc_count(), 3);
        assert_eq!(segment.term_count(), 1);

        let doc_ids = segment.search_term("hello").unwrap().unwrap();
        assert_eq!(doc_ids, vec![1, 2, 3]);
    }

    #[test]
    fn test_segment_delete() {
        let temp_dir = TempDir::new().unwrap();
        let segment_path = temp_dir.path().join("segment_002");

        let mut builder = Segment::builder(2, segment_path.clone());

        builder.add_posting("world".to_string(), vec![1, 2, 3], vec![]);
        builder.add_stored_doc(1, "doc1".to_string(), b"doc1".to_vec());
        builder.add_stored_doc(2, "doc2".to_string(), b"doc2".to_vec());
        builder.add_stored_doc(3, "doc3".to_string(), b"doc3".to_vec());

        let mut segment = builder.build().unwrap();

        assert_eq!(segment.doc_count(), 3);

        segment.delete_document(2);
        assert_eq!(segment.doc_count(), 2);

        let doc_ids = segment.search_term("world").unwrap().unwrap();
        assert_eq!(doc_ids, vec![1, 3]);

        segment.persist_deletes().unwrap();
    }
}
