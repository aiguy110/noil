use noil::config::types::{CollectorEndpoint, Config};
use noil::parent::collector_client::CollectorClient;
use noil::parent::collector_stream::CollectorStream;
use std::time::Duration;

#[test]
fn test_collector_client_creation() {
    let endpoint = CollectorEndpoint {
        id: "test-collector".to_string(),
        url: "http://localhost:7105".to_string(),
        retry_interval: Duration::from_secs(5),
        timeout: Duration::from_secs(30),
    };

    let client = CollectorClient::new(&endpoint).unwrap();
    assert_eq!(client.collector_id(), "test-collector");
}

#[test]
fn test_collector_stream_creation() {
    let endpoint = CollectorEndpoint {
        id: "test-collector".to_string(),
        url: "http://localhost:7105".to_string(),
        retry_interval: Duration::from_secs(5),
        timeout: Duration::from_secs(30),
    };

    let client = CollectorClient::new(&endpoint).unwrap();
    let stream = CollectorStream::new(client);

    assert_eq!(stream.collector_id(), "test-collector");
    assert_eq!(stream.last_sequence(), 0);
    assert!(stream.watermark().is_none());
}

#[test]
fn test_watermark_generation_comparison() {
    // Test that generation takes precedence in watermark comparison
    let gen0_time1 = (0, chrono::Utc::now());
    let gen1_time0 = (1, chrono::Utc::now() - chrono::Duration::hours(1));

    // Generation 1 should be greater even though timestamp is earlier
    assert!(gen1_time0 > gen0_time1);
}

#[test]
fn test_stream_stats() {
    let endpoint = CollectorEndpoint {
        id: "test-collector".to_string(),
        url: "http://localhost:7105".to_string(),
        retry_interval: Duration::from_secs(5),
        timeout: Duration::from_secs(30),
    };

    let client = CollectorClient::new(&endpoint).unwrap();
    let stream = CollectorStream::new(client);

    let stats = stream.stats();
    assert_eq!(stats.collector_id, "test-collector");
    assert_eq!(stats.last_sequence, 0);
    assert_eq!(stats.queued_logs, 0);
    assert!(stats.watermark.is_none());
    assert!(!stats.closed);
}

#[test]
fn test_stream_reset() {
    let endpoint = CollectorEndpoint {
        id: "test-collector".to_string(),
        url: "http://localhost:7105".to_string(),
        retry_interval: Duration::from_secs(5),
        timeout: Duration::from_secs(30),
    };

    let client = CollectorClient::new(&endpoint).unwrap();
    let mut stream = CollectorStream::new(client);

    stream.reset_to_sequence(100);
    assert_eq!(stream.last_sequence(), 100);
    assert!(stream.watermark().is_none());
}

#[test]
fn test_stream_close() {
    let endpoint = CollectorEndpoint {
        id: "test-collector".to_string(),
        url: "http://localhost:7105".to_string(),
        retry_interval: Duration::from_secs(5),
        timeout: Duration::from_secs(30),
    };

    let client = CollectorClient::new(&endpoint).unwrap();
    let mut stream = CollectorStream::new(client);

    stream.close();
    let stats = stream.stats();
    assert!(stats.closed);
    assert_eq!(stats.queued_logs, 0);
}

// Note: Full end-to-end integration tests that actually start collectors and parent
// instances will be added in Phase 5. These tests verify the basic data structures
// and client/stream creation.

#[test]
fn test_remote_collectors_config_parsing() {
    let yaml = r#"
remote_collectors:
  endpoints:
    - id: collector1
      url: http://192.168.1.10:7105
      retry_interval: 5s
      timeout: 30s
    - id: collector2
      url: http://192.168.1.11:7105
      retry_interval: 5s
      timeout: 30s
  poll_interval: 1s
  backpressure:
    strategy: block
    buffer_limit: 10000

sources: {}
fiber_types: {}
pipeline:
  backpressure:
    strategy: block
    buffer_limit: 10000
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30
sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s
storage:
  path: /var/lib/noil/noil.duckdb
  batch_size: 1000
  flush_interval_seconds: 5
web:
  listen: 127.0.0.1:7104
"#;

    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert!(config.has_remote_sources());

    let remote = config.remote_collectors.unwrap();
    assert_eq!(remote.endpoints.len(), 2);
    assert_eq!(remote.endpoints[0].id, "collector1");
    assert_eq!(remote.endpoints[0].url, "http://192.168.1.10:7105");
    assert_eq!(remote.endpoints[1].id, "collector2");
    assert_eq!(remote.poll_interval, Duration::from_secs(1));
}
