use noil::config::{generate::generate_starter_config, load_config};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_generated_config_is_valid() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_content = generate_starter_config();
    fs::write(&config_path, config_content).unwrap();

    let config = load_config(&config_path).expect("Generated config should be valid");

    assert_eq!(config.sources.len(), 5);
    // 2 explicit fiber types + 5 auto-generated source fibers (one per source)
    assert_eq!(config.fiber_types.len(), 7);
    assert!(config.sources.contains_key("nginx_access"));
    assert!(config.sources.contains_key("program1"));
    assert!(config.sources.contains_key("program2"));
    assert!(config.sources.contains_key("simple_service"));
    assert!(config.fiber_types.contains_key("request_trace"));
    assert!(config.fiber_types.contains_key("simple_log"));
    // Check for auto-generated source fibers
    assert!(config.fiber_types.contains_key("nginx_access_all"));
    assert!(config.fiber_types.contains_key("application_log_all"));
    assert!(config.fiber_types.contains_key("program1_all"));
    assert!(config.fiber_types.contains_key("program2_all"));
    assert!(config.fiber_types.contains_key("simple_service_all"));
}

#[test]
fn test_missing_source_reference() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  existing_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
    sources:
      nonexistent_source:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let result = load_config(&config_path);
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("references non-existent source"));
    assert!(err_msg.contains("nonexistent_source"));
}

#[test]
fn test_invalid_timestamp_pattern() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<wrong_name>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
    sources:
      test_source:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let result = load_config(&config_path);
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("timestamp pattern must contain named capture group 'ts'"));
}

#[test]
fn test_invalid_regex_in_pattern() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
    sources:
      test_source:
        patterns:
          - regex: 'invalid(regex'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let result = load_config(&config_path);
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("invalid regex"));
}

#[test]
fn test_duplicate_attribute_names() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
      - name: foo
        type: int
    sources:
      test_source:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let result = load_config(&config_path);
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("duplicate attribute name"));
    assert!(err_msg.contains("foo"));
}

#[test]
fn test_release_matching_peer_keys_not_in_pattern() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
        key: true
      - name: bar
        type: string
        key: true
    sources:
      test_source:
        patterns:
          - regex: '(?P<foo>\w+)'
            release_matching_peer_keys: [bar]

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let result = load_config(&config_path);
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("release_matching_peer_keys contains 'bar' which is not a capture group")
    );
}

#[test]
fn test_release_self_keys_not_a_key() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
        key: false
    sources:
      test_source:
        patterns:
          - regex: 'test'
            release_self_keys: [foo]

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let result = load_config(&config_path);
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("release_self_keys contains 'foo' which is not marked as a key"));
}

#[test]
fn test_derived_attribute_references_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
      - name: bar
        type: string
        derived: "${nonexistent}"
    sources:
      test_source:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let result = load_config(&config_path);
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("references non-existent attribute"));
    assert!(err_msg.contains("nonexistent"));
}

#[test]
fn test_duration_parsing() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber_seconds:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
    sources:
      test_source:
        patterns:
          - regex: 'test'

  test_fiber_minutes:
    temporal:
      max_gap: 10m
    attributes:
      - name: bar
        type: string
    sources:
      test_source:
        patterns:
          - regex: 'test'

  test_fiber_infinite:
    temporal:
      max_gap: infinite
    attributes:
      - name: baz
        type: string
    sources:
      test_source:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let config = load_config(&config_path).expect("Config should be valid");

    let fiber_seconds = &config.fiber_types["test_fiber_seconds"];
    assert_eq!(
        fiber_seconds.temporal.max_gap,
        Some(std::time::Duration::from_secs(5))
);

    let fiber_minutes = &config.fiber_types["test_fiber_minutes"];
    assert_eq!(
        fiber_minutes.temporal.max_gap,
        Some(std::time::Duration::from_secs(600))
);

    let fiber_infinite = &config.fiber_types["test_fiber_infinite"];
    assert_eq!(fiber_infinite.temporal.max_gap, None);
}

#[test]
fn test_valid_config_with_all_features() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: /tmp/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4}-\d{2}-\d{2})'
      format: '%Y-%m-%d'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    description: "Test fiber type"
    temporal:
      max_gap: 5s
      gap_mode: session
    attributes:
      - name: ip
        type: ip
        key: true
      - name: port
        type: int
      - name: connection
        type: string
        key: true
        derived: "${ip}:${port}"
    sources:
      test_source:
        patterns:
          - regex: 'START (?P<ip>\d+\.\d+\.\d+\.\d+):(?P<port>\d+)'
            release_matching_peer_keys: [ip]
          - regex: 'END'
            close: true
            release_self_keys: [connection]

pipeline:
  backpressure:
    strategy: buffer_in_memory
    buffer_limit: 5000
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let config = load_config(&config_path).expect("Config should be valid");

    assert_eq!(config.sources.len(), 1);
    // 1 explicit fiber type + 1 auto-generated source fiber
    assert_eq!(config.fiber_types.len(), 2);

    let fiber = &config.fiber_types["test_fiber"];
    assert_eq!(fiber.description, Some("Test fiber type".to_string()));
    assert_eq!(fiber.attributes.len(), 3);

    // Check derived attribute
    let conn_attr = fiber
        .attributes
        .iter()
        .find(|a| a.name == "connection")
        .unwrap();
    assert!(conn_attr.key);
    assert_eq!(conn_attr.derived, Some("${ip}:${port}".to_string()));
}

#[test]
fn test_tilde_expansion_in_paths() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  test_source:
    type: file
    path: ~/test.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  test_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
    sources:
      test_source:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: ~/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let config = load_config(&config_path).expect("Config should be valid");

    // Verify that paths were expanded
    if let Some(home) = dirs::home_dir() {
        let expected_source_path = home.join("test.log");
        let expected_storage_path = home.join("test.duckdb");

        assert_eq!(
            config.sources["test_source"].path,
            expected_source_path,
            "Source path should have tilde expanded"
        );
        assert_eq!(
            config.storage.path, expected_storage_path,
            "Storage path should have tilde expanded"
        );

        // Also verify they don't still contain tilde
        assert!(!config.sources["test_source"]
            .path
            .to_string_lossy()
            .starts_with('~'));
        assert!(!config.storage.path.to_string_lossy().starts_with('~'));
    }
}

#[test]
fn test_auto_source_fibers_enabled() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  source1:
    type: file
    path: /tmp/test1.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

  source2:
    type: file
    path: /tmp/test2.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  custom_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
    sources:
      source1:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let config = load_config(&config_path).expect("Config should be valid");

    // Verify auto_source_fibers defaults to true
    assert!(config.auto_source_fibers);

    // 1 explicit fiber type + 2 auto-generated source fibers
    assert_eq!(config.fiber_types.len(), 3);
    assert!(config.fiber_types.contains_key("custom_fiber"));
    assert!(config.fiber_types.contains_key("source1_all"));
    assert!(config.fiber_types.contains_key("source2_all"));

    // Verify auto-generated fiber has correct properties
    let source1_fiber = &config.fiber_types["source1_all"];
    assert_eq!(
        source1_fiber.description,
        Some("Auto-generated fiber containing all logs from source1".to_string())
    );
    assert_eq!(source1_fiber.temporal.max_gap, None); // infinite
    assert_eq!(source1_fiber.attributes.len(), 1);
    assert_eq!(source1_fiber.attributes[0].name, "source_marker");
    assert!(source1_fiber.attributes[0].key);
    assert_eq!(
        source1_fiber.attributes[0].derived,
        Some("source1".to_string())
    );
}

#[test]
fn test_auto_source_fibers_disabled() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  source1:
    type: file
    path: /tmp/test1.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

auto_source_fibers: false

fiber_types:
  custom_fiber:
    temporal:
      max_gap: 5s
    attributes:
      - name: foo
        type: string
    sources:
      source1:
        patterns:
          - regex: 'test'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let config = load_config(&config_path).expect("Config should be valid");

    // Verify auto_source_fibers is disabled
    assert!(!config.auto_source_fibers);

    // Only the explicitly defined fiber type should exist
    assert_eq!(config.fiber_types.len(), 1);
    assert!(config.fiber_types.contains_key("custom_fiber"));
    assert!(!config.fiber_types.contains_key("source1_all"));
}

#[test]
fn test_auto_source_fibers_can_be_overridden() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.yml");

    let config_yaml = r#"
sources:
  source1:
    type: file
    path: /tmp/test1.log
    timestamp:
      pattern: '^(?P<ts>\d{4})'
      format: '%Y'
    read:
      start: beginning
      follow: true

fiber_types:
  source1_all:
    description: "Custom override of auto-generated fiber"
    temporal:
      max_gap: 10s
    attributes:
      - name: custom_attr
        type: string
        key: true
    sources:
      source1:
        patterns:
          - regex: 'custom_pattern'

pipeline:
  backpressure:
    strategy: block
  errors:
    on_parse_error: drop
  checkpoint:
    enabled: true
    interval_seconds: 30

sequencer:
  batch_epoch_duration: 10s
  watermark_safety_margin: 1s

storage:
  path: /tmp/test.duckdb
  batch_size: 1000
  flush_interval_seconds: 5

web:
  listen: 127.0.0.1:8080
"#;

    fs::write(&config_path, config_yaml).unwrap();

    let config = load_config(&config_path).expect("Config should be valid");

    // Should have only the manually defined fiber type (not auto-generated)
    assert_eq!(config.fiber_types.len(), 1);
    assert!(config.fiber_types.contains_key("source1_all"));

    // Verify it uses the custom definition, not auto-generated
    let fiber = &config.fiber_types["source1_all"];
    assert_eq!(
        fiber.description,
        Some("Custom override of auto-generated fiber".to_string())
    );
    assert_eq!(fiber.attributes[0].name, "custom_attr");
}
