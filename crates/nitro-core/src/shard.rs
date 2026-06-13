//! Shard management for distributed search
//!
//! Implements horizontal partitioning (sharding) to distribute documents
//! across multiple nodes for large-scale deployments.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Shard identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShardId(pub u32);

/// Shard configuration
#[derive(Debug, Clone)]
pub struct ShardConfig {
    /// Total number of shards
    pub num_shards: u32,
    /// Replication factor (number of copies)
    pub replication_factor: u32,
    /// Shard assignment strategy
    pub strategy: ShardStrategy,
}

impl Default for ShardConfig {
    fn default() -> Self {
        Self {
            num_shards: 1,
            replication_factor: 1,
            strategy: ShardStrategy::Hash,
        }
    }
}

/// Shard assignment strategy
#[derive(Debug, Clone, Copy)]
pub enum ShardStrategy {
    /// Hash-based routing (consistent hashing)
    Hash,
    /// Range-based routing (for ordered queries)
    Range,
    /// Custom routing key
    Custom,
}

/// Shard router - determines which shard a document belongs to
pub struct ShardRouter {
    config: ShardConfig,
}

impl ShardRouter {
    pub fn new(config: ShardConfig) -> Self {
        Self { config }
    }

    /// Route document to shard based on document ID
    pub fn route_by_id(&self, doc_id: &str) -> ShardId {
        match self.config.strategy {
            ShardStrategy::Hash => self.hash_route(doc_id),
            ShardStrategy::Range => self.range_route(doc_id),
            ShardStrategy::Custom => self.hash_route(doc_id), // Fallback to hash
        }
    }

    /// Hash-based routing (consistent hashing)
    fn hash_route(&self, key: &str) -> ShardId {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = hasher.finish();
        let shard_num = (hash % self.config.num_shards as u64) as u32;
        ShardId(shard_num)
    }

    /// Range-based routing (for ordered data)
    fn range_route(&self, key: &str) -> ShardId {
        // Simple implementation: parse as number if possible
        if let Ok(num) = key.parse::<u64>() {
            let range_size = u64::MAX / self.config.num_shards as u64;
            let shard_num = (num / range_size) as u32;
            ShardId(shard_num.min(self.config.num_shards - 1))
        } else {
            // Fallback to hash routing
            self.hash_route(key)
        }
    }

    /// Get all shards
    pub fn all_shards(&self) -> Vec<ShardId> {
        (0..self.config.num_shards).map(ShardId).collect()
    }

    /// Get replica shards for a given primary shard
    pub fn get_replicas(&self, primary: ShardId) -> Vec<ShardId> {
        if self.config.replication_factor <= 1 {
            return vec![primary];
        }

        let mut replicas = vec![primary];
        for i in 1..self.config.replication_factor {
            let replica_num = (primary.0 + i) % self.config.num_shards;
            replicas.push(ShardId(replica_num));
        }
        replicas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_routing() {
        let config = ShardConfig {
            num_shards: 3,
            replication_factor: 1,
            strategy: ShardStrategy::Hash,
        };
        let router = ShardRouter::new(config);

        // Test deterministic routing
        let shard1 = router.route_by_id("doc1");
        let shard2 = router.route_by_id("doc1");
        assert_eq!(shard1, shard2);

        // Test distribution
        let mut shards = vec![0u32; 3];
        for i in 0..1000 {
            let shard = router.route_by_id(&format!("doc{}", i));
            shards[shard.0 as usize] += 1;
        }

        // Each shard should have roughly 333 documents
        for count in shards {
            assert!(count > 200 && count < 500);
        }
    }

    #[test]
    fn test_replica_routing() {
        let config = ShardConfig {
            num_shards: 5,
            replication_factor: 3,
            strategy: ShardStrategy::Hash,
        };
        let router = ShardRouter::new(config);

        let primary = ShardId(2);
        let replicas = router.get_replicas(primary);

        assert_eq!(replicas.len(), 3);
        assert_eq!(replicas[0], primary);
        assert_eq!(replicas[1], ShardId(3));
        assert_eq!(replicas[2], ShardId(4));
    }
}
