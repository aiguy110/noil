use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::Path;

/// Compute a deterministic version ID for a configuration file based on its content.
///
/// The version is computed as a hash of the file's contents, ensuring that:
/// - The same config file produces the same version
/// - Different config files produce different versions
///
/// This version is stored with all logs and fiber memberships to enable:
/// - Processing in-flight logs with original config semantics
/// - Reprocessing historical logs with new rules
/// - Comparing results across config versions
pub fn compute_config_version(config_path: &Path) -> Result<u64, io::Error> {
    let content = std::fs::read_to_string(config_path)?;
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_same_content_produces_same_version() {
        let content = "sources:\n  test:\n    type: file\n";

        let mut file1 = NamedTempFile::new().unwrap();
        file1.write_all(content.as_bytes()).unwrap();
        file1.flush().unwrap();

        let mut file2 = NamedTempFile::new().unwrap();
        file2.write_all(content.as_bytes()).unwrap();
        file2.flush().unwrap();

        let version1 = compute_config_version(file1.path()).unwrap();
        let version2 = compute_config_version(file2.path()).unwrap();

        assert_eq!(version1, version2);
    }

    #[test]
    fn test_different_content_produces_different_version() {
        let content1 = "sources:\n  test1:\n    type: file\n";
        let content2 = "sources:\n  test2:\n    type: file\n";

        let mut file1 = NamedTempFile::new().unwrap();
        file1.write_all(content1.as_bytes()).unwrap();
        file1.flush().unwrap();

        let mut file2 = NamedTempFile::new().unwrap();
        file2.write_all(content2.as_bytes()).unwrap();
        file2.flush().unwrap();

        let version1 = compute_config_version(file1.path()).unwrap();
        let version2 = compute_config_version(file2.path()).unwrap();

        assert_ne!(version1, version2);
    }

    #[test]
    fn test_nonexistent_file_returns_error() {
        let result = compute_config_version(Path::new("/nonexistent/path/config.yml"));
        assert!(result.is_err());
    }
}
