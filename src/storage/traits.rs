use async_trait::async_trait;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn init_schema(&self) -> Result<(), StorageError>;
    // TODO: Add storage trait methods
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Storage error: {0}")]
    Generic(String),
}
