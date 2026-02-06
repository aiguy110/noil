use std::io::Write;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::time::sleep;
use noil::storage::traits::Storage;

#[tokio::test]
async fn test_collector_http_status_endpoint() {
    // Create a test log file
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "2026-01-28T10:00:00Z First log").unwrap();
    writeln!(temp_file, "2026-01-28T10:00:01Z Second log").unwrap();
    temp_file.flush().unwrap();

    // Create a collector config
    let config_content = format!(
        r#"
collector:
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: block
  checkpoint:
    enabled: false
    interval_seconds: 30
  status_ui:
    enabled: true

sources:
  test_source:
    type: file
    path: {}
    timestamp:
      pattern: '^(?P<ts>\d{{4}}-\d{{2}}-\d{{2}}T\d{{2}}:\d{{2}}:\d{{2}}Z)'
      format: iso8601
    read:
      start: beginning
      follow: false

fiber_types: {{}}

pipeline:
  backpressure:
    strategy: block
    buffer_limit: 10000
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: false
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test_collector.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: "127.0.0.1:17105"
"#,
        temp_file.path().display()
    );

    let mut config_file = NamedTempFile::new().unwrap();
    write!(config_file, "{}", config_content).unwrap();
    config_file.flush().unwrap();

    // Start collector in background
    let config_path = config_file.path().to_path_buf();
    let handle = tokio::spawn(async move {
        let config = noil::config::parse::load_config(&config_path).unwrap();
        let storage = std::sync::Arc::new(
            noil::storage::duckdb::DuckDbStorage::new(&config.storage.path)
                .unwrap()
        );
        storage.init_schema().await.unwrap();
        let runner =
            noil::collector::runner::CollectorRunner::new(config, 1).unwrap();
        runner.run(storage).await
    });

    // Wait for server to start
    sleep(Duration::from_secs(2)).await;

    // Test status endpoint
    let client = reqwest::Client::new();
    let response = client
        .get("http://127.0.0.1:17105/collector/status")
        .send()
        .await;

    match response {
        Ok(resp) => {
            assert_eq!(resp.status(), 200);
            let json: serde_json::Value = resp.json().await.unwrap();
            assert!(json["collector_id"].is_string());
            assert!(json["uptime_seconds"].is_number());
            assert!(json["buffer_status"].is_object());
        }
        Err(e) => {
            eprintln!("Failed to connect to collector: {}", e);
            // Abort and cleanup
            handle.abort();
            panic!("Collector server did not start properly");
        }
    }

    // Cleanup
    handle.abort();
}

#[tokio::test]
async fn test_collector_batches_endpoint() {
    // Create a test log file
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "2026-01-28T10:00:00Z Log 1").unwrap();
    writeln!(temp_file, "2026-01-28T10:00:05Z Log 2").unwrap();
    writeln!(temp_file, "2026-01-28T10:00:15Z Log 3").unwrap();
    temp_file.flush().unwrap();

    // Create a collector config
    let config_content = format!(
        r#"
collector:
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: block
  checkpoint:
    enabled: false
    interval_seconds: 30

sources:
  test_source:
    type: file
    path: {}
    timestamp:
      pattern: '^(?P<ts>\d{{4}}-\d{{2}}-\d{{2}}T\d{{2}}:\d{{2}}:\d{{2}}Z)'
      format: iso8601
    read:
      start: beginning
      follow: false

fiber_types: {{}}

pipeline:
  backpressure:
    strategy: block
    buffer_limit: 10000
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: false
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test_collector2.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: "127.0.0.1:17107"
"#,
        temp_file.path().display()
    );

    let mut config_file = NamedTempFile::new().unwrap();
    write!(config_file, "{}", config_content).unwrap();
    config_file.flush().unwrap();

    // Start collector in background
    let config_path = config_file.path().to_path_buf();
    let handle = tokio::spawn(async move {
        let config = noil::config::parse::load_config(&config_path).unwrap();
        let storage = std::sync::Arc::new(
            noil::storage::duckdb::DuckDbStorage::new(&config.storage.path)
                .unwrap()
        );
        storage.init_schema().await.unwrap();
        let runner =
            noil::collector::runner::CollectorRunner::new(config, 1).unwrap();
        runner.run(storage).await
    });

    // Wait for server to start and process logs
    sleep(Duration::from_secs(3)).await;

    // Test batches endpoint
    let client = reqwest::Client::new();
    let response = client
        .get("http://127.0.0.1:17107/collector/batches?after=0&limit=10")
        .send()
        .await;

    match response {
        Ok(resp) => {
            assert_eq!(resp.status(), 200);
            let json: serde_json::Value = resp.json().await.unwrap();
            assert!(json["batches"].is_array());

            // We should have at least 1 batch (logs span 2 epochs)
            let batches = json["batches"].as_array().unwrap();
            if !batches.is_empty() {
                let first_batch = &batches[0];
                assert!(first_batch["batch_id"].is_string());
                assert!(first_batch["collector_id"].is_string());
                assert!(first_batch["sequence_num"].is_number());
                assert!(first_batch["logs"].is_array());
            }
        }
        Err(e) => {
            eprintln!("Failed to connect to collector: {}", e);
            handle.abort();
            panic!("Collector server did not start properly");
        }
    }

    // Cleanup
    handle.abort();
}

#[tokio::test]
async fn test_collector_acknowledge_endpoint() {
    // Create a test log file
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "2026-01-28T10:00:00Z Test log").unwrap();
    temp_file.flush().unwrap();

    // Create a collector config
    let config_content = format!(
        r#"
collector:
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: block
  checkpoint:
    enabled: false
    interval_seconds: 30

sources:
  test_source:
    type: file
    path: {}
    timestamp:
      pattern: '^(?P<ts>\d{{4}}-\d{{2}}-\d{{2}}T\d{{2}}:\d{{2}}:\d{{2}}Z)'
      format: iso8601
    read:
      start: beginning
      follow: false

fiber_types: {{}}

pipeline:
  backpressure:
    strategy: block
    buffer_limit: 10000
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: false
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test_collector3.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: "127.0.0.1:17109"
"#,
        temp_file.path().display()
    );

    let mut config_file = NamedTempFile::new().unwrap();
    write!(config_file, "{}", config_content).unwrap();
    config_file.flush().unwrap();

    // Start collector in background
    let config_path = config_file.path().to_path_buf();
    let handle = tokio::spawn(async move {
        let config = noil::config::parse::load_config(&config_path).unwrap();
        let storage = std::sync::Arc::new(
            noil::storage::duckdb::DuckDbStorage::new(&config.storage.path)
                .unwrap()
        );
        storage.init_schema().await.unwrap();
        let runner =
            noil::collector::runner::CollectorRunner::new(config, 1).unwrap();
        runner.run(storage).await
    });

    // Wait for server to start
    sleep(Duration::from_secs(2)).await;

    // Test acknowledge endpoint
    let client = reqwest::Client::new();
    let ack_request = serde_json::json!({
        "sequence_nums": [0, 1, 2]
    });

    let response = client
        .post("http://127.0.0.1:17109/collector/acknowledge")
        .json(&ack_request)
        .send()
        .await;

    match response {
        Ok(resp) => {
            assert_eq!(resp.status(), 200);
            let json: serde_json::Value = resp.json().await.unwrap();
            assert!(json["acknowledged_count"].is_number());
            assert_eq!(json["acknowledged_count"], 3);
        }
        Err(e) => {
            eprintln!("Failed to connect to collector: {}", e);
            handle.abort();
            panic!("Collector server did not start properly");
        }
    }

    // Cleanup
    handle.abort();
}
