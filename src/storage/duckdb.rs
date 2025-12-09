use super::traits::{Storage, StorageError};
use async_trait::async_trait;

pub struct DuckDbStorage {
    // TODO: Define DuckDB storage implementation
}

impl DuckDbStorage {
    pub fn new() -> Self {
        todo!("implement DuckDbStorage")
    }
}

#[async_trait]
impl Storage for DuckDbStorage {
    async fn init_schema(&self) -> Result<(), StorageError> {
        todo!("implement init_schema")
    }
}
