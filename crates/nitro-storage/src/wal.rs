//! Write-Ahead Log (WAL) for crash recovery
//!
//! Provides durable writes with fsync guarantees and recovery capabilities.
//! Each WAL entry includes a CRC32 checksum for corruption detection.

use crc32fast::Hasher;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WalError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Corrupted WAL entry: expected checksum {expected}, got {actual}")]
    Corrupted { expected: u32, actual: u32 },
    #[error("Invalid WAL entry type: {0}")]
    InvalidType(u8),
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// WAL entry types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalEntryType {
    Insert = 1,
    Update = 2,
    Delete = 3,
}

impl TryFrom<u8> for WalEntryType {
    type Error = WalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(WalEntryType::Insert),
            2 => Ok(WalEntryType::Update),
            3 => Ok(WalEntryType::Delete),
            _ => Err(WalError::InvalidType(value)),
        }
    }
}

/// A single WAL entry
#[derive(Debug, Clone)]
pub struct WalEntry {
    pub entry_type: WalEntryType,
    pub collection: String,
    pub doc_id: String,
    pub payload: Vec<u8>,
}

impl WalEntry {
    pub fn new_insert(collection: String, doc_id: String, payload: Vec<u8>) -> Self {
        Self {
            entry_type: WalEntryType::Insert,
            collection,
            doc_id,
            payload,
        }
    }

    pub fn new_delete(collection: String, doc_id: String) -> Self {
        Self {
            entry_type: WalEntryType::Delete,
            collection,
            doc_id,
            payload: Vec::new(),
        }
    }

    /// Encode entry to bytes
    /// Format: [entry_type: u8][collection_len: u32][collection: bytes]
    ///         [doc_id_len: u32][doc_id: bytes][payload_len: u32][payload: bytes]
    ///         [checksum: u32]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Entry type
        buf.push(self.entry_type as u8);

        // Collection
        let collection_bytes = self.collection.as_bytes();
        buf.extend(&(collection_bytes.len() as u32).to_le_bytes());
        buf.extend(collection_bytes);

        // Doc ID
        let doc_id_bytes = self.doc_id.as_bytes();
        buf.extend(&(doc_id_bytes.len() as u32).to_le_bytes());
        buf.extend(doc_id_bytes);

        // Payload
        buf.extend(&(self.payload.len() as u32).to_le_bytes());
        buf.extend(&self.payload);

        // Checksum
        let mut hasher = Hasher::new();
        hasher.update(&buf);
        let checksum = hasher.finalize();
        buf.extend(&checksum.to_le_bytes());

        buf
    }

    /// Decode entry from bytes
    pub fn decode(data: &[u8]) -> Result<Self, WalError> {
        if data.len() < 1 + 4 + 4 + 4 + 4 {
            return Err(WalError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "WAL entry too short",
            )));
        }

        let mut pos = 0;

        // Entry type
        let entry_type = WalEntryType::try_from(data[pos])?;
        pos += 1;

        // Collection
        let collection_len =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        let collection = String::from_utf8_lossy(&data[pos..pos + collection_len]).to_string();
        pos += collection_len;

        // Doc ID
        let doc_id_len =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        let doc_id = String::from_utf8_lossy(&data[pos..pos + doc_id_len]).to_string();
        pos += doc_id_len;

        // Payload
        let payload_len =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        let payload = data[pos..pos + payload_len].to_vec();
        pos += payload_len;

        // Checksum
        let expected_checksum =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let mut hasher = Hasher::new();
        hasher.update(&data[..pos]);
        let actual_checksum = hasher.finalize();

        if expected_checksum != actual_checksum {
            return Err(WalError::Corrupted {
                expected: expected_checksum,
                actual: actual_checksum,
            });
        }

        Ok(Self {
            entry_type,
            collection,
            doc_id,
            payload,
        })
    }
}

/// Write-Ahead Log writer
pub struct WalWriter {
    file: File,
    path: PathBuf,
}

impl WalWriter {
    /// Open or create a WAL file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, WalError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        Ok(Self { file, path })
    }

    /// Write an entry with fsync guarantee
    pub fn write(&mut self, entry: &WalEntry) -> Result<(), WalError> {
        let encoded = entry.encode();
        self.file.write_all(&encoded)?;
        self.file.sync_all()?; // Fsync for durability
        Ok(())
    }

    /// Get the path to the WAL file
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// WAL reader for recovery
pub struct WalReader {
    path: PathBuf,
}

impl WalReader {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Read all valid entries from the WAL
    /// Stops at the first corrupted entry
    pub fn read_all(&self) -> Result<Vec<WalEntry>, WalError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();

            // Read entry type
            let mut type_buf = [0u8; 1];
            match reader.read_exact(&mut type_buf) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(WalError::Io(e)),
            }

            let entry_type = WalEntryType::try_from(type_buf[0])?;

            // Read collection length
            let mut len_buf = [0u8; 4];
            reader.read_exact(&mut len_buf)?;
            let collection_len = u32::from_le_bytes(len_buf) as usize;

            // Read collection
            let mut collection_buf = vec![0u8; collection_len];
            reader.read_exact(&mut collection_buf)?;

            // Read doc ID length
            reader.read_exact(&mut len_buf)?;
            let doc_id_len = u32::from_le_bytes(len_buf) as usize;

            // Read doc ID
            let mut doc_id_buf = vec![0u8; doc_id_len];
            reader.read_exact(&mut doc_id_buf)?;

            // Read payload length
            reader.read_exact(&mut len_buf)?;
            let payload_len = u32::from_le_bytes(len_buf) as usize;

            // Read payload
            let mut payload_buf = vec![0u8; payload_len];
            reader.read_exact(&mut payload_buf)?;

            // Read checksum
            let mut checksum_buf = [0u8; 4];
            reader.read_exact(&mut checksum_buf)?;

            // Reconstruct entry
            buf.push(entry_type as u8);
            buf.extend(&(collection_len as u32).to_le_bytes());
            buf.extend(&collection_buf);
            buf.extend(&(doc_id_len as u32).to_le_bytes());
            buf.extend(&doc_id_buf);
            buf.extend(&(payload_len as u32).to_le_bytes());
            buf.extend(&payload_buf);
            buf.extend(&checksum_buf);

            match WalEntry::decode(&buf) {
                Ok(entry) => entries.push(entry),
                Err(WalError::Corrupted { .. }) => {
                    // Stop at first corruption
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_wal_entry_encode_decode() {
        let entry = WalEntry::new_insert(
            "test_collection".to_string(),
            "doc_123".to_string(),
            b"test payload".to_vec(),
        );

        let encoded = entry.encode();
        let decoded = WalEntry::decode(&encoded).unwrap();

        assert_eq!(decoded.entry_type, WalEntryType::Insert);
        assert_eq!(decoded.collection, "test_collection");
        assert_eq!(decoded.doc_id, "doc_123");
        assert_eq!(decoded.payload, b"test payload");
    }

    #[test]
    fn test_wal_checksum() {
        let entry = WalEntry::new_insert("test".to_string(), "doc1".to_string(), vec![1, 2, 3, 4]);

        let mut encoded = entry.encode();

        // Corrupt the data
        if let Some(byte) = encoded.get_mut(5) {
            *byte ^= 0xFF;
        }

        assert!(WalEntry::decode(&encoded).is_err());
    }

    #[test]
    fn test_wal_writer_reader() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let mut writer = WalWriter::open(path).unwrap();

        let entry1 =
            WalEntry::new_insert("col1".to_string(), "doc1".to_string(), b"data1".to_vec());
        let entry2 = WalEntry::new_delete("col1".to_string(), "doc2".to_string());

        writer.write(&entry1).unwrap();
        writer.write(&entry2).unwrap();

        let reader = WalReader::new(path);
        let entries = reader.read_all().unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry_type, WalEntryType::Insert);
        assert_eq!(entries[1].entry_type, WalEntryType::Delete);
    }
}
