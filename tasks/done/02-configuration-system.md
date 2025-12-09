# 02: Configuration System

Implement config types, YAML parsing, validation, and the `config init` command.

## Config Types (`config/types.rs`)

Define serde-deserializable structs matching the schema in CLAUDE.md. Key types:

```rust
pub struct Config {
    pub sources: HashMap<String, SourceConfig>,
    pub fiber_types: HashMap<String, FiberTypeConfig>,
    pub pipeline: PipelineConfig,
    pub sequencer: SequencerConfig,
    pub storage: StorageConfig,
    pub web: WebConfig,
}

pub struct SourceConfig {
    pub source_type: SourceType,  // #[serde(rename = "type")]
    pub path: PathBuf,
    pub timestamp: TimestampConfig,
    pub read: ReadConfig,
}

pub struct TimestampConfig {
    pub pattern: String,  // Regex with named group 'ts'
    pub format: String,   // strptime, "iso8601", "epoch", "epoch_ms"
}

pub struct FiberTypeConfig {
    pub description: Option<String>,
    pub temporal: TemporalConfig,
    pub attributes: Vec<AttributeConfig>,
    pub sources: HashMap<String, FiberSourceConfig>,
}

pub struct AttributeConfig {
    pub name: String,
    pub attr_type: AttributeType,  // #[serde(rename = "type")]
    #[serde(default)]
    pub key: bool,
    pub derived: Option<String>,
}

pub struct PatternConfig {
    pub regex: String,
    #[serde(default)]
    pub release_matching_peer_keys: Vec<String>,
    #[serde(default)]
    pub release_self_keys: Vec<String>,
    #[serde(default)]
    pub close: bool,
}
```

Handle `max_gap: infinite` by using `Option<Duration>` where `None` means infinite.

## Parsing (`config/parse.rs`)

- `pub fn load_config(path: &Path) -> Result<Config, ConfigError>`
- Use `serde_yaml::from_reader`
- Wrap errors with context (file path, line number if possible)

## Validation (`config/parse.rs`)

After parsing, validate:

1. All sources referenced in fiber_types exist in sources
2. Timestamp patterns compile as valid regex with `ts` capture group
3. All pattern regexes compile
4. `release_matching_peer_keys` items are capture groups in the pattern AND are keys
5. `release_self_keys` items are defined attributes with `key: true`
6. Derived attribute `${name}` references exist as attributes
7. No circular dependencies in derived attributes (topological sort)
8. Attribute names unique within fiber type

Return structured errors listing all validation failures, not just the first.

## Config Generation (`config/generate.rs`)

Generate the example config from CLAUDE.md as a string. This is the `config init` output.

```rust
pub fn generate_starter_config() -> String
```

The generated config should be heavily commented, explaining options.

## Config Init Command (`cli/config.rs`)

- If `--stdout`: print to stdout
- Otherwise: write to `~/.config/noil/config.yml` (create parent dirs), or fall back to `/etc/noil/config.yml` if home not writable
- Don't overwrite existing file without warning

## Config Resolution

Create a helper:

```rust
pub fn resolve_config_path(explicit: Option<&Path>) -> Option<PathBuf>
```

Returns the first existing path from: explicit arg, `~/.config/noil/config.yml`, `/etc/noil/config.yml`.

## Acceptance Criteria

- `cargo run -- config init --stdout` outputs valid YAML matching CLAUDE.md example
- Parsing the generated config succeeds
- Invalid configs produce clear error messages listing all issues
- Circular derived attribute dependencies are detected
