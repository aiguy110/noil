use similar::{ChangeTag, TextDiff};

/// Create a unified diff between two text strings
///
/// Returns a human-readable diff in unified diff format with +/- prefixes
pub fn create_diff(from: &str, to: &str) -> String {
    let diff = TextDiff::from_lines(from, to);
    let mut output = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        output.push_str(&format!("{} {}", sign, change));
    }

    output
}

/// Create a unified diff with context information
///
/// Returns a diff with file headers and line numbers for better readability
pub fn create_diff_with_context(from: &str, to: &str, from_label: &str, to_label: &str) -> String {
    let diff = TextDiff::from_lines(from, to);
    let mut output = String::new();

    // Add header
    output.push_str(&format!("--- {}\n", from_label));
    output.push_str(&format!("+++ {}\n", to_label));

    // Track line numbers
    let mut old_line = 1;
    let mut new_line = 1;
    let mut hunk_changes = Vec::new();
    let mut hunk_start_old = 1;
    let mut hunk_start_new = 1;

    for (idx, change) in diff.iter_all_changes().enumerate() {
        if idx == 0 {
            hunk_start_old = old_line;
            hunk_start_new = new_line;
        }

        match change.tag() {
            ChangeTag::Delete => {
                hunk_changes.push(format!("-{}", change));
                old_line += 1;
            }
            ChangeTag::Insert => {
                hunk_changes.push(format!("+{}", change));
                new_line += 1;
            }
            ChangeTag::Equal => {
                hunk_changes.push(format!(" {}", change));
                old_line += 1;
                new_line += 1;
            }
        }
    }

    // Output hunk header
    if !hunk_changes.is_empty() {
        let hunk_old_len = old_line - hunk_start_old;
        let hunk_new_len = new_line - hunk_start_new;
        output.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk_start_old, hunk_old_len, hunk_start_new, hunk_new_len
        ));

        // Output changes
        for change in hunk_changes {
            output.push_str(&change);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_diff_no_changes() {
        let text = "line1\nline2\nline3\n";
        let diff = create_diff(text, text);

        // All lines should be equal (prefix with space)
        assert!(diff.contains(" line1"));
        assert!(diff.contains(" line2"));
        assert!(diff.contains(" line3"));
        assert!(!diff.contains("+"));
        assert!(!diff.contains("-"));
    }

    #[test]
    fn test_create_diff_addition() {
        let from = "line1\nline2\n";
        let to = "line1\nline2\nline3\n";
        let diff = create_diff(from, to);

        assert!(diff.contains(" line1"));
        assert!(diff.contains(" line2"));
        assert!(diff.contains("+ line3"));
    }

    #[test]
    fn test_create_diff_deletion() {
        let from = "line1\nline2\nline3\n";
        let to = "line1\nline3\n";
        let diff = create_diff(from, to);

        assert!(diff.contains(" line1"));
        assert!(diff.contains("- line2"));
        assert!(diff.contains(" line3"));
    }

    #[test]
    fn test_create_diff_modification() {
        let from = "line1\nline2\nline3\n";
        let to = "line1\nmodified\nline3\n";
        let diff = create_diff(from, to);

        assert!(diff.contains(" line1"));
        assert!(diff.contains("- line2"));
        assert!(diff.contains("+ modified"));
        assert!(diff.contains(" line3"));
    }

    #[test]
    fn test_create_diff_with_context_header() {
        let from = "line1\nline2\n";
        let to = "line1\nline3\n";
        let diff = create_diff_with_context(from, to, "old.txt", "new.txt");

        assert!(diff.contains("--- old.txt"));
        assert!(diff.contains("+++ new.txt"));
        assert!(diff.contains("@@"));
    }

    #[test]
    fn test_empty_diff() {
        let diff = create_diff("", "");
        assert_eq!(diff, "");
    }

    #[test]
    fn test_diff_with_empty_from() {
        let to = "new line\n";
        let diff = create_diff("", to);
        assert!(diff.contains("+ new line"));
    }

    #[test]
    fn test_diff_with_empty_to() {
        let from = "old line\n";
        let diff = create_diff(from, "");
        assert!(diff.contains("- old line"));
    }
}
