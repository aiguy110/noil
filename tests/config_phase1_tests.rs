use noil::config::types::{BufferStrategy, Config, OperationMode};
use std::time::Duration;

#[test]
fn test_collector_config_parsing() {
    let yaml = r#"
mode: collector

collector:
  listen: 127.0.0.1:7105
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: block
  checkpoint:
    enabled: true
    interval_seconds: 30
  status_ui:
    enabled: true

sources:
  test_source:
    type: file
    path: /var/log/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)'
      format: iso8601
    read:
      start: beginning
      follow: true

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
  path: /tmp/noil.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:7104
"#;

    let config: Config = serde_yaml::from_str(yaml).expect("Failed to parse collector config");

    assert_eq!(config.mode, OperationMode::Collector);
    assert!(config.collector.is_some());
    assert!(config.parent.is_none());

    let collector = config.collector.unwrap();
    assert_eq!(collector.listen, "127.0.0.1:7105");
    assert_eq!(collector.epoch_duration, Duration::from_secs(10));
    assert_eq!(collector.buffer.max_epochs, 100);
    assert_eq!(collector.buffer.strategy, BufferStrategy::Block);
    assert!(collector.checkpoint.enabled);
    assert_eq!(collector.checkpoint.interval_seconds, 30);
    assert!(collector.status_ui.enabled);
}

#[test]
fn test_parent_config_parsing() {
    let yaml = r#"
mode: parent

parent:
  collectors:
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

fiber_types:
  test_fiber:
    description: "Test fiber"
    temporal:
      max_gap: 5s
      gap_mode: session
    attributes:
      - name: test_attr
        type: string
        key: true
    sources: {}

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
  path: /tmp/noil.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:7104
"#;

    let config: Config = serde_yaml::from_str(yaml).expect("Failed to parse parent config");

    assert_eq!(config.mode, OperationMode::Parent);
    assert!(config.parent.is_some());
    assert!(config.collector.is_none());

    let parent = config.parent.unwrap();
    assert_eq!(parent.collectors.len(), 2);
    assert_eq!(parent.collectors[0].id, "collector1");
    assert_eq!(parent.collectors[0].url, "http://192.168.1.10:7105");
    assert_eq!(parent.collectors[0].retry_interval, Duration::from_secs(5));
    assert_eq!(parent.collectors[0].timeout, Duration::from_secs(30));
    assert_eq!(parent.poll_interval, Duration::from_secs(1));
}

#[test]
fn test_standalone_config_defaults() {
    let yaml = r#"
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
  path: /tmp/noil.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:7104
"#;

    let config: Config = serde_yaml::from_str(yaml).expect("Failed to parse standalone config");

    // Mode should default to Standalone
    assert_eq!(config.mode, OperationMode::Standalone);
    assert!(config.collector.is_none());
    assert!(config.parent.is_none());
}

#[test]
fn test_buffer_strategy_parsing() {
    // Test block strategy
    let yaml_block = r#"
mode: collector
collector:
  listen: 127.0.0.1:7105
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: block
sources: {}
fiber_types: {}
pipeline:
  backpressure: { strategy: block, buffer_limit: 10000 }
  errors: { on_parse_error: drop }
  checkpoint: { enabled: true, interval_seconds: 30 }
sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s
storage: { path: /tmp/noil.duckdb, batch_size: 1000, flush_interval_seconds: 5 }
web: { listen: 127.0.0.1:7104 }
"#;
    let config: Config = serde_yaml::from_str(yaml_block).unwrap();
    assert_eq!(config.collector.as_ref().unwrap().buffer.strategy, BufferStrategy::Block);

    // Test drop_oldest strategy
    let yaml_drop = r#"
mode: collector
collector:
  listen: 127.0.0.1:7105
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: drop_oldest
sources: {}
fiber_types: {}
pipeline:
  backpressure: { strategy: block, buffer_limit: 10000 }
  errors: { on_parse_error: drop }
  checkpoint: { enabled: true, interval_seconds: 30 }
sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s
storage: { path: /tmp/noil.duckdb, batch_size: 1000, flush_interval_seconds: 5 }
web: { listen: 127.0.0.1:7104 }
"#;
    let config: Config = serde_yaml::from_str(yaml_drop).unwrap();
    assert_eq!(config.collector.as_ref().unwrap().buffer.strategy, BufferStrategy::DropOldest);

    // Test wait_forever strategy
    let yaml_wait = r#"
mode: collector
collector:
  listen: 127.0.0.1:7105
  epoch_duration: 10s
  buffer:
    max_epochs: 100
    strategy: wait_forever
sources: {}
fiber_types: {}
pipeline:
  backpressure: { strategy: block, buffer_limit: 10000 }
  errors: { on_parse_error: drop }
  checkpoint: { enabled: true, interval_seconds: 30 }
sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s
storage: { path: /tmp/noil.duckdb, batch_size: 1000, flush_interval_seconds: 5 }
web: { listen: 127.0.0.1:7104 }
"#;
    let config: Config = serde_yaml::from_str(yaml_wait).unwrap();
    assert_eq!(config.collector.as_ref().unwrap().buffer.strategy, BufferStrategy::WaitForever);
}
