use nitro_core::{Collection, Document};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use thiserror::Error;

use crate::engine::SearchEngine;

#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub struct PersistenceManager {
    data_dir: PathBuf,
}

impl PersistenceManager {
    pub fn new(data_dir: &str) -> Self {
        let path = PathBuf::from(data_dir);
        fs::create_dir_all(&path).ok();
        Self { data_dir: path }
    }

    fn collection_dir(&self, collection: &str) -> PathBuf {
        self.data_dir.join(collection)
    }

    fn documents_file(&self, collection: &str) -> PathBuf {
        self.collection_dir(collection).join("documents.jsonl")
    }

    fn collection_meta_file(&self, collection: &str) -> PathBuf {
        self.collection_dir(collection).join("meta.json")
    }

    pub fn save_collection(
        &self,
        name: &str,
        collection: &Collection,
    ) -> Result<(), PersistenceError> {
        let dir = self.collection_dir(name);
        fs::create_dir_all(&dir)?;

        let meta = serde_json::to_string_pretty(collection)?;
        fs::write(self.collection_meta_file(name), meta)?;

        Ok(())
    }

    pub fn save_document(&self, collection: &str, doc: &Document) -> Result<(), PersistenceError> {
        let dir = self.collection_dir(collection);
        fs::create_dir_all(&dir)?;

        let docs_file = self.documents_file(collection);

        // Load existing docs (dedup by id)
        let mut docs: Vec<Document> = Vec::new();
        if docs_file.exists() {
            let file = fs::File::open(&docs_file)?;
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if let Ok(existing) = serde_json::from_str::<Document>(&line) {
                    if existing.id != doc.id {
                        docs.push(existing);
                    }
                }
            }
        }

        docs.push(doc.clone());

        // Rewrite full file
        let mut file = fs::File::create(&docs_file)?;
        for d in &docs {
            let line = serde_json::to_string(d)?;
            writeln!(file, "{}", line)?;
        }

        Ok(())
    }

    pub fn delete_document(&self, collection: &str, doc_id: &str) -> Result<(), PersistenceError> {
        let docs_file = self.documents_file(collection);
        if !docs_file.exists() {
            return Ok(());
        }

        let file = fs::File::open(&docs_file)?;
        let reader = BufReader::new(file);

        let mut docs = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if let Ok(doc) = serde_json::from_str::<Document>(&line) {
                if doc.id != doc_id {
                    docs.push(doc);
                }
            }
        }

        let mut file = fs::File::create(&docs_file)?;
        for doc in docs {
            let line = serde_json::to_string(&doc)?;
            writeln!(file, "{}", line)?;
        }

        Ok(())
    }

    pub fn load_all(&self, engine: &SearchEngine) -> Result<(), PersistenceError> {
        if !self.data_dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let collection_name = entry.file_name().to_string_lossy().to_string();
                self.load_collection(engine, &collection_name)?;
            }
        }

        Ok(())
    }

    fn load_collection(
        &self,
        engine: &SearchEngine,
        collection_name: &str,
    ) -> Result<(), PersistenceError> {
        let meta_file = self.collection_meta_file(collection_name);
        if meta_file.exists() {
            let meta_str = fs::read_to_string(&meta_file)?;
            let collection: Collection = serde_json::from_str(&meta_str)?;
            engine.create_collection(collection_name, collection);
        }

        let docs_file = self.documents_file(collection_name);
        if docs_file.exists() {
            let file = fs::File::open(&docs_file)?;
            let reader = BufReader::new(file);

            for line in reader.lines() {
                let line = line?;
                if let Ok(doc) = serde_json::from_str::<Document>(&line) {
                    engine.insert_document(collection_name, doc);
                }
            }
        }

        Ok(())
    }

    pub fn delete_collection(&self, collection: &str) -> Result<(), PersistenceError> {
        let dir = self.collection_dir(collection);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    pub fn create_snapshot(&self, name: &str) -> Result<(), PersistenceError> {
        let snapshot_dir = self.data_dir.join(format!("snapshots/{}", name));
        if snapshot_dir.exists() {
            fs::remove_dir_all(&snapshot_dir)?;
        }
        fs::create_dir_all(&snapshot_dir)?;

        // Copy all collections to snapshot
        if self.data_dir.exists() {
            for entry in fs::read_dir(&self.data_dir)? {
                let entry = entry?;
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if name_str != "snapshots" && entry.file_type()?.is_dir() {
                    let dest = snapshot_dir.join(&file_name);
                    copy_dir_all(entry.path(), dest)?;
                }
            }
        }
        Ok(())
    }

    pub fn restore_snapshot(&self, name: &str) -> Result<(), PersistenceError> {
        let snapshot_dir = self.data_dir.join(format!("snapshots/{}", name));
        if !snapshot_dir.exists() {
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Snapshot '{}' not found", name),
            )));
        }

        // Clear current data (except snapshots dir)
        if self.data_dir.exists() {
            for entry in fs::read_dir(&self.data_dir)? {
                let entry = entry?;
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if name_str != "snapshots" {
                    if entry.file_type()?.is_dir() {
                        fs::remove_dir_all(entry.path())?;
                    } else {
                        fs::remove_file(entry.path())?;
                    }
                }
            }
        }

        // Copy snapshot back to data dir
        for entry in fs::read_dir(&snapshot_dir)? {
            let entry = entry?;
            let dest = self.data_dir.join(entry.file_name());
            copy_dir_all(entry.path(), dest)?;
        }

        Ok(())
    }
}

fn copy_dir_all(src: std::path::PathBuf, dst: std::path::PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}
