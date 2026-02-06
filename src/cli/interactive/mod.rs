pub mod file_browser;
pub mod regex_input;
pub mod yaml_builder;

use crate::source::timestamp::TimestampExtractor;
use dialoguer::{Confirm, Input, Select};
use std::io::{self, BufRead, IsTerminal};
use std::path::PathBuf;
use yaml_builder::{CollectorEndpoint, InteractiveConfig, SourceEntry};

pub struct InteractiveResult {
    pub yaml: String,
    /// None means stdout, Some(path) means write to that file.
    pub output_path: Option<PathBuf>,
}

pub fn run_interactive(
    mode: Option<&str>,
    stdout: bool,
) -> Result<InteractiveResult, Box<dyn std::error::Error>> {
    // TTY check
    if !io::stdin().is_terminal() {
        return Err("--interactive requires a terminal (stdin is not a TTY)".into());
    }

    // Mode selection
    let mode = if let Some(m) = mode {
        m.to_string()
    } else {
        let modes = &["standalone", "collector", "parent"];
        let selection = Select::new()
            .with_prompt("Select operation mode")
            .items(modes)
            .default(0)
            .interact()?;
        modes[selection].to_string()
    };

    let mut sources: Vec<SourceEntry> = Vec::new();
    let mut collector_listen: Option<String> = None;
    let mut collector_endpoints: Vec<CollectorEndpoint> = Vec::new();

    // Source configuration (standalone + collector)
    if mode != "parent" {
        loop {
            println!();
            println!("--- Add a log source ---");

            // File browser
            let file_path = match file_browser::browse_for_file()? {
                Some(p) => p,
                None => {
                    if sources.is_empty() {
                        println!("No file selected. At least one source is required.");
                        continue;
                    } else {
                        break;
                    }
                }
            };

            println!("Selected: {}", file_path.display());

            // Read first non-empty lines for preview
            let sample_lines = read_sample_lines(&file_path, 5);
            if sample_lines.is_empty() {
                println!("Warning: Could not read any lines from {}", file_path.display());
                println!("Continuing without sample preview.");
            } else {
                println!();
                println!("First {} lines:", sample_lines.len());
                for line in &sample_lines {
                    let display = if line.len() > 120 {
                        format!("{}...", &line[..117])
                    } else {
                        line.clone()
                    };
                    println!("  {}", display);
                }
            }

            // Regex pattern + timestamp format input
            let (timestamp_pattern, timestamp_format) = if !sample_lines.is_empty() {
                println!();
                println!("Now enter a regex pattern to extract the timestamp.");
                println!("The pattern must contain a (?P<ts>...) named group.");
                println!();

                match regex_input::input_regex_and_format(&sample_lines)? {
                    Some(result) => (result.pattern, result.format),
                    None => {
                        println!("Cancelled. Skipping this source.");
                        continue;
                    }
                }
            } else {
                let pattern: String = Input::new()
                    .with_prompt("Timestamp regex pattern (must contain (?P<ts>...) group)")
                    .interact_text()?;

                let format_options =
                    &["iso8601", "epoch", "epoch_ms", "Custom strptime format"];
                let format_idx = Select::new()
                    .with_prompt("Timestamp format")
                    .items(format_options)
                    .default(0)
                    .interact()?;

                let format = if format_idx == 3 {
                    let fmt: String = Input::new()
                        .with_prompt(
                            "Enter strptime format string (e.g., '%Y-%m-%d %H:%M:%S')",
                        )
                        .interact_text()?;
                    fmt
                } else {
                    format_options[format_idx].to_string()
                };

                (pattern, format)
            };

            println!("Pattern: {}", timestamp_pattern);
            println!("Format: {}", timestamp_format);

            // Validate by trying to extract from first line
            if !sample_lines.is_empty() {
                match TimestampExtractor::new(&timestamp_pattern, &timestamp_format) {
                    Ok(extractor) => match extractor.extract(&sample_lines[0]) {
                        Ok(Some(ts)) => {
                            println!("  Parsed timestamp: {}", ts);
                        }
                        Ok(None) => {
                            println!(
                                "  Warning: Pattern did not match the first sample line."
                            );
                            println!("  You may want to adjust the pattern later.");
                        }
                        Err(e) => {
                            println!("  Warning: Timestamp parse error: {}", e);
                            println!("  The pattern matched but the format may be wrong.");
                        }
                    },
                    Err(e) => {
                        println!("  Warning: {}", e);
                    }
                }
            }

            // Source ID
            let default_id = derive_source_id(&file_path);
            let source_id: String = Input::new()
                .with_prompt("Source ID")
                .default(default_id)
                .interact_text()?;

            // Read start
            let start_options = &["beginning", "end", "stored_offset"];
            let start_idx = Select::new()
                .with_prompt("Where to start reading")
                .items(start_options)
                .default(0)
                .interact()?;
            let read_start = start_options[start_idx].to_string();

            // Follow
            let follow = Confirm::new()
                .with_prompt("Follow file for new lines?")
                .default(true)
                .interact()?;

            sources.push(SourceEntry {
                id: source_id,
                path: file_path,
                timestamp_pattern,
                timestamp_format,
                read_start,
                follow,
            });

            println!("Source added.");
            println!();

            if !Confirm::new()
                .with_prompt("Add another source?")
                .default(false)
                .interact()?
            {
                break;
            }
        }
    }

    // Collector listen address (also used as web server address)
    if mode == "collector" {
        println!();
        let listen: String = Input::new()
            .with_prompt("Collector listen address")
            .default("0.0.0.0:7105".to_string())
            .interact_text()?;
        collector_listen = Some(listen);
    }


    // Parent mode: collector endpoints
    if mode == "parent" {
        loop {
            println!();
            println!("--- Add a collector endpoint ---");

            let id: String = Input::new()
                .with_prompt("Collector ID")
                .interact_text()?;

            let url: String = Input::new()
                .with_prompt("Collector URL (e.g., http://192.168.1.10:7105)")
                .interact_text()?;

            collector_endpoints.push(CollectorEndpoint { id, url });

            println!("Collector endpoint added.");

            if !Confirm::new()
                .with_prompt("Add another collector?")
                .default(false)
                .interact()?
            {
                break;
            }
        }
    }

    // Common settings
    println!();
    println!("--- General settings ---");

    let default_storage = match mode.as_str() {
        "collector" => "$env{TMPDIR}/noil-collector.duckdb".to_string(),
        "parent" => "$env{TMPDIR}/noil-parent.duckdb".to_string(),
        _ => "/var/lib/noil/noil.duckdb".to_string(),
    };
    let storage_path: String = Input::new()
        .with_prompt("Storage database path")
        .default(default_storage)
        .interact_text()?;

    let web_listen = if mode == "collector" {
        // In collector mode, web server shares the collector listen address
        collector_listen.clone().unwrap_or_else(|| "0.0.0.0:7105".to_string())
    } else {
        let default_web = if mode == "parent" {
            "0.0.0.0:7104".to_string()
        } else {
            "127.0.0.1:7104".to_string()
        };
        Input::new()
            .with_prompt("Web server listen address")
            .default(default_web)
            .interact_text()?
    };

    // Output destination
    let output_path = if stdout {
        None
    } else {
        let dest_options = &["Write to a file", "Print to stdout"];
        let dest_idx = Select::new()
            .with_prompt("Where to write the config")
            .items(dest_options)
            .default(0)
            .interact()?;
        if dest_idx == 0 {
            let default_path = dirs::home_dir()
                .map(|h| h.join(".config/noil/config.yml"))
                .unwrap_or_else(|| PathBuf::from("/etc/noil/config.yml"));
            let path_str: String = Input::new()
                .with_prompt("Config file path")
                .default(default_path.display().to_string())
                .interact_text()?;
            Some(PathBuf::from(path_str))
        } else {
            None
        }
    };

    // Build config
    let config = InteractiveConfig {
        mode,
        sources,
        collector_listen,
        collector_endpoints,
        storage_path,
        web_listen,
    };

    let yaml = yaml_builder::build_yaml(&config);

    Ok(InteractiveResult {
        yaml,
        output_path,
    })
}

fn read_sample_lines(path: &PathBuf, count: usize) -> Vec<String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = io::BufReader::new(file);
    let mut lines = Vec::new();

    for line in reader.lines() {
        match line {
            Ok(l) => {
                let trimmed = l.trim_end().to_string();
                if !trimmed.is_empty() {
                    lines.push(trimmed);
                    if lines.len() >= count {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }

    lines
}

fn derive_source_id(path: &PathBuf) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("source");

    // Replace non-alphanumeric chars with underscores
    let mut id: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();

    // Ensure it doesn't start with a digit
    if id.starts_with(|c: char| c.is_ascii_digit()) {
        id = format!("source_{}", id);
    }

    if id.is_empty() {
        "source".to_string()
    } else {
        id
    }
}
