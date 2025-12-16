use crate::config::types::AttributeType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Value types for fiber attributes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttributeValue {
    String(String),
    Int(i64),
    Float(f64),
}

impl AttributeValue {
    /// Convert the attribute value to a string representation for key matching
    pub fn as_key_string(&self) -> String {
        match self {
            AttributeValue::String(s) => s.clone(),
            AttributeValue::Int(i) => i.to_string(),
            AttributeValue::Float(f) => f.to_string(),
        }
    }

    /// Parse a string value according to the attribute type
    pub fn from_str(s: &str, attr_type: AttributeType) -> Option<Self> {
        match attr_type {
            AttributeType::String => Some(AttributeValue::String(s.to_string())),
            AttributeType::Ip => Some(AttributeValue::String(normalize_ip(s)?)),
            AttributeType::Mac => Some(AttributeValue::String(normalize_mac(s)?)),
            AttributeType::Int => s.parse::<i64>().ok().map(AttributeValue::Int),
            AttributeType::Float => s.parse::<f64>().ok().map(AttributeValue::Float),
        }
    }
}

/// Normalize an IP address (remove leading zeros in octets)
fn normalize_ip(s: &str) -> Option<String> {
    // Handle IPv4
    if s.contains('.') {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() == 4 {
            let normalized: Option<Vec<String>> = parts
                .iter()
                .map(|p| p.parse::<u8>().ok().map(|n| n.to_string()))
                .collect();
            return normalized.map(|v| v.join("."));
        }
    }
    // For IPv6 or other formats, just return as-is for now
    Some(s.to_string())
}

/// Normalize a MAC address (lowercase with colons)
fn normalize_mac(s: &str) -> Option<String> {
    // Remove all separators and convert to lowercase
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_lowercase();

    if cleaned.len() != 12 {
        return None;
    }

    // Format as xx:xx:xx:xx:xx:xx
    let parts: Vec<String> = cleaned
        .as_bytes()
        .chunks(2)
        .map(|chunk| String::from_utf8(chunk.to_vec()).unwrap())
        .collect();

    Some(parts.join(":"))
}

/// An open (active) fiber that can receive new logs
#[derive(Debug, Clone)]
pub struct OpenFiber {
    /// Unique identifier for this fiber
    pub fiber_id: Uuid,
    /// Type of this fiber
    pub fiber_type: String,
    /// Current keys (key_name -> value as string)
    pub keys: HashMap<String, String>,
    /// All attributes (name -> value)
    pub attributes: HashMap<String, AttributeValue>,
    /// Timestamp of first log
    pub first_activity: DateTime<Utc>,
    /// Timestamp of most recent log
    pub last_activity: DateTime<Utc>,
    /// IDs of all logs belonging to this fiber
    pub log_ids: Vec<Uuid>,
}

impl OpenFiber {
    /// Create a new open fiber
    pub fn new(fiber_type: String, timestamp: DateTime<Utc>) -> Self {
        Self {
            fiber_id: Uuid::new_v4(),
            fiber_type,
            keys: HashMap::new(),
            attributes: HashMap::new(),
            first_activity: timestamp,
            last_activity: timestamp,
            log_ids: Vec::new(),
        }
    }

    /// Add a log to this fiber
    pub fn add_log(&mut self, log_id: Uuid, timestamp: DateTime<Utc>) {
        self.log_ids.push(log_id);
        self.last_activity = timestamp;
    }

    /// Add or update a key
    pub fn set_key(&mut self, name: String, value: String) {
        self.keys.insert(name, value);
    }

    /// Remove a key by name
    pub fn remove_key(&mut self, name: &str) -> Option<String> {
        self.keys.remove(name)
    }

    /// Add or update an attribute, returning the old value if different
    pub fn set_attribute(&mut self, name: String, value: AttributeValue) -> Option<AttributeValue> {
        let old_value = self.attributes.get(&name).cloned();
        self.attributes.insert(name, value.clone());

        // Return old value only if different (for conflict detection)
        match old_value {
            Some(old) if old != value => Some(old),
            _ => None,
        }
    }

    /// Get all keys as a vec of (name, value) pairs
    pub fn key_pairs(&self) -> Vec<(String, String)> {
        self.keys
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Merge another fiber into this one (this fiber survives)
    pub fn merge(&mut self, other: OpenFiber) -> Vec<(String, AttributeValue, AttributeValue)> {
        let mut conflicts = Vec::new();

        // Merge keys
        for (key_name, value) in other.keys {
            self.keys.insert(key_name, value);
        }

        // Merge attributes (latest wins, track conflicts)
        for (attr_name, other_value) in other.attributes {
            if let Some(self_value) = self.attributes.get(&attr_name) {
                if *self_value != other_value {
                    // Conflict: use the value from the fiber with later last_activity
                    if other.last_activity > self.last_activity {
                        conflicts.push((attr_name.clone(), self_value.clone(), other_value.clone()));
                        self.attributes.insert(attr_name, other_value);
                    } else {
                        conflicts.push((attr_name.clone(), other_value, self_value.clone()));
                    }
                }
            } else {
                self.attributes.insert(attr_name, other_value);
            }
        }

        // Merge log_ids
        self.log_ids.extend(other.log_ids);

        // Update timestamps
        if other.first_activity < self.first_activity {
            self.first_activity = other.first_activity;
        }
        if other.last_activity > self.last_activity {
            self.last_activity = other.last_activity;
        }

        conflicts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_ip() {
        assert_eq!(normalize_ip("192.168.001.001"), Some("192.168.1.1".to_string()));
        assert_eq!(normalize_ip("10.0.0.1"), Some("10.0.0.1".to_string()));
        assert_eq!(normalize_ip("255.255.255.255"), Some("255.255.255.255".to_string()));
    }

    #[test]
    fn test_normalize_mac() {
        assert_eq!(
            normalize_mac("AA-BB-CC-11-22-33"),
            Some("aa:bb:cc:11:22:33".to_string())
        );
        assert_eq!(
            normalize_mac("AA:BB:CC:11:22:33"),
            Some("aa:bb:cc:11:22:33".to_string())
        );
        assert_eq!(
            normalize_mac("aabbcc112233"),
            Some("aa:bb:cc:11:22:33".to_string())
        );
        assert_eq!(normalize_mac("invalid"), None);
    }

    #[test]
    fn test_attribute_value_from_str() {
        // String type
        let val = AttributeValue::from_str("hello", AttributeType::String);
        assert_eq!(val, Some(AttributeValue::String("hello".to_string())));

        // Int type
        let val = AttributeValue::from_str("42", AttributeType::Int);
        assert_eq!(val, Some(AttributeValue::Int(42)));

        // Float type
        let val = AttributeValue::from_str("3.14", AttributeType::Float);
        assert_eq!(val, Some(AttributeValue::Float(3.14)));

        // IP normalization
        let val = AttributeValue::from_str("192.168.001.001", AttributeType::Ip);
        assert_eq!(val, Some(AttributeValue::String("192.168.1.1".to_string())));

        // MAC normalization
        let val = AttributeValue::from_str("AA-BB-CC-11-22-33", AttributeType::Mac);
        assert_eq!(
            val,
            Some(AttributeValue::String("aa:bb:cc:11:22:33".to_string()))
        );
    }

    #[test]
    fn test_open_fiber_creation() {
        let ts: DateTime<Utc> = "2025-12-04T10:00:00Z".parse().unwrap();
        let fiber = OpenFiber::new("test_type".to_string(), ts);

        assert_eq!(fiber.fiber_type, "test_type");
        assert_eq!(fiber.first_activity, ts);
        assert_eq!(fiber.last_activity, ts);
        assert!(fiber.keys.is_empty());
        assert!(fiber.attributes.is_empty());
        assert!(fiber.log_ids.is_empty());
    }

    #[test]
    fn test_open_fiber_add_log() {
        let ts1: DateTime<Utc> = "2025-12-04T10:00:00Z".parse().unwrap();
        let ts2: DateTime<Utc> = "2025-12-04T10:00:05Z".parse().unwrap();

        let mut fiber = OpenFiber::new("test_type".to_string(), ts1);
        let log_id = Uuid::new_v4();
        fiber.add_log(log_id, ts2);

        assert_eq!(fiber.log_ids.len(), 1);
        assert_eq!(fiber.log_ids[0], log_id);
        assert_eq!(fiber.last_activity, ts2);
    }

    #[test]
    fn test_open_fiber_keys() {
        let ts: DateTime<Utc> = "2025-12-04T10:00:00Z".parse().unwrap();
        let mut fiber = OpenFiber::new("test_type".to_string(), ts);

        fiber.set_key("thread_id".to_string(), "5".to_string());
        assert_eq!(fiber.keys.get("thread_id"), Some(&"5".to_string()));

        fiber.set_key("mac".to_string(), "aa:bb:cc:11:22:33".to_string());
        assert_eq!(fiber.keys.len(), 2);

        let removed = fiber.remove_key("thread_id");
        assert_eq!(removed, Some("5".to_string()));
        assert_eq!(fiber.keys.len(), 1);
    }

    #[test]
    fn test_open_fiber_attribute_conflict() {
        let ts: DateTime<Utc> = "2025-12-04T10:00:00Z".parse().unwrap();
        let mut fiber = OpenFiber::new("test_type".to_string(), ts);

        // Set initial value
        let old = fiber.set_attribute("ip".to_string(), AttributeValue::String("10.0.0.1".to_string()));
        assert!(old.is_none());

        // Set same value again
        let old = fiber.set_attribute("ip".to_string(), AttributeValue::String("10.0.0.1".to_string()));
        assert!(old.is_none());

        // Set different value
        let old = fiber.set_attribute("ip".to_string(), AttributeValue::String("10.0.0.2".to_string()));
        assert_eq!(old, Some(AttributeValue::String("10.0.0.1".to_string())));
    }

    #[test]
    fn test_fiber_merge() {
        let ts1: DateTime<Utc> = "2025-12-04T10:00:00Z".parse().unwrap();
        let ts2: DateTime<Utc> = "2025-12-04T10:00:05Z".parse().unwrap();

        let mut fiber1 = OpenFiber::new("test_type".to_string(), ts1);
        fiber1.set_key("key1".to_string(), "value1".to_string());
        fiber1.set_attribute("attr1".to_string(), AttributeValue::String("a".to_string()));
        fiber1.add_log(Uuid::new_v4(), ts1);

        let mut fiber2 = OpenFiber::new("test_type".to_string(), ts2);
        fiber2.set_key("key2".to_string(), "value2".to_string());
        fiber2.set_attribute("attr2".to_string(), AttributeValue::String("b".to_string()));
        fiber2.add_log(Uuid::new_v4(), ts2);

        // Merge fiber2 into fiber1
        let conflicts = fiber1.merge(fiber2);

        assert!(conflicts.is_empty());
        assert_eq!(fiber1.keys.len(), 2);
        assert_eq!(fiber1.attributes.len(), 2);
        assert_eq!(fiber1.log_ids.len(), 2);
        assert_eq!(fiber1.first_activity, ts1);
        assert_eq!(fiber1.last_activity, ts2);
    }

    #[test]
    fn test_fiber_merge_with_conflicts() {
        let ts1: DateTime<Utc> = "2025-12-04T10:00:00Z".parse().unwrap();
        let ts2: DateTime<Utc> = "2025-12-04T10:00:05Z".parse().unwrap();

        let mut fiber1 = OpenFiber::new("test_type".to_string(), ts1);
        fiber1.set_attribute("shared".to_string(), AttributeValue::String("old".to_string()));

        let mut fiber2 = OpenFiber::new("test_type".to_string(), ts2);
        fiber2.set_attribute("shared".to_string(), AttributeValue::String("new".to_string()));

        let conflicts = fiber1.merge(fiber2);

        assert_eq!(conflicts.len(), 1);
        // fiber2 has later timestamp, so its value wins
        assert_eq!(
            fiber1.attributes.get("shared"),
            Some(&AttributeValue::String("new".to_string()))
        );
    }
}
