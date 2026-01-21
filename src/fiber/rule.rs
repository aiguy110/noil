use crate::config::types::{AttributeType, FiberTypeConfig, GapMode, PatternConfig};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use thiserror::Error;

/// Error type for fiber rule compilation
#[derive(Debug, Error)]
pub enum RuleError {
    #[error("regex compilation error for pattern '{pattern}': {source}")]
    RegexCompilation {
        pattern: String,
        #[source]
        source: regex::Error,
    },

    #[error("circular dependency detected in derived attributes: {0}")]
    CircularDependency(String),

    #[error("derived attribute '{attr}' references undefined attribute '{reference}'")]
    UndefinedReference { attr: String, reference: String },

    #[error("release_matching_peer_keys '{key}' in pattern is not extractable by this pattern")]
    KeyNotExtractable { key: String },

    #[error("release_matching_peer_keys '{key}' is not marked as a key in attributes")]
    ReleaseMatchingNotKey { key: String },

    #[error("release_self_keys '{key}' is not marked as a key in attributes")]
    ReleaseSelfNotKey { key: String },

    #[error("duplicate attribute name: {0}")]
    DuplicateAttribute(String),
}

/// Temporal configuration for a fiber type
#[derive(Debug, Clone)]
pub struct TemporalConfig {
    /// Maximum time gap between logs (None = infinite)
    pub max_gap: Option<Duration>,
    /// Gap mode: session (from last log) or from_start (from first log)
    pub gap_mode: GapMode,
}

/// Definition for an attribute
#[derive(Debug, Clone)]
pub struct AttributeDef {
    pub name: String,
    pub attr_type: AttributeType,
    pub key: bool,
    pub derived: Option<String>,
}

/// Template for derived attribute computation
#[derive(Debug, Clone)]
pub struct DerivedTemplate {
    /// The original template string (e.g., "${ip}:${port}")
    pub template: String,
    /// Attribute names referenced by this template
    pub dependencies: Vec<String>,
}

impl DerivedTemplate {
    /// Parse a template string and extract dependencies
    fn from_template(template: &str) -> Self {
        let mut dependencies = Vec::new();
        let mut i = 0;
        let chars: Vec<char> = template.chars().collect();

        while i < chars.len() {
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' {
                // Find closing brace
                let start = i + 2;
                let mut end = start;
                while end < chars.len() && chars[end] != '}' {
                    end += 1;
                }
                if end < chars.len() {
                    let dep_name: String = chars[start..end].iter().collect();
                    if !dep_name.is_empty() && !dependencies.contains(&dep_name) {
                        dependencies.push(dep_name);
                    }
                    i = end + 1;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }

        Self {
            template: template.to_string(),
            dependencies,
        }
    }

    /// Interpolate the template with given values
    pub fn interpolate(&self, values: &HashMap<String, String>) -> Option<String> {
        // Check if all dependencies are satisfied
        for dep in &self.dependencies {
            if !values.contains_key(dep) {
                return None;
            }
        }

        // Perform substitution
        let mut result = self.template.clone();
        for dep in &self.dependencies {
            let placeholder = format!("${{{}}}", dep);
            if let Some(value) = values.get(dep) {
                result = result.replace(&placeholder, value);
            }
        }

        Some(result)
    }
}

/// A compiled pattern for matching log lines
#[derive(Debug)]
pub struct CompiledPattern {
    /// The compiled regex
    pub regex: Regex,
    /// Keys to release from peer fibers before processing
    pub release_matching_peer_keys: Vec<String>,
    /// Keys to release from self after processing
    pub release_self_keys: Vec<String>,
    /// Whether to close the fiber after this pattern matches
    pub close: bool,
    /// Keys this pattern can extract (capture groups that are marked as keys)
    pub extracted_keys: HashSet<String>,
    /// All capture groups in this pattern
    pub capture_groups: HashSet<String>,
}

impl CompiledPattern {
    fn from_config(
        config: &PatternConfig,
        key_names: &HashSet<String>,
    ) -> Result<Self, RuleError> {
        let regex = Regex::new(&config.regex).map_err(|e| RuleError::RegexCompilation {
            pattern: config.regex.clone(),
            source: e,
        })?;

        // Get all named capture groups
        let capture_groups: HashSet<String> = regex
            .capture_names()
            .flatten()
            .map(|s| s.to_string())
            .collect();

        // Determine which capture groups are keys
        let extracted_keys: HashSet<String> = capture_groups
            .iter()
            .filter(|name| key_names.contains(*name))
            .cloned()
            .collect();

        // Validate release_matching_peer_keys
        for key in &config.release_matching_peer_keys {
            if !capture_groups.contains(key) {
                return Err(RuleError::KeyNotExtractable { key: key.clone() });
            }
            if !key_names.contains(key) {
                return Err(RuleError::ReleaseMatchingNotKey { key: key.clone() });
            }
        }

        // Validate release_self_keys
        for key in &config.release_self_keys {
            if !key_names.contains(key) {
                return Err(RuleError::ReleaseSelfNotKey { key: key.clone() });
            }
        }

        Ok(Self {
            regex,
            release_matching_peer_keys: config.release_matching_peer_keys.clone(),
            release_self_keys: config.release_self_keys.clone(),
            close: config.close,
            extracted_keys,
            capture_groups,
        })
    }
}

/// A compiled fiber type ready for processing
#[derive(Debug)]
pub struct CompiledFiberType {
    /// Name of this fiber type
    pub name: String,
    /// Temporal configuration
    pub temporal: TemporalConfig,
    /// All attribute definitions
    pub attributes: Vec<AttributeDef>,
    /// Names of attributes that are keys
    pub key_names: HashSet<String>,
    /// Topologically sorted order for derived attribute computation
    pub derived_order: Vec<String>,
    /// Templates for derived attributes
    pub derived_templates: HashMap<String, DerivedTemplate>,
    /// Patterns for each source
    pub source_patterns: HashMap<String, Vec<CompiledPattern>>,
}

impl CompiledFiberType {
    /// Compile a fiber type from configuration
    pub fn from_config(name: &str, config: &FiberTypeConfig) -> Result<Self, RuleError> {
        // Check for duplicate attribute names
        let mut attr_names: HashSet<String> = HashSet::new();
        for attr in &config.attributes {
            if !attr_names.insert(attr.name.clone()) {
                return Err(RuleError::DuplicateAttribute(attr.name.clone()));
            }
        }

        // Collect key names
        let key_names: HashSet<String> = config
            .attributes
            .iter()
            .filter(|a| a.key)
            .map(|a| a.name.clone())
            .collect();

        // Build attribute definitions
        let attributes: Vec<AttributeDef> = config
            .attributes
            .iter()
            .map(|a| AttributeDef {
                name: a.name.clone(),
                attr_type: a.attr_type,
                key: a.key,
                derived: a.derived.clone(),
            })
            .collect();

        // Build derived templates and validate references
        let mut derived_templates = HashMap::new();
        for attr in &config.attributes {
            if let Some(ref template) = attr.derived {
                let dt = DerivedTemplate::from_template(template);

                // Validate that all references exist
                for dep in &dt.dependencies {
                    if !attr_names.contains(dep) {
                        return Err(RuleError::UndefinedReference {
                            attr: attr.name.clone(),
                            reference: dep.clone(),
                        });
                    }
                }

                derived_templates.insert(attr.name.clone(), dt);
            }
        }

        // Topological sort of derived attributes
        let derived_order = topological_sort_derived(&derived_templates)?;

        // Compile patterns for each source
        let mut source_patterns = HashMap::new();
        for (source_id, source_config) in &config.sources {
            let mut patterns = Vec::new();
            for pattern_config in &source_config.patterns {
                let compiled = CompiledPattern::from_config(pattern_config, &key_names)?;
                patterns.push(compiled);
            }
            source_patterns.insert(source_id.clone(), patterns);
        }

        Ok(Self {
            name: name.to_string(),
            temporal: TemporalConfig {
                max_gap: config.temporal.max_gap,
                gap_mode: config.temporal.gap_mode,
            },
            attributes,
            key_names,
            derived_order,
            derived_templates,
            source_patterns,
        })
    }

    /// Get the attribute type for a given attribute name
    pub fn get_attribute_type(&self, name: &str) -> Option<AttributeType> {
        self.attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.attr_type)
    }
}

/// Perform topological sort on derived attributes based on their dependencies
fn topological_sort_derived(
    templates: &HashMap<String, DerivedTemplate>,
) -> Result<Vec<String>, RuleError> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut in_progress = HashSet::new();

    fn visit(
        name: &str,
        templates: &HashMap<String, DerivedTemplate>,
        visited: &mut HashSet<String>,
        in_progress: &mut HashSet<String>,
        result: &mut Vec<String>,
    ) -> Result<(), RuleError> {
        if visited.contains(name) {
            return Ok(());
        }
        if in_progress.contains(name) {
            return Err(RuleError::CircularDependency(name.to_string()));
        }

        if let Some(template) = templates.get(name) {
            in_progress.insert(name.to_string());

            for dep in &template.dependencies {
                // Only visit if dependency is itself derived
                if templates.contains_key(dep) {
                    visit(dep, templates, visited, in_progress, result)?;
                }
            }

            in_progress.remove(name);
        }

        visited.insert(name.to_string());
        if templates.contains_key(name) {
            result.push(name.to_string());
        }
        Ok(())
    }

    for name in templates.keys() {
        visit(name, templates, &mut visited, &mut in_progress, &mut result)?;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{
        AttributeConfig, FiberSourceConfig, FiberTypeConfig, PatternConfig,
        TemporalConfig as ConfigTemporalConfig,
    };

    fn make_basic_fiber_type() -> FiberTypeConfig {
        FiberTypeConfig {
            description: Some("Test fiber type".to_string()),
            temporal: ConfigTemporalConfig {
                max_gap: Some(Duration::from_secs(5)),
                gap_mode: GapMode::Session,
            },
            attributes: vec![
                AttributeConfig {
                    name: "thread_id".to_string(),
                    attr_type: AttributeType::String,
                    key: true,
                    derived: None,
                },
                AttributeConfig {
                    name: "ip".to_string(),
                    attr_type: AttributeType::Ip,
                    key: false,
                    derived: None,
                },
            ],
            sources: {
                let mut sources = HashMap::new();
                sources.insert(
                    "program1".to_string(),
                    FiberSourceConfig {
                        patterns: vec![PatternConfig {
                            regex: r"thread-(?P<thread_id>\d+)".to_string(),
                            release_matching_peer_keys: vec![],
                            release_self_keys: vec![],
                            close: false,
                        }],
                    },
                );
                sources
            },
            is_source_fiber: false,
        }
    }

    #[test]
    fn test_compile_basic_fiber_type() {
        let config = make_basic_fiber_type();
        let compiled = CompiledFiberType::from_config("test", &config).unwrap();

        assert_eq!(compiled.name, "test");
        assert_eq!(compiled.key_names.len(), 1);
        assert!(compiled.key_names.contains("thread_id"));
        assert_eq!(compiled.attributes.len(), 2);
        assert!(compiled.source_patterns.contains_key("program1"));
    }

    #[test]
    fn test_derived_template_parsing() {
        let template = DerivedTemplate::from_template("${ip}:${port}->${dst_ip}:${dst_port}");
        assert_eq!(template.dependencies.len(), 4);
        assert!(template.dependencies.contains(&"ip".to_string()));
        assert!(template.dependencies.contains(&"port".to_string()));
        assert!(template.dependencies.contains(&"dst_ip".to_string()));
        assert!(template.dependencies.contains(&"dst_port".to_string()));
    }

    #[test]
    fn test_derived_template_no_deps() {
        let template = DerivedTemplate::from_template("static_value");
        assert!(template.dependencies.is_empty());
    }

    #[test]
    fn test_derived_template_interpolation() {
        let template = DerivedTemplate::from_template("${ip}:${port}");
        let mut values = HashMap::new();
        values.insert("ip".to_string(), "10.0.0.1".to_string());
        values.insert("port".to_string(), "8080".to_string());

        let result = template.interpolate(&values);
        assert_eq!(result, Some("10.0.0.1:8080".to_string()));
    }

    #[test]
    fn test_derived_template_interpolation_missing_dep() {
        let template = DerivedTemplate::from_template("${ip}:${port}");
        let mut values = HashMap::new();
        values.insert("ip".to_string(), "10.0.0.1".to_string());
        // Missing port

        let result = template.interpolate(&values);
        assert!(result.is_none());
    }

    #[test]
    fn test_duplicate_attribute_error() {
        let mut config = make_basic_fiber_type();
        config.attributes.push(AttributeConfig {
            name: "thread_id".to_string(), // Duplicate
            attr_type: AttributeType::String,
            key: false,
            derived: None,
        });

        let result = CompiledFiberType::from_config("test", &config);
        assert!(matches!(result, Err(RuleError::DuplicateAttribute(_))));
    }

    #[test]
    fn test_release_matching_peer_keys_not_extractable() {
        let mut config = make_basic_fiber_type();
        // Add a pattern with release_matching_peer_keys that doesn't extract the key
        config.sources.get_mut("program1").unwrap().patterns[0]
            .release_matching_peer_keys
            .push("thread_id".to_string());

        // This should succeed because thread_id IS extracted by the pattern
        let result = CompiledFiberType::from_config("test", &config);
        assert!(result.is_ok());

        // Now try with a key that's not extracted
        config.attributes.push(AttributeConfig {
            name: "other_key".to_string(),
            attr_type: AttributeType::String,
            key: true,
            derived: None,
        });
        config.sources.get_mut("program1").unwrap().patterns[0]
            .release_matching_peer_keys
            .push("other_key".to_string());

        let result = CompiledFiberType::from_config("test", &config);
        assert!(matches!(result, Err(RuleError::KeyNotExtractable { .. })));
    }

    #[test]
    fn test_release_self_keys_not_a_key() {
        let mut config = make_basic_fiber_type();
        // Try to release ip which is not a key
        config.sources.get_mut("program1").unwrap().patterns[0]
            .release_self_keys
            .push("ip".to_string());

        let result = CompiledFiberType::from_config("test", &config);
        assert!(matches!(result, Err(RuleError::ReleaseSelfNotKey { .. })));
    }

    #[test]
    fn test_derived_circular_dependency() {
        let config = FiberTypeConfig {
            description: None,
            temporal: ConfigTemporalConfig {
                max_gap: Some(Duration::from_secs(5)),
                gap_mode: GapMode::Session,
            },
            attributes: vec![
                AttributeConfig {
                    name: "a".to_string(),
                    attr_type: AttributeType::String,
                    key: false,
                    derived: Some("${b}".to_string()),
                },
                AttributeConfig {
                    name: "b".to_string(),
                    attr_type: AttributeType::String,
                    key: false,
                    derived: Some("${a}".to_string()),
                },
            ],
            sources: HashMap::new(),
            is_source_fiber: false,
        };

        let result = CompiledFiberType::from_config("test", &config);
        assert!(matches!(result, Err(RuleError::CircularDependency(_))));
    }

    #[test]
    fn test_derived_undefined_reference() {
        let config = FiberTypeConfig {
            description: None,
            temporal: ConfigTemporalConfig {
                max_gap: Some(Duration::from_secs(5)),
                gap_mode: GapMode::Session,
            },
            attributes: vec![AttributeConfig {
                name: "a".to_string(),
                attr_type: AttributeType::String,
                key: false,
                derived: Some("${undefined}".to_string()),
            }],
            sources: HashMap::new(),
            is_source_fiber: false,
        };

        let result = CompiledFiberType::from_config("test", &config);
        assert!(matches!(result, Err(RuleError::UndefinedReference { .. })));
    }

    #[test]
    fn test_topological_sort() {
        let mut templates = HashMap::new();
        templates.insert(
            "c".to_string(),
            DerivedTemplate::from_template("${a}${b}"),
        );
        templates.insert(
            "a".to_string(),
            DerivedTemplate::from_template("static"),
        );
        templates.insert(
            "b".to_string(),
            DerivedTemplate::from_template("${a}"),
        );

        let order = topological_sort_derived(&templates).unwrap();

        // a must come before b and c, b must come before c
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        let c_pos = order.iter().position(|x| x == "c").unwrap();

        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn test_pattern_extracts_keys() {
        let key_names: HashSet<String> = ["thread_id".to_string(), "mac".to_string()]
            .into_iter()
            .collect();

        let config = PatternConfig {
            regex: r"thread-(?P<thread_id>\d+).*MAC (?P<mac>[0-9a-f:]+)".to_string(),
            release_matching_peer_keys: vec![],
            release_self_keys: vec![],
            close: false,
        };

        let compiled = CompiledPattern::from_config(&config, &key_names).unwrap();

        assert!(compiled.extracted_keys.contains("thread_id"));
        assert!(compiled.extracted_keys.contains("mac"));
        assert_eq!(compiled.extracted_keys.len(), 2);
    }
}
