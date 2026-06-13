//! Safe memory-mapped file wrapper
//!
//! Provides a safe abstraction over memmap2 for reading segment files
//! with automatic bounds checking and lifecycle management.

use memmap2::Mmap;
use std::fs::{File, OpenOptions};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MmapError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Memory map error: {0}")]
    Map(String),
    #[error("Out of bounds: offset={offset}, len={len}, size={size}")]
    OutOfBounds { offset: usize, len: usize, size: usize },
}

/// Safe memory-mapped file reader
pub struct MmapReader {
    mmap: Mmap,
    file: File,
}

impl MmapReader {
    /// Open and memory-map a file for reading
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MmapError> {
        let file = OpenOptions::new().read(true).open(path)?;

        // Handle empty files
        let metadata = file.metadata()?;
        if metadata.len() == 0 {
            return Err(MmapError::Map("Cannot mmap empty file".to_string()));
        }

        let mmap = unsafe { Mmap::map(&file).map_err(|e| MmapError::Map(e.to_string()))? };

        Ok(Self { mmap, file })
    }

    /// Get the size of the mapped file
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    /// Check if the mapped file is empty
    pub fn is_empty(&self) -> bool {
        self.mmap.is_empty()
    }

    /// Read bytes at a given offset with bounds checking
    pub fn read_bytes(&self, offset: usize, len: usize) -> Result<&[u8], MmapError> {
        if offset + len > self.mmap.len() {
            return Err(MmapError::OutOfBounds {
                offset,
                len,
                size: self.mmap.len(),
            });
        }
        Ok(&self.mmap[offset..offset + len])
    }

    /// Read a u32 at a given offset (little-endian)
    pub fn read_u32(&self, offset: usize) -> Result<u32, MmapError> {
        let bytes = self.read_bytes(offset, 4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a u64 at a given offset (little-endian)
    pub fn read_u64(&self, offset: usize) -> Result<u64, MmapError> {
        let bytes = self.read_bytes(offset, 8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// Get the underlying file reference
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Advise the OS about access patterns
    pub fn advise(&self, advice: memmap2::Advice) -> Result<(), MmapError> {
        self.mmap.advise(advice).map_err(|e| MmapError::Map(e.to_string()))
    }
}

/// Builder for creating segment files with memory mapping
pub struct MmapWriter {
    file: File,
    path: std::path::PathBuf,
}

impl MmapWriter {
    /// Create a new file for writing
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self, MmapError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;

        Ok(Self { file, path })
    }

    /// Write bytes to the file
    pub fn write_all(&mut self, data: &[u8]) -> Result<(), MmapError> {
        use std::io::Write;
        self.file.write_all(data)?;
        Ok(())
    }

    /// Write a u32 in little-endian format
    pub fn write_u32(&mut self, value: u32) -> Result<(), MmapError> {
        self.write_all(&value.to_le_bytes())
    }

    /// Write a u64 in little-endian format
    pub fn write_u64(&mut self, value: u64) -> Result<(), MmapError> {
        self.write_all(&value.to_le_bytes())
    }

    /// Sync all data to disk
    pub fn sync(&self) -> Result<(), MmapError> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Convert to a reader after writing is complete
    pub fn into_reader(self) -> Result<MmapReader, MmapError> {
        self.sync()?;
        MmapReader::open(self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_mmap_reader() {
        let mut temp = NamedTempFile::new().unwrap();
        let data: Vec<u8> = (0..100).collect();
        temp.write_all(&data).unwrap();
        temp.flush().unwrap();

        let reader = MmapReader::open(temp.path()).unwrap();
        assert_eq!(reader.len(), 100);

        let bytes = reader.read_bytes(10, 5).unwrap();
        assert_eq!(bytes, &[10, 11, 12, 13, 14]);
    }

    #[test]
    fn test_mmap_bounds_checking() {
        let mut temp = NamedTempFile::new().unwrap();
        temp.write_all(&[1, 2, 3, 4, 5]).unwrap();
        temp.flush().unwrap();

        let reader = MmapReader::open(temp.path()).unwrap();
        assert!(reader.read_bytes(3, 5).is_err());
    }
}
