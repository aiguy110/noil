use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;

/// Compute a deterministic version hash for a configuration based on its content.
///
/// The version is computed as a SHA-256 hash of the normalized YAML content, ensuring that:
/// - The same config produces the same version hash
/// - Different configs produce different version hashes
/// - Formatting differences (whitespace, etc.) don't affect the hash
///
/// This version is stored with all logs and fiber memberships to enable:
/// - Processing in-flight logs with original config semantics
/// - Reprocessing historical logs with new rules
/// - Comparing results across config versions
pub fn compute_config_hash(yaml_content: &str) -> String {
    let normalized = normalize_yaml(yaml_content);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute config hash from a file path
pub fn compute_config_hash_from_file(config_path: &Path) -> Result<String, io::Error> {
    let content = std::fs::read_to_string(config_path)?;
    Ok(compute_config_hash(&content))
}

/// Normalize YAML content by parsing and re-serializing to canonical form.
/// This ensures that formatting differences don't affect the hash.
fn normalize_yaml(content: &str) -> String {
    match serde_yaml::from_str::<serde_yaml::Value>(content) {
        Ok(value) => serde_yaml::to_string(&value).unwrap_or_else(|_| content.to_string()),
        Err(_) => content.to_string(),
    }
}

/// Legacy function for backward compatibility - computes numeric version
/// This is kept to avoid breaking existing code, but new code should use compute_config_hash
#[deprecated(note = "Use compute_config_hash_from_file instead")]
pub fn compute_config_version(config_path: &Path) -> Result<u64, io::Error> {
    let content = std::fs::read_to_string(config_path)?;
    let hash = compute_config_hash(&content);
    // Convert first 8 bytes of hash to u64 for backward compatibility
    let bytes: [u8; 8] = hash.as_bytes()[0..8]
        .try_into()
        .unwrap_or([0; 8]);
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_same_content_produces_same_hash() {
        let content = "sources:\n  test:\n    type: file\n";

        let hash1 = compute_config_hash(content);
        let hash2 = compute_config_hash(content);

        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 produces 64 hex characters
    }

    #[test]
    fn test_different_content_produces_different_hash() {
        let content1 = "sources:\n  test1:\n    type: file\n";
        let content2 = "sources:\n  test2:\n    type: file\n";

        let hash1 = compute_config_hash(content1);
        let hash2 = compute_config_hash(content2);

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_normalized_content_produces_same_hash() {
        // Same semantic content with different formatting
        let content1 = "sources:\n  test:\n    type: file\n    path: /var/log/test.log";
        let content2 = "sources:\n  test:\n    path: /var/log/test.log\n    type: file";

        let hash1 = compute_config_hash(content1);
        let hash2 = compute_config_hash(content2);

        // After normalization, hashes should be the same
        // (YAML maps are unordered, so serde_yaml normalization handles this)
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_whitespace_differences_normalized() {
        let content1 = "sources:\n  test:\n    type: file";
        let content2 = "sources:\n  test:\n    type:  file"; // Extra space

        let hash1 = compute_config_hash(content1);
        let hash2 = compute_config_hash(content2);

        // Should normalize to same hash
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_hash_from_file() {
        let content = "sources:\n  test:\n    type: file\n";

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();

        let hash = compute_config_hash_from_file(file.path()).unwrap();
        let expected_hash = compute_config_hash(content);

        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_nonexistent_file_returns_error() {
        let result = compute_config_hash_from_file(Path::new("/nonexistent/path/config.yml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_yaml_uses_raw_content() {
        let invalid_yaml = "this is not valid: yaml: content: [[[";
        let hash = compute_config_hash(invalid_yaml);

        // Should still produce a hash (using raw content)
        assert_eq!(hash.len(), 64);
    }
}
