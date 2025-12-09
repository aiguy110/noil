# 01: Project Scaffolding

Set up the Rust project structure, dependencies, and CLI skeleton.

## Cargo.toml Dependencies

```toml
[package]
name = "noil"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "serde"] }
regex = "1"
duckdb = { version = "1.0" }
axum = "0.7"
tokio-util = "0.7"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
async-trait = "0.1"
```

## Module Structure

Create the directory structure per CLAUDE.md:

```
src/
├── main.rs
├── lib.rs
├── cli/
│   ├── mod.rs
│   ├── run.rs
│   └── config.rs
├── config/
│   ├── mod.rs
│   ├── types.rs
│   ├── parse.rs
│   └── generate.rs
├── source/
│   ├── mod.rs
│   ├── reader.rs
│   └── timestamp.rs
├── sequencer/
│   ├── mod.rs
│   ├── local.rs
│   └── merge.rs
├── fiber/
│   ├── mod.rs
│   ├── processor.rs
│   ├── rule.rs
│   └── session.rs
├── storage/
│   ├── mod.rs
│   ├── traits.rs
│   ├── duckdb.rs
│   └── checkpoint.rs
├── pipeline/
│   ├── mod.rs
│   ├── channel.rs
│   └── backpressure.rs
└── web/
    ├── mod.rs
    ├── server.rs
    └── api.rs
```

Each module file should have placeholder content (empty structs/functions with `todo!()` or `unimplemented!()`).

## CLI Structure

Use clap derive macros:

```rust
#[derive(Parser)]
#[command(name = "noil")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Run,
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    Init {
        #[arg(long)]
        stdout: bool,
    },
}
```

Default behavior (no subcommand) should be equivalent to `run`.

## main.rs

- Parse CLI args
- Initialize tracing subscriber
- Dispatch to appropriate handler in `cli/` module
- Config resolution: check `--config` arg, then `~/.config/noil/config.yml`, then `/etc/noil/config.yml`

## Acceptance Criteria

- `cargo build` succeeds
- `cargo run -- --help` shows help
- `cargo run -- config init --stdout` prints a placeholder message
- `cargo run` prints "config not found" or similar (config parsing not yet implemented)
