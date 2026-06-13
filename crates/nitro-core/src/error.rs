use thiserror::Error;

#[derive(Error, Debug)]
pub enum NitroError {
    #[error("Collection not found: {0}")]
    CollectionNotFound(String),

    #[error("Document not found: {0}")]
    DocumentNotFound(String),

    #[error("Invalid schema: {0}")]
    InvalidSchema(String),

    #[error("Invalid query: {0}")]
    InvalidQuery(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Index error: {0}")]
    IndexError(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl From<std::io::Error> for NitroError {
    fn from(e: std::io::Error) -> Self {
        NitroError::IoError(e.to_string())
    }
}

impl From<serde_json::Error> for NitroError {
    fn from(e: serde_json::Error) -> Self {
        NitroError::SerializationError(e.to_string())
    }
}

impl From<bincode::Error> for NitroError {
    fn from(e: bincode::Error) -> Self {
        NitroError::SerializationError(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, NitroError>;
