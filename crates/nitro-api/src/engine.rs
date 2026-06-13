use nitro_core::{Collection, Document, SearchResults, ShardConfig, ShardRouter};
use nitro_index::Tokenizer;
use nitro_query::QueryParser;
use nitro_ranking::BM25Scorer;
use nitro_storage::{MergePolicy, SegmentManager};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{debug, info};

use crate::search_executor::SearchExecutor;

pub struct SearchEngine {
    collections: Arc<RwLock<HashMap<String, Collection>>>,
    segment_managers: Arc<RwLock<HashMap<String, Arc<SegmentManager>>>>,
    tokenizers: Arc<RwLock<HashMap<String, Tokenizer>>>,
    field_weights: Arc<RwLock<HashMap<String, HashMap<String, f64>>>>,
    shard_routers: Arc<RwLock<HashMap<String, Arc<ShardRouter>>>>,
    executor: SearchExecutor,
    data_dir: PathBuf,
}

impl SearchEngine {
    pub fn new() -> Self {
        let data_dir = std::env::var("NITRO_DATA_DIR").unwrap_or_else(|_| "./data".to_string());

        Self {
            collections: Arc::new(RwLock::new(HashMap::new())),
            segment_managers: Arc::new(RwLock::new(HashMap::new())),
            tokenizers: Arc::new(RwLock::new(HashMap::new())),
            field_weights: Arc::new(RwLock::new(HashMap::new())),
            shard_routers: Arc::new(RwLock::new(HashMap::new())),
            executor: SearchExecutor::new(),
            data_dir: PathBuf::from(data_dir),
        }
    }

    pub fn create_collection(&self, name: &str, collection: Collection) {
        let mut collections = self.collections.write().unwrap();
        let mut segment_managers = self.segment_managers.write().unwrap();
        let mut tokenizers = self.tokenizers.write().unwrap();
        let mut shard_routers = self.shard_routers.write().unwrap();

        collections.insert(name.to_string(), collection);

        // Create segment manager for this collection
        let col_dir = self.data_dir.join(name);
        if let Ok(manager) = SegmentManager::new(col_dir, MergePolicy::default()) {
            segment_managers.insert(name.to_string(), Arc::new(manager));
        }

        // Create shard router for this collection
        let shard_config = ShardConfig::default(); // Can be customized per collection
        shard_routers.insert(name.to_string(), Arc::new(ShardRouter::new(shard_config)));

        tokenizers.insert(name.to_string(), Tokenizer::english());
        info!("Created collection: {}", name);
    }

    pub fn set_field_weights(&self, collection: &str, weights: HashMap<String, f64>) {
        let mut field_weights = self.field_weights.write().unwrap();
        field_weights.insert(collection.to_string(), weights);
    }

    pub fn insert_document(&self, collection: &str, doc: Document) {
        let segment_managers = self.segment_managers.read().unwrap();
        let tokenizers = self.tokenizers.read().unwrap();

        if let Some(manager) = segment_managers.get(collection) {
            if let Some(tokenizer) = tokenizers.get(collection) {
                // Tokenize document
                let mut tokens = Vec::new();
                for (_, value) in &doc.fields {
                    if let Some(text) = value.as_text() {
                        let field_tokens = tokenizer.tokenize(text);
                        tokens.extend(field_tokens);
                    }
                }

                // Add to buffer (will be flushed to segment)
                let mut buffer = manager.buffer().write().unwrap();
                buffer.insert(doc.id.clone(), (doc.clone(), tokens));
                debug!("Buffered document {} into collection {}", doc.id, collection);
            }
        }
    }

    pub fn delete_document(&self, collection: &str, doc_id: &str) {
        let segment_managers = self.segment_managers.read().unwrap();

        if let Some(manager) = segment_managers.get(collection) {
            // Remove from buffer if exists (soft delete)
            let mut buffer = manager.buffer().write().unwrap();
            buffer.remove(doc_id);
            debug!("Deleted document {} from collection {}", doc_id, collection);

            // TODO: Implement segment-level soft delete with interior mutability
            // For now, documents in segments remain until segment merge
        }
    }

    pub fn search(&self, collection: &str, query_str: &str, limit: usize) -> SearchResults {
        let segment_managers = self.segment_managers.read().unwrap();
        let tokenizers = self.tokenizers.read().unwrap();
        let field_weights = self.field_weights.read().unwrap();

        let manager = match segment_managers.get(collection) {
            Some(m) => m,
            None => return SearchResults { total: 0, hits: Vec::new(), time_ms: 0 },
        };

        let tokenizer = match tokenizers.get(collection) {
            Some(t) => t,
            None => return SearchResults { total: 0, hits: Vec::new(), time_ms: 0 },
        };

        let weights = field_weights.get(collection).cloned().unwrap_or_default();

        // Parse query
        let query = QueryParser::parse(query_str);

        // Build BM25 scorer
        let mut scorer = BM25Scorer::default();
        for (field, weight) in &weights {
            scorer.set_field_weight(field, *weight);
        }

        // Get all active segments
        let segments = manager.segments();

        // Execute parallel search
        self.executor.execute_parallel(&query, &segments, tokenizer, &scorer, limit)
    }

    pub fn get_collection(&self, name: &str) -> Option<Collection> {
        let collections = self.collections.read().unwrap();
        collections.get(name).cloned()
    }

    pub fn list_collections(&self) -> Vec<String> {
        let collections = self.collections.read().unwrap();
        collections.keys().cloned().collect()
    }

    pub fn get_documents(&self, collection: &str) -> HashMap<String, Document> {
        let segment_managers = self.segment_managers.read().unwrap();

        if let Some(manager) = segment_managers.get(collection) {
            let mut result = HashMap::new();

            // Get from buffer
            let buffer = manager.buffer().read().unwrap();
            for (id, (doc, _)) in buffer.iter() {
                result.insert(id.clone(), doc.clone());
            }

            // Get from segments
            let segments = manager.segments_lock().read().unwrap();
            for segment in segments.iter() {
                for doc_id in segment.all_doc_ids() {
                    if let Ok(Some(_doc_data)) = segment.get_document(doc_id) {
                        // In a real implementation, we'd deserialize the document
                        // For now, we'll create a placeholder
                        result.insert(doc_id.to_string(), Document::new(&doc_id.to_string()));
                    }
                }
            }

            result
        } else {
            HashMap::new()
        }
    }

    pub fn get_document(&self, collection: &str, doc_id: &str) -> Option<Document> {
        let segment_managers = self.segment_managers.read().unwrap();

        if let Some(manager) = segment_managers.get(collection) {
            // Check buffer first
            let buffer = manager.buffer().read().unwrap();
            if let Some((doc, _)) = buffer.get(doc_id) {
                return Some(doc.clone());
            }

            // Check segments
            let segments = manager.segments_lock().read().unwrap();
            for segment in segments.iter() {
                if let Ok(doc_id_num) = doc_id.parse::<u32>() {
                    if let Ok(Some(_doc_data)) = segment.get_document(doc_id_num) {
                        return Some(Document::new(doc_id));
                    }
                }
            }
        }

        None
    }

    pub fn delete_collection(&self, collection: &str) {
        let mut collections = self.collections.write().unwrap();
        let mut segment_managers = self.segment_managers.write().unwrap();
        let mut tokenizers = self.tokenizers.write().unwrap();
        let mut field_weights = self.field_weights.write().unwrap();

        collections.remove(collection);
        segment_managers.remove(collection);
        tokenizers.remove(collection);
        field_weights.remove(collection);
        info!("Deleted collection {}", collection);
    }
}
