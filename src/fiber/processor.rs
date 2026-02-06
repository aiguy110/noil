use crate::config::types::{Config, GapMode};
use crate::fiber::rule::{CompiledFiberType, CompiledPattern, RuleError};
use crate::fiber::session::{AttributeValue, OpenFiber};
use crate::source::reader::LogRecord;
use crate::storage::traits::{FiberMembership, FiberRecord};
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use tracing::warn;
use uuid::Uuid;

/// Information extracted from a matched pattern (owned, to avoid borrow issues)
struct PatternMatchInfo {
    extracted: HashMap<String, String>,
    release_matching_peer_keys: Vec<String>,
    release_self_keys: Vec<String>,
    close: bool,
}

/// Result from processing a single log record for a single fiber type
#[derive(Debug, Default)]
pub struct ProcessResult {
    /// New fiber memberships (log -> fiber)
    pub memberships: Vec<FiberMembership>,
    /// Newly created fibers
    pub new_fibers: Vec<FiberRecord>,
    /// Updated fibers (attributes changed, merged, etc.)
    pub updated_fibers: Vec<FiberRecord>,
    /// IDs of fibers that were closed
    pub closed_fiber_ids: Vec<Uuid>,
    /// IDs of fibers that were merged into other fibers
    pub merged_fiber_ids: Vec<Uuid>,
}

/// Processor for a single fiber type
pub struct FiberTypeProcessor {
    /// Compiled fiber type rules
    fiber_type: CompiledFiberType,
    /// Config version for storage records
    config_version: u64,
    /// Open (active) fibers
    open_fibers: HashMap<Uuid, OpenFiber>,
    /// Key index: (key_name, value) -> fiber_id
    key_index: HashMap<(String, String), Uuid>,
    /// Logical clock (timestamp of most recently processed log)
    logical_clock: Option<DateTime<Utc>>,
}

impl FiberTypeProcessor {
    /// Create a new fiber type processor
    pub fn new(fiber_type: CompiledFiberType, config_version: u64) -> Self {
        Self {
            fiber_type,
            config_version,
            open_fibers: HashMap::new(),
            key_index: HashMap::new(),
            logical_clock: None,
        }
    }

    /// Get the fiber type name
    pub fn fiber_type_name(&self) -> &str {
        &self.fiber_type.name
    }

    /// Process a log record
    pub fn process_log(&mut self, log: &LogRecord) -> ProcessResult {
        let mut result = ProcessResult::default();

        // Update logical clock
        self.logical_clock = Some(log.timestamp);

        // Step 1: Find matching patterns for this log's source
        let patterns = match self.fiber_type.source_patterns.get(&log.source_id) {
            Some(p) => p,
            None => {
                // This fiber type doesn't handle logs from this source
                // Check timeouts and return
                self.check_timeouts(&mut result);
                return result;
            }
        };

        // Step 2: Extract attributes using first matching pattern
        // We need to extract pattern info before mutating self
        let match_result = self.extract_attributes_with_info(log, patterns);

        let Some(match_info) = match_result else {
            // No pattern matched, check timeouts and return
            self.check_timeouts(&mut result);
            return result;
        };

        // Step 3: Compute derived attributes
        let mut all_attrs = match_info.extracted.clone();
        self.compute_derived_attributes(&mut all_attrs);

        // Step 4: Execute release_matching_peer_keys
        self.release_matching_peer_keys_by_name(&match_info.release_matching_peer_keys, &match_info.extracted, &mut result);

        // Step 5: Find matching fibers via key index
        let matching_fiber_ids = self.find_matching_fibers(&all_attrs);

        // Step 6: Create, join, or merge fibers
        let (target_fiber_id, is_new_fiber) = if matching_fiber_ids.is_empty() {
            // Create new fiber
            let fiber = OpenFiber::new(self.fiber_type.name.clone(), log.timestamp);
            let fiber_id = fiber.fiber_id;
            self.open_fibers.insert(fiber_id, fiber);
            (fiber_id, true)
        } else if matching_fiber_ids.len() == 1 {
            // Join existing fiber
            (matching_fiber_ids[0], false)
        } else {
            // Merge multiple fibers
            (self.merge_fibers(&matching_fiber_ids, &mut result), false)
        };

        // Step 7: Add log to fiber, update keys and attributes
        self.update_fiber_with_attributes(target_fiber_id, log, &all_attrs);

        // Record new fiber AFTER attributes are set (so FiberRecord has correct attributes)
        if is_new_fiber {
            result.new_fibers.push(self.fiber_to_record(target_fiber_id));
        }

        // Record membership
        result.memberships.push(FiberMembership {
            log_id: log.id,
            fiber_id: target_fiber_id,
            config_version: self.config_version,
        });

        // Step 8: Execute release_self_keys
        self.release_self_keys_by_name(&match_info.release_self_keys, target_fiber_id);

        // Step 9: Execute close if specified
        if match_info.close {
            self.close_fiber(target_fiber_id, &mut result);
        }

        // Step 10: Check for timeout closures
        self.check_timeouts(&mut result);

        // Mark the target fiber as updated
        if !result.new_fibers.iter().any(|f| f.fiber_id == target_fiber_id)
            && !result.closed_fiber_ids.contains(&target_fiber_id)
        {
            result.updated_fibers.push(self.fiber_to_record(target_fiber_id));
        }

        result
    }

    /// Extract attributes from a log using the first matching pattern
    /// Returns owned data to avoid borrow checker issues
    fn extract_attributes_with_info(
        &self,
        log: &LogRecord,
        patterns: &[CompiledPattern],
    ) -> Option<PatternMatchInfo> {
        for pattern in patterns {
            if let Some(captures) = pattern.regex.captures(&log.raw_text) {
                let mut extracted = HashMap::new();
                for name in &pattern.capture_groups {
                    if let Some(m) = captures.name(name) {
                        extracted.insert(name.clone(), m.as_str().to_string());
                    }
                }
                return Some(PatternMatchInfo {
                    extracted,
                    release_matching_peer_keys: pattern.release_matching_peer_keys.clone(),
                    release_self_keys: pattern.release_self_keys.clone(),
                    close: pattern.close,
                });
            }
        }
        None
    }

    /// Compute derived attributes based on extracted values
    fn compute_derived_attributes(&self, attrs: &mut HashMap<String, String>) {
        for derived_name in &self.fiber_type.derived_order {
            if let Some(template) = self.fiber_type.derived_templates.get(derived_name) {
                if let Some(value) = template.interpolate(attrs) {
                    attrs.insert(derived_name.clone(), value);
                }
            }
        }
    }

    /// Execute release_matching_peer_keys action (by key names)
    fn release_matching_peer_keys_by_name(
        &mut self,
        key_names: &[String],
        extracted: &HashMap<String, String>,
        result: &mut ProcessResult,
    ) {
        for key_name in key_names {
            if let Some(value) = extracted.get(key_name) {
                // Find fiber with this key
                let key_tuple = (key_name.clone(), value.clone());
                if let Some(&fiber_id) = self.key_index.get(&key_tuple) {
                    // Remove key from this fiber
                    if let Some(fiber) = self.open_fibers.get_mut(&fiber_id) {
                        fiber.remove_key(key_name);
                        self.key_index.remove(&key_tuple);

                        // Mark fiber as updated
                        if !result.updated_fibers.iter().any(|f| f.fiber_id == fiber_id) {
                            result.updated_fibers.push(self.fiber_to_record(fiber_id));
                        }
                    }
                }
            }
        }
    }

    /// Update a fiber with extracted attributes
    fn update_fiber_with_attributes(
        &mut self,
        fiber_id: Uuid,
        log: &LogRecord,
        all_attrs: &HashMap<String, String>,
    ) {
        if let Some(fiber) = self.open_fibers.get_mut(&fiber_id) {
            fiber.add_log(log.id, log.timestamp);

            // Collect keys to update (to avoid borrow issues with key_index)
            let mut key_updates: Vec<(String, String, Option<String>)> = Vec::new();

            // Add/update keys and attributes
            for (name, value) in all_attrs {
                // Update attribute
                let attr_type = self.fiber_type.get_attribute_type(name);
                if let Some(attr_type) = attr_type {
                    if let Some(typed_value) = AttributeValue::from_str(value, attr_type) {
                        if let Some(old_value) = fiber.set_attribute(name.clone(), typed_value) {
                            warn!(
                                fiber_id = %fiber.fiber_id,
                                attribute = %name,
                                old_value = ?old_value,
                                new_value = %value,
                                "Attribute value changed"
                            );
                        }
                    }
                }

                // If this is a key, track update
                if self.fiber_type.key_names.contains(name) {
                    let old_value = fiber.keys.get(name).cloned();
                    if old_value.as_ref() != Some(value) {
                        key_updates.push((name.clone(), value.clone(), old_value));
                    }
                    fiber.set_key(name.clone(), value.clone());
                }
            }

            // Update key index after releasing borrow on fiber
            for (name, value, old_value) in key_updates {
                if let Some(old) = old_value {
                    self.key_index.remove(&(name.clone(), old));
                }
                self.key_index.insert((name, value), fiber_id);
            }
        }
    }

    /// Find all fibers that match the extracted keys
    fn find_matching_fibers(&self, attrs: &HashMap<String, String>) -> Vec<Uuid> {
        let mut matching = HashSet::new();

        for (name, value) in attrs {
            if self.fiber_type.key_names.contains(name) {
                if let Some(&fiber_id) = self.key_index.get(&(name.clone(), value.clone())) {
                    matching.insert(fiber_id);
                }
            }
        }

        matching.into_iter().collect()
    }

    /// Merge multiple fibers into one, returning the survivor's ID
    fn merge_fibers(&mut self, fiber_ids: &[Uuid], result: &mut ProcessResult) -> Uuid {
        // Select survivor: oldest by first_activity
        let survivor_id = *fiber_ids
            .iter()
            .min_by_key(|id| {
                self.open_fibers
                    .get(id)
                    .map(|f| f.first_activity)
                    .unwrap_or(DateTime::<Utc>::MAX_UTC)
            })
            .unwrap();

        // Merge other fibers into survivor
        for &fiber_id in fiber_ids {
            if fiber_id == survivor_id {
                continue;
            }

            if let Some(other_fiber) = self.open_fibers.remove(&fiber_id) {
                // Update key index to point to survivor
                for (key_name, value) in &other_fiber.keys {
                    self.key_index
                        .insert((key_name.clone(), value.clone()), survivor_id);
                }

                // Merge into survivor
                if let Some(survivor) = self.open_fibers.get_mut(&survivor_id) {
                    let conflicts = survivor.merge(other_fiber);
                    for (attr_name, old_val, new_val) in conflicts {
                        warn!(
                            survivor_id = %survivor_id,
                            merged_id = %fiber_id,
                            attribute = %attr_name,
                            old_value = ?old_val,
                            new_value = ?new_val,
                            "Attribute conflict during fiber merge"
                        );
                    }
                }

                // Record merged fiber
                result.merged_fiber_ids.push(fiber_id);
            }
        }

        survivor_id
    }

    /// Execute release_self_keys action (by key names)
    fn release_self_keys_by_name(&mut self, key_names: &[String], fiber_id: Uuid) {
        if let Some(fiber) = self.open_fibers.get_mut(&fiber_id) {
            let mut to_remove = Vec::new();
            for key_name in key_names {
                if let Some(value) = fiber.remove_key(key_name) {
                    to_remove.push((key_name.clone(), value));
                }
            }
            for (key_name, value) in to_remove {
                self.key_index.remove(&(key_name, value));
            }
        }
    }

    /// Close a fiber
    fn close_fiber(&mut self, fiber_id: Uuid, result: &mut ProcessResult) {
        if let Some(fiber) = self.open_fibers.remove(&fiber_id) {
            // Remove all keys from index
            for (key_name, value) in fiber.keys {
                self.key_index.remove(&(key_name, value));
            }
            result.closed_fiber_ids.push(fiber_id);
        }
    }

    /// Check for fibers that should be closed due to timeout
    fn check_timeouts(&mut self, result: &mut ProcessResult) {
        let Some(logical_clock) = self.logical_clock else {
            return;
        };

        let Some(max_gap) = self.fiber_type.temporal.max_gap else {
            return; // Infinite max_gap, no timeouts
        };

        let max_gap = Duration::from_std(max_gap).unwrap();
        let mut to_close = Vec::new();

        for (&fiber_id, fiber) in &self.open_fibers {
            let reference_time = match self.fiber_type.temporal.gap_mode {
                GapMode::Session => fiber.last_activity,
                GapMode::FromStart => fiber.first_activity,
            };

            if logical_clock - reference_time > max_gap {
                to_close.push(fiber_id);
            }
        }

        for fiber_id in to_close {
            self.close_fiber(fiber_id, result);
        }
    }

    /// Convert an open fiber to a storage record
    fn fiber_to_record(&self, fiber_id: Uuid) -> FiberRecord {
        let fiber = self.open_fibers.get(&fiber_id).unwrap();

        // Convert attributes to JSON
        let attributes = serde_json::to_value(&fiber.attributes).unwrap_or(serde_json::Value::Null);

        FiberRecord {
            fiber_id: fiber.fiber_id,
            fiber_type: fiber.fiber_type.clone(),
            config_version: self.config_version,
            attributes,
            first_activity: fiber.first_activity,
            last_activity: fiber.last_activity,
            closed: false,
        }
    }

    /// Get the number of open fibers
    pub fn open_fiber_count(&self) -> usize {
        self.open_fibers.len()
    }

    /// Flush all open fibers (close them without timeout check)
    pub fn flush(&mut self) -> ProcessResult {
        let mut result = ProcessResult::default();
        let fiber_ids: Vec<Uuid> = self.open_fibers.keys().copied().collect();
        for fiber_id in fiber_ids {
            self.close_fiber(fiber_id, &mut result);
        }
        result
    }

    /// Create a checkpoint of this fiber type processor's state
    pub fn create_checkpoint(&self) -> crate::storage::checkpoint::FiberProcessorCheckpoint {
        let open_fibers = self
            .open_fibers
            .values()
            .map(|fiber| {
                // Convert AttributeValue to serde_json::Value for checkpoint serialization
                let attributes: HashMap<String, serde_json::Value> = fiber
                    .attributes
                    .iter()
                    .map(|(k, v)| {
                        let json_val = match v {
                            AttributeValue::String(s) => serde_json::Value::String(s.clone()),
                            AttributeValue::Int(i) => serde_json::Value::Number((*i).into()),
                            AttributeValue::Float(f) => {
                                serde_json::Number::from_f64(*f)
                                    .map(serde_json::Value::Number)
                                    .unwrap_or(serde_json::Value::Null)
                            }
                        };
                        (k.clone(), json_val)
                    })
                    .collect();

                crate::storage::checkpoint::OpenFiberCheckpoint {
                    fiber_id: fiber.fiber_id,
                    keys: fiber.keys.clone(),
                    attributes,
                    first_activity: fiber.first_activity,
                    last_activity: fiber.last_activity,
                    log_ids: fiber.log_ids.clone(),
                }
            })
            .collect();

        crate::storage::checkpoint::FiberProcessorCheckpoint {
            open_fibers,
            logical_clock: self.logical_clock.unwrap_or_else(|| Utc::now()),
        }
    }

    /// Restore fiber type processor state from a checkpoint
    pub fn restore_from_checkpoint(
        &mut self,
        checkpoint: &crate::storage::checkpoint::FiberProcessorCheckpoint,
    ) {
        // Clear existing state
        self.open_fibers.clear();
        self.key_index.clear();

        // Restore logical clock
        self.logical_clock = Some(checkpoint.logical_clock);

        // Restore open fibers
        for fiber_cp in &checkpoint.open_fibers {
            // Convert serde_json::Value back to AttributeValue
            let attributes: HashMap<String, AttributeValue> = fiber_cp
                .attributes
                .iter()
                .filter_map(|(k, v)| {
                    let attr_val = match v {
                        serde_json::Value::String(s) => Some(AttributeValue::String(s.clone())),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                Some(AttributeValue::Int(i))
                            } else if let Some(f) = n.as_f64() {
                                Some(AttributeValue::Float(f))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    attr_val.map(|v| (k.clone(), v))
                })
                .collect();

            let fiber = OpenFiber {
                fiber_id: fiber_cp.fiber_id,
                fiber_type: self.fiber_type.name.clone(),
                keys: fiber_cp.keys.clone(),
                attributes,
                first_activity: fiber_cp.first_activity,
                last_activity: fiber_cp.last_activity,
                log_ids: fiber_cp.log_ids.clone(),
            };

            // Rebuild key index
            for (key_name, value) in &fiber.keys {
                self.key_index
                    .insert((key_name.clone(), value.clone()), fiber.fiber_id);
            }

            self.open_fibers.insert(fiber.fiber_id, fiber);
        }
    }
}

/// Multi-type fiber processor that coordinates multiple FiberTypeProcessors
pub struct FiberProcessor {
    processors: HashMap<String, FiberTypeProcessor>,
}

impl FiberProcessor {
    /// Create a new fiber processor from configuration
    pub fn from_config(config: &Config, config_version: u64) -> Result<Self, RuleError> {
        let mut processors = HashMap::new();

        for (name, fiber_config) in config.fiber_types_or_empty() {
            let compiled = CompiledFiberType::from_config(name, fiber_config)?;
            let processor = FiberTypeProcessor::new(compiled, config_version);
            processors.insert(name.clone(), processor);
        }

        Ok(Self { processors })
    }

    /// Process a log record across all fiber types
    pub fn process_log(&mut self, log: &LogRecord) -> Vec<ProcessResult> {
        self.processors
            .values_mut()
            .map(|p| p.process_log(log))
            .collect()
    }

    /// Get the number of open fibers across all types
    pub fn total_open_fibers(&self) -> usize {
        self.processors.values().map(|p| p.open_fiber_count()).sum()
    }

    /// Get a reference to a specific fiber type processor
    pub fn get_processor(&self, fiber_type: &str) -> Option<&FiberTypeProcessor> {
        self.processors.get(fiber_type)
    }

    /// Get a mutable reference to a specific fiber type processor
    pub fn get_processor_mut(&mut self, fiber_type: &str) -> Option<&mut FiberTypeProcessor> {
        self.processors.get_mut(fiber_type)
    }

    /// Flush all open fibers across all types
    pub fn flush(&mut self) -> Vec<ProcessResult> {
        self.processors.values_mut().map(|p| p.flush()).collect()
    }

    /// Create a checkpoint of all fiber type processors
    pub fn create_checkpoint(
        &self,
    ) -> HashMap<String, crate::storage::checkpoint::FiberProcessorCheckpoint> {
        self.processors
            .iter()
            .map(|(name, processor)| (name.clone(), processor.create_checkpoint()))
            .collect()
    }

    /// Restore fiber processor state from a checkpoint
    pub fn restore_from_checkpoint(
        &mut self,
        checkpoints: &HashMap<String, crate::storage::checkpoint::FiberProcessorCheckpoint>,
    ) {
        for (fiber_type, checkpoint) in checkpoints {
            if let Some(processor) = self.processors.get_mut(fiber_type) {
                processor.restore_from_checkpoint(checkpoint);
            } else {
                warn!(
                    "Checkpoint contains fiber type '{}' not in current config, skipping",
                    fiber_type
                );
            }
        }
    }

    /// Check if a processor exists for the given source ID
    pub fn has_processor_for_source(&self, source_id: &str) -> bool {
        self.processors.values().any(|p| {
            p.fiber_type.source_patterns.contains_key(source_id)
        })
    }

    /// Dynamically add an auto-generated source fiber type processor.
    /// This is used in parent mode when encountering logs from a previously-unseen source.
    /// Returns true if a new processor was added, false if one already existed.
    ///
    /// Note: We only check if a source fiber processor already exists (keyed by source_id).
    /// We intentionally do NOT check `has_processor_for_source` because traced fiber types
    /// may handle the same source, and source fibers should be created independently of
    /// traced fibers to provide a "all logs from source" view.
    pub fn add_source_fiber_type(&mut self, source_id: &str, config_version: u64) -> Result<bool, RuleError> {
        // Check if we already have a source fiber processor for this source
        if self.processors.contains_key(source_id) {
            return Ok(false);
        }

        // Create auto-source fiber type config
        let fiber_config = crate::config::parse::create_auto_source_fiber_config(source_id);
        let compiled = CompiledFiberType::from_config(source_id, &fiber_config)?;
        let processor = FiberTypeProcessor::new(compiled, config_version);

        tracing::info!(
            source_id = %source_id,
            "Dynamically added source fiber type processor"
        );

        self.processors.insert(source_id.to_string(), processor);
        Ok(true)
    }

    /// Get a list of all fiber type names
    pub fn fiber_type_names(&self) -> Vec<String> {
        self.processors.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{
        AttributeConfig, AttributeType, FiberSourceConfig, FiberTypeConfig, GapMode,
        PatternConfig, TemporalConfig as ConfigTemporalConfig,
    };
    use std::time::Duration;

    fn make_log(source: &str, timestamp: &str, text: &str) -> LogRecord {
        LogRecord {
            id: Uuid::new_v4(),
            timestamp: timestamp.parse().unwrap(),
            source_id: source.to_string(),
            raw_text: text.to_string(),
            file_offset: 0,
        }
    }

    fn make_simple_fiber_type() -> FiberTypeConfig {
        FiberTypeConfig {
            description: Some("Simple test fiber".to_string()),
            temporal: ConfigTemporalConfig {
                max_gap: Some(Duration::from_secs(5)),
                gap_mode: GapMode::Session,
            },
            attributes: vec![AttributeConfig {
                name: "thread_id".to_string(),
                attr_type: AttributeType::String,
                key: true,
                derived: None,
            }],
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
    fn test_process_log_creates_fiber() {
        let config = make_simple_fiber_type();
        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        let log = make_log("program1", "2025-12-04T10:00:00Z", "thread-5 doing stuff");
        let result = processor.process_log(&log);

        assert_eq!(result.new_fibers.len(), 1);
        assert_eq!(result.memberships.len(), 1);
        assert_eq!(processor.open_fiber_count(), 1);
    }

    #[test]
    fn test_process_log_joins_existing_fiber() {
        let config = make_simple_fiber_type();
        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        // First log creates fiber
        let log1 = make_log("program1", "2025-12-04T10:00:00Z", "thread-5 doing stuff");
        let result1 = processor.process_log(&log1);
        assert_eq!(result1.new_fibers.len(), 1);
        let fiber_id = result1.new_fibers[0].fiber_id;

        // Second log with same thread joins
        let log2 = make_log("program1", "2025-12-04T10:00:01Z", "thread-5 more stuff");
        let result2 = processor.process_log(&log2);
        assert_eq!(result2.new_fibers.len(), 0);
        assert_eq!(result2.memberships.len(), 1);
        assert_eq!(result2.memberships[0].fiber_id, fiber_id);
        assert_eq!(processor.open_fiber_count(), 1);
    }

    #[test]
    fn test_process_log_different_key_creates_new_fiber() {
        let config = make_simple_fiber_type();
        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        let log1 = make_log("program1", "2025-12-04T10:00:00Z", "thread-5 doing stuff");
        processor.process_log(&log1);

        let log2 = make_log("program1", "2025-12-04T10:00:01Z", "thread-6 doing stuff");
        let result2 = processor.process_log(&log2);

        assert_eq!(result2.new_fibers.len(), 1);
        assert_eq!(processor.open_fiber_count(), 2);
    }

    #[test]
    fn test_timeout_closes_fiber() {
        let config = make_simple_fiber_type();
        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        // Create fiber
        let log1 = make_log("program1", "2025-12-04T10:00:00Z", "thread-5 doing stuff");
        processor.process_log(&log1);
        assert_eq!(processor.open_fiber_count(), 1);

        // Process log after timeout (6 seconds, max_gap is 5)
        let log2 = make_log("program1", "2025-12-04T10:00:06Z", "thread-6 other stuff");
        let result2 = processor.process_log(&log2);

        // First fiber should be closed due to timeout
        assert_eq!(result2.closed_fiber_ids.len(), 1);
        // New fiber created for thread-6
        assert_eq!(result2.new_fibers.len(), 1);
        assert_eq!(processor.open_fiber_count(), 1);
    }

    #[test]
    fn test_release_matching_peer_keys() {
        let mut config = make_simple_fiber_type();
        config.sources.get_mut("program1").unwrap().patterns = vec![
            PatternConfig {
                regex: r"thread-(?P<thread_id>\d+) START".to_string(),
                release_matching_peer_keys: vec!["thread_id".to_string()],
                release_self_keys: vec![],
                close: false,
            },
            PatternConfig {
                regex: r"thread-(?P<thread_id>\d+)".to_string(),
                release_matching_peer_keys: vec![],
                release_self_keys: vec![],
                close: false,
            },
        ];

        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        // Create fiber for thread-5
        let log1 = make_log("program1", "2025-12-04T10:00:00Z", "thread-5 doing stuff");
        let result1 = processor.process_log(&log1);
        let fiber1_id = result1.new_fibers[0].fiber_id;

        // New "START" log for thread-5 should release key from first fiber
        let log2 = make_log("program1", "2025-12-04T10:00:01Z", "thread-5 START");
        let result2 = processor.process_log(&log2);

        // Should create new fiber because the old one's key was released
        assert_eq!(result2.new_fibers.len(), 1);
        assert_ne!(result2.new_fibers[0].fiber_id, fiber1_id);
        assert_eq!(processor.open_fiber_count(), 2);
    }

    #[test]
    fn test_close_pattern() {
        let mut config = make_simple_fiber_type();
        // Insert END pattern first so it matches before the generic pattern
        config.sources.get_mut("program1").unwrap().patterns.insert(0, PatternConfig {
            regex: r"thread-(?P<thread_id>\d+) END".to_string(),
            release_matching_peer_keys: vec![],
            release_self_keys: vec![],
            close: true,
        });

        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        // Create fiber
        let log1 = make_log("program1", "2025-12-04T10:00:00Z", "thread-5 doing stuff");
        processor.process_log(&log1);
        assert_eq!(processor.open_fiber_count(), 1);

        // Close fiber with END pattern
        let log2 = make_log("program1", "2025-12-04T10:00:01Z", "thread-5 END");
        let result2 = processor.process_log(&log2);

        assert_eq!(result2.closed_fiber_ids.len(), 1);
        assert_eq!(processor.open_fiber_count(), 0);
    }

    #[test]
    fn test_fiber_merge() {
        let config = FiberTypeConfig {
            description: Some("Merge test".to_string()),
            temporal: ConfigTemporalConfig {
                max_gap: Some(Duration::from_secs(10)),
                gap_mode: GapMode::Session,
            },
            attributes: vec![
                AttributeConfig {
                    name: "key1".to_string(),
                    attr_type: AttributeType::String,
                    key: true,
                    derived: None,
                },
                AttributeConfig {
                    name: "key2".to_string(),
                    attr_type: AttributeType::String,
                    key: true,
                    derived: None,
                },
            ],
            sources: {
                let mut sources = HashMap::new();
                sources.insert(
                    "program1".to_string(),
                    FiberSourceConfig {
                        // More specific pattern (both keys) first
                        patterns: vec![
                            PatternConfig {
                                regex: r"K1=(?P<key1>\w+) K2=(?P<key2>\w+)".to_string(),
                                release_matching_peer_keys: vec![],
                                release_self_keys: vec![],
                                close: false,
                            },
                            PatternConfig {
                                regex: r"K1=(?P<key1>\w+)".to_string(),
                                release_matching_peer_keys: vec![],
                                release_self_keys: vec![],
                                close: false,
                            },
                            PatternConfig {
                                regex: r"K2=(?P<key2>\w+)".to_string(),
                                release_matching_peer_keys: vec![],
                                release_self_keys: vec![],
                                close: false,
                            },
                        ],
                    },
                );
                sources
            },
            is_source_fiber: false,
        };

        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        // Create fiber with key1=A
        let log1 = make_log("program1", "2025-12-04T10:00:00Z", "K1=A");
        processor.process_log(&log1);

        // Create fiber with key2=B
        let log2 = make_log("program1", "2025-12-04T10:00:01Z", "K2=B");
        processor.process_log(&log2);

        assert_eq!(processor.open_fiber_count(), 2);

        // Log that matches both keys -> merge
        let log3 = make_log("program1", "2025-12-04T10:00:02Z", "K1=A K2=B");
        let result3 = processor.process_log(&log3);

        assert_eq!(result3.merged_fiber_ids.len(), 1);
        assert_eq!(processor.open_fiber_count(), 1);
    }

    #[test]
    fn test_derived_attributes() {
        let config = FiberTypeConfig {
            description: Some("Derived test".to_string()),
            temporal: ConfigTemporalConfig {
                max_gap: Some(Duration::from_secs(10)),
                gap_mode: GapMode::Session,
            },
            attributes: vec![
                AttributeConfig {
                    name: "ip".to_string(),
                    attr_type: AttributeType::String,
                    key: false,
                    derived: None,
                },
                AttributeConfig {
                    name: "port".to_string(),
                    attr_type: AttributeType::String,
                    key: false,
                    derived: None,
                },
                AttributeConfig {
                    name: "endpoint".to_string(),
                    attr_type: AttributeType::String,
                    key: true,
                    derived: Some("${ip}:${port}".to_string()),
                },
            ],
            sources: {
                let mut sources = HashMap::new();
                sources.insert(
                    "program1".to_string(),
                    FiberSourceConfig {
                        patterns: vec![PatternConfig {
                            regex: r"(?P<ip>\d+\.\d+\.\d+\.\d+):(?P<port>\d+)".to_string(),
                            release_matching_peer_keys: vec![],
                            release_self_keys: vec![],
                            close: false,
                        }],
                    },
                );
                sources
            },
            is_source_fiber: false,
        };

        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        let log1 = make_log("program1", "2025-12-04T10:00:00Z", "connecting to 10.0.0.1:8080");
        let result1 = processor.process_log(&log1);

        assert_eq!(result1.new_fibers.len(), 1);

        // Second log with same derived key should join
        let log2 = make_log("program1", "2025-12-04T10:00:01Z", "data from 10.0.0.1:8080");
        let result2 = processor.process_log(&log2);

        assert_eq!(result2.new_fibers.len(), 0);
        assert_eq!(processor.open_fiber_count(), 1);
    }

    #[test]
    fn test_unmatched_source_ignored() {
        let config = make_simple_fiber_type();
        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        // Log from unknown source
        let log = make_log("unknown_source", "2025-12-04T10:00:00Z", "thread-5 doing stuff");
        let result = processor.process_log(&log);

        assert_eq!(result.new_fibers.len(), 0);
        assert_eq!(result.memberships.len(), 0);
        assert_eq!(processor.open_fiber_count(), 0);
    }

    #[test]
    fn test_flush_closes_all_fibers() {
        let config = make_simple_fiber_type();
        let compiled = CompiledFiberType::from_config("test", &config).unwrap();
        let mut processor = FiberTypeProcessor::new(compiled, 1);

        // Create multiple fibers
        processor.process_log(&make_log("program1", "2025-12-04T10:00:00Z", "thread-1 stuff"));
        processor.process_log(&make_log("program1", "2025-12-04T10:00:00Z", "thread-2 stuff"));
        processor.process_log(&make_log("program1", "2025-12-04T10:00:00Z", "thread-3 stuff"));

        assert_eq!(processor.open_fiber_count(), 3);

        let result = processor.flush();
        assert_eq!(result.closed_fiber_ids.len(), 3);
        assert_eq!(processor.open_fiber_count(), 0);
    }
}
