use super::types::*;
use crate::config::expand_tilde;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse YAML: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("validation failed:\n{}", .0.join("\n"))]
    ValidationList(Vec<String>),

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::traits::StorageError),
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let (config, _) = load_config_with_yaml(path)?;
    Ok(config)
}

/// Load config and return both the parsed config and the original YAML string
pub fn load_config_with_yaml(path: &Path) -> Result<(Config, String), ConfigError> {
    use std::io::Read;

    let mut file = File::open(path).map_err(|e| {
        ConfigError::Io(std::io::Error::new(
            e.kind(),
            format!("failed to open config file '{}': {}", path.display(), e),
        ))
    })?;

    let mut yaml_string = String::new();
    file.read_to_string(&mut yaml_string).map_err(|e| {
        ConfigError::Io(std::io::Error::new(
            e.kind(),
            format!("failed to read config file '{}': {}", path.display(), e),
        ))
    })?;

    let mut config: Config = serde_yaml::from_str(&yaml_string).map_err(|e| {
        // Wrap error with file context
        ConfigError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("in file '{}': {}", path.display(), e),
        ))
    })?;

    // Expand tilde in all paths
    expand_paths(&mut config);

    // Add automatic source fibers if enabled
    if config.auto_source_fibers {
        add_auto_source_fibers(&mut config);
    }

    validate_config(&config)?;

    Ok((config, yaml_string))
}

/// Expands tilde (~) in all PathBuf fields in the config.
fn expand_paths(config: &mut Config) {
    // Expand source paths
    for source in config.sources.values_mut() {
        source.path = expand_tilde(&source.path);
    }

    // Expand storage path
    config.storage.path = expand_tilde(&config.storage.path);
}

/// Automatically generates a fiber type for each source that collects all logs
/// from that source into a single never-closing fiber. This provides a convenient
/// jumping-off point for UI navigation.
fn add_auto_source_fibers(config: &mut Config) {
    for source_name in config.sources.keys() {
        let fiber_type_name = source_name.clone();

        // Skip if a fiber type with this name already exists
        if config.fiber_types.contains_key(&fiber_type_name) {
            continue;
        }

        // Create a never-closing fiber type that matches all logs from this source
        let mut source_patterns = HashMap::new();
        source_patterns.insert(
            source_name.clone(),
            FiberSourceConfig {
                patterns: vec![PatternConfig {
                    regex: ".+".to_string(),
                    release_matching_peer_keys: vec![],
                    release_self_keys: vec![],
                    close: false,
                }],
            },
        );

        let fiber_type = FiberTypeConfig {
            description: Some(format!(
                "Auto-generated fiber containing all logs from {}",
                source_name
            )),
            temporal: TemporalConfig {
                max_gap: None, // infinite - never closes due to time
                gap_mode: GapMode::Session,
            },
            attributes: vec![AttributeConfig {
                name: "source_marker".to_string(),
                attr_type: AttributeType::String,
                key: true,
                derived: Some(source_name.clone()),
            }],
            sources: source_patterns,
            is_source_fiber: true,
        };

        config.fiber_types.insert(fiber_type_name, fiber_type);
    }
}

fn validate_config(config: &Config) -> Result<(), ConfigError> {
    let mut errors = Vec::new();

    // Validate each fiber type
    for (fiber_type_name, fiber_type) in &config.fiber_types {
        validate_fiber_type(fiber_type_name, fiber_type, &config.sources, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::ValidationList(errors))
    }
}

fn validate_fiber_type(
    fiber_type_name: &str,
    fiber_type: &FiberTypeConfig,
    sources: &HashMap<String, SourceConfig>,
    errors: &mut Vec<String>,
) {
    let prefix = format!("fiber_type '{}'", fiber_type_name);

    // Check that all referenced sources exist
    for (source_name, _) in &fiber_type.sources {
        if !sources.contains_key(source_name) {
            errors.push(format!(
                "{}: references non-existent source '{}'",
                prefix, source_name
            ));
        }
    }

    // Validate timestamp patterns for referenced sources
    for (source_name, source_config) in sources {
        if fiber_type.sources.contains_key(source_name) {
            validate_timestamp_pattern(
                &format!("source '{}'", source_name),
                &source_config.timestamp.pattern,
                errors,
            );
        }
    }

    // Build attribute map for validation
    let mut attr_map: HashMap<String, &AttributeConfig> = HashMap::new();
    let mut attr_names = HashSet::new();

    for attr in &fiber_type.attributes {
        if !attr_names.insert(&attr.name) {
            errors.push(format!(
                "{}: duplicate attribute name '{}'",
                prefix, attr.name
            ));
        }
        attr_map.insert(attr.name.clone(), attr);
    }

    // Validate patterns
    for (source_name, source_config) in &fiber_type.sources {
        for (i, pattern) in source_config.patterns.iter().enumerate() {
            validate_pattern(
                &format!("{}, source '{}', pattern {}", prefix, source_name, i),
                pattern,
                &attr_map,
                errors,
            );
        }
    }

    // Validate derived attributes for circular dependencies
    validate_derived_attributes(&prefix, &fiber_type.attributes, errors);
}

fn validate_timestamp_pattern(context: &str, pattern: &str, errors: &mut Vec<String>) {
    match Regex::new(pattern) {
        Ok(re) => {
            // Check if it has a 'ts' capture group
            if re.capture_names().all(|name| name != Some("ts")) {
                errors.push(format!(
                    "{}: timestamp pattern must contain named capture group 'ts': {}",
                    context, pattern
                ));
            }
        }
        Err(e) => {
            errors.push(format!(
                "{}: invalid timestamp pattern regex: {} (error: {})",
                context, pattern, e
            ));
        }
    }
}

fn validate_pattern(
    context: &str,
    pattern: &PatternConfig,
    attr_map: &HashMap<String, &AttributeConfig>,
    errors: &mut Vec<String>,
) {
    // Check that regex compiles
    let re = match Regex::new(&pattern.regex) {
        Ok(re) => re,
        Err(e) => {
            errors.push(format!(
                "{}: invalid regex '{}': {}",
                context, pattern.regex, e
            ));
            return;
        }
    };

    // Extract capture group names from regex
    let capture_names: HashSet<String> = re
        .capture_names()
        .flatten()
        .map(|s| s.to_string())
        .collect();

    // Validate release_matching_peer_keys
    for key_name in &pattern.release_matching_peer_keys {
        // Must be a capture group in the pattern
        if !capture_names.contains(key_name) {
            errors.push(format!(
                "{}: release_matching_peer_keys contains '{}' which is not a capture group in the pattern",
                context, key_name
            ));
        }

        // Must be a defined key attribute
        match attr_map.get(key_name) {
            Some(attr) if attr.key => {
                // Valid
            }
            Some(_) => {
                errors.push(format!(
                    "{}: release_matching_peer_keys contains '{}' which is not marked as a key",
                    context, key_name
                ));
            }
            None => {
                errors.push(format!(
                    "{}: release_matching_peer_keys contains '{}' which is not a defined attribute",
                    context, key_name
                ));
            }
        }
    }

    // Validate release_self_keys
    for key_name in &pattern.release_self_keys {
        match attr_map.get(key_name) {
            Some(attr) if attr.key => {
                // Valid
            }
            Some(_) => {
                errors.push(format!(
                    "{}: release_self_keys contains '{}' which is not marked as a key",
                    context, key_name
                ));
            }
            None => {
                errors.push(format!(
                    "{}: release_self_keys contains '{}' which is not a defined attribute",
                    context, key_name
                ));
            }
        }
    }
}

fn validate_derived_attributes(
    context: &str,
    attributes: &[AttributeConfig],
    errors: &mut Vec<String>,
) {
    // Build attribute name set
    let attr_names: HashSet<&str> = attributes.iter().map(|a| a.name.as_str()).collect();

    // For each derived attribute, check that all referenced attributes exist
    for attr in attributes {
        if let Some(derived) = &attr.derived {
            let references = extract_variable_references(derived);
            for reference in &references {
                if !attr_names.contains(reference.as_str()) {
                    errors.push(format!(
                        "{}: derived attribute '{}' references non-existent attribute '{}'",
                        context, attr.name, reference
                    ));
                }
            }
        }
    }

    // Check for circular dependencies using topological sort
    if let Err(cycle) = topological_sort_attributes(attributes) {
        errors.push(format!(
            "{}: circular dependency in derived attributes: {}",
            context, cycle
        ));
    }
}

fn extract_variable_references(template: &str) -> Vec<String> {
    let re = Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").unwrap();
    re.captures_iter(template)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn topological_sort_attributes(attributes: &[AttributeConfig]) -> Result<Vec<String>, String> {
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();

    // Initialize
    for attr in attributes {
        graph.insert(&attr.name, Vec::new());
        in_degree.insert(&attr.name, 0);
    }

    // Build dependency graph (edges point from dependency to dependent)
    for attr in attributes {
        if let Some(derived) = &attr.derived {
            let references = extract_variable_references(derived);
            for reference in references {
                if let Some(deps) = graph.get_mut(reference.as_str()) {
                    deps.push(&attr.name);
                    *in_degree.get_mut(attr.name.as_str()).unwrap() += 1;
                }
            }
        }
    }

    // Kahn's algorithm
    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &degree)| degree == 0)
        .map(|(&name, _)| name)
        .collect();

    let mut sorted = Vec::new();

    while let Some(node) = queue.pop() {
        sorted.push(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for &neighbor in neighbors {
                let degree = in_degree.get_mut(neighbor).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push(neighbor);
                }
            }
        }
    }

    if sorted.len() != attributes.len() {
        // Find a node that's part of the cycle
        let remaining: Vec<&str> = in_degree
            .iter()
            .filter(|(_, &degree)| degree > 0)
            .map(|(&name, _)| name)
            .collect();
        Err(format!("cycle involving: {}", remaining.join(", ")))
    } else {
        Ok(sorted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_variable_references() {
        let template = "${ip}:${port}->${dst_ip}:${dst_port}";
        let refs = extract_variable_references(template);
        assert_eq!(refs, vec!["ip", "port", "dst_ip", "dst_port"]);
    }

    #[test]
    fn test_extract_no_references() {
        let template = "static_value";
        let refs = extract_variable_references(template);
        assert_eq!(refs, Vec::<String>::new());
    }
}
