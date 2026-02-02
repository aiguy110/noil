use crate::collector::batch::LogBatch;
use crate::config::types::CollectorEndpoint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CollectorClientError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON serialization/deserialization failed: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Collector returned error status {status}: {message}")]
    CollectorError { status: u16, message: String },

    #[error("Timeout waiting for response")]
    Timeout,

    #[error("Max retries exceeded")]
    MaxRetriesExceeded,
}

pub type Result<T> = std::result::Result<T, CollectorClientError>;

/// HTTP client for polling a collector
#[derive(Debug)]
pub struct CollectorClient {
    collector_id: String,
    base_url: String,
    client: reqwest::Client,
    retry_interval: Duration,
}

impl CollectorClient {
    pub fn new(config: &CollectorEndpoint) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()?;

        Ok(Self {
            collector_id: config.id.clone(),
            base_url: config.url.clone(),
            client,
            retry_interval: config.retry_interval,
        })
    }

    pub fn collector_id(&self) -> &str {
        &self.collector_id
    }

    /// Get collector status
    pub async fn get_status(&self) -> Result<CollectorStatus> {
        let url = format!("{}/collector/status", self.base_url);
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(CollectorClientError::CollectorError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let status = response.json().await?;
        Ok(status)
    }

    /// Get batches after the specified sequence number
    /// If after is None, returns batches from the beginning
    /// If after is Some(n), returns batches with sequence_num > n
    pub async fn get_batches(&self, after: Option<u64>, limit: usize) -> Result<BatchesResponse> {
        let url = match after {
            Some(seq) => format!(
                "{}/collector/batches?after={}&limit={}",
                self.base_url, seq, limit
            ),
            None => format!(
                "{}/collector/batches?limit={}",
                self.base_url, limit
            ),
        };
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(CollectorClientError::CollectorError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let batches = response.json().await?;
        Ok(batches)
    }

    /// Acknowledge processing of batches
    pub async fn acknowledge(&self, sequence_nums: Vec<u64>) -> Result<AcknowledgeResponse> {
        let url = format!("{}/collector/acknowledge", self.base_url);
        let request_body = AcknowledgeRequest { sequence_nums };

        let response = self.client.post(&url).json(&request_body).send().await?;

        if !response.status().is_success() {
            return Err(CollectorClientError::CollectorError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let ack_response = response.json().await?;
        Ok(ack_response)
    }

    /// Rewind collector to a previous sequence number
    pub async fn rewind(&self, target_sequence: Option<u64>) -> Result<RewindResponse> {
        let url = format!("{}/collector/rewind", self.base_url);
        let request_body = RewindRequest {
            target_sequence,
            preserve_buffer: false,
        };

        let response = self.client.post(&url).json(&request_body).send().await?;

        if !response.status().is_success() {
            return Err(CollectorClientError::CollectorError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let rewind_response = response.json().await?;
        Ok(rewind_response)
    }

    /// Get current checkpoint state
    pub async fn get_checkpoint(&self) -> Result<CheckpointResponse> {
        let url = format!("{}/collector/checkpoint", self.base_url);
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(CollectorClientError::CollectorError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let checkpoint = response.json().await?;
        Ok(checkpoint)
    }

    /// Retry a request with exponential backoff
    pub async fn with_retry<F, Fut, T>(&self, mut operation: F, max_retries: usize) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut attempts = 0;
        let mut backoff = self.retry_interval;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    attempts += 1;
                    if attempts >= max_retries {
                        tracing::error!(
                            collector_id = %self.collector_id,
                            attempts = attempts,
                            error = %e,
                            "Max retries exceeded"
                        );
                        return Err(CollectorClientError::MaxRetriesExceeded);
                    }

                    tracing::warn!(
                        collector_id = %self.collector_id,
                        attempt = attempts,
                        backoff_ms = backoff.as_millis(),
                        error = %e,
                        "Request failed, retrying"
                    );

                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                }
            }
        }
    }
}

// ===== Response Types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorStatus {
    pub collector_id: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub buffer_status: BufferStatus,
    pub watermark: Option<DateTime<Utc>>,
    pub sources: Vec<SourceStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferStatus {
    pub current_epochs: usize,
    pub max_epochs: usize,
    pub oldest_sequence: u64,
    pub newest_sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStatus {
    pub id: String,
    pub watermark: Option<DateTime<Utc>>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchesResponse {
    pub batches: Vec<LogBatch>,
    pub has_more: bool,
    pub next_sequence: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcknowledgeRequest {
    pub sequence_nums: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcknowledgeResponse {
    pub acknowledged_count: usize,
    pub freed_buffer_space: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindRequest {
    pub target_sequence: Option<u64>,
    pub preserve_buffer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindResponse {
    pub old_sequence: u64,
    pub new_sequence: u64,
    pub buffer_cleared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointResponse {
    pub checkpoint: serde_json::Value, // Generic checkpoint data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_constructs_correct_urls() {
        let config = CollectorEndpoint {
            id: "test-collector".to_string(),
            url: "http://localhost:7105".to_string(),
            retry_interval: Duration::from_secs(5),
            timeout: Duration::from_secs(30),
        };

        let client = CollectorClient::new(&config).unwrap();
        assert_eq!(client.base_url, "http://localhost:7105");
        assert_eq!(client.collector_id, "test-collector");
    }
}
