/// Parse a unified diff to extract which lines were added in each file.
/// This is used for computing "diff coverage" — what percentage of newly
/// added/modified lines are covered by tests.
///
/// Also provides a [`DiffSource`] trait that abstracts over different
/// ways to obtain a diff (stdin, git, GitHub API).
use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};

use crate::github;

// ---------------------------------------------------------------------------
// Diff sources
// ---------------------------------------------------------------------------

/// A source for obtaining a unified diff.
pub trait DiffSource {
    /// Fetch the diff text.
    fn fetch_diff(&self) -> Result<String>;

    /// Get the commit SHA, if available.
    fn sha(&self) -> Option<&str> {
        None
    }
}

/// Diff from stdin.
pub struct StdinDiff;

impl DiffSource for StdinDiff {
    fn fetch_diff(&self) -> Result<String> {
        std::io::read_to_string(std::io::stdin()).context("Failed to read diff from stdin")
    }
}

/// Diff from a git command (e.g., `git diff HEAD~1`).
pub struct GitDiff {
    /// Arguments to pass to `git diff`.
    pub args: String,
}

impl DiffSource for GitDiff {
    fn fetch_diff(&self) -> Result<String> {
        let diff_args: Vec<&str> = self.args.split_whitespace().collect();
        let output = Command::new("git")
            .arg("diff")
            .args(&diff_args)
            .output()
            .context("Failed to run git diff")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git diff failed: {stderr}");
        }

        String::from_utf8(output.stdout).context("git diff output not valid UTF-8")
    }
}

/// Diff from a GitHub pull request.
pub struct GitHubDiff {
    /// The resolved GitHub context.
    pub context: github::Context,
}

impl GitHubDiff {
    /// Create from environment variables.
    pub fn from_env() -> Result<Self> {
        let context = github::Context::from_env()?;
        Ok(Self { context })
    }
}

impl DiffSource for GitHubDiff {
    fn fetch_diff(&self) -> Result<String> {
        self.context.fetch_diff()
    }

    fn sha(&self) -> Option<&str> {
        self.context.sha.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Diff parsing
// ---------------------------------------------------------------------------

/// Prepend a path prefix to all file paths in a diff result.
pub fn apply_path_prefix(
    diff_lines: HashMap<String, Vec<u32>>,
    prefix: &str,
) -> HashMap<String, Vec<u32>> {
    let prefix = prefix.trim_end_matches('/');
    diff_lines
        .into_iter()
        .map(|(path, lines)| (format!("{prefix}/{path}"), lines))
        .collect()
}

/// Parse a unified diff (e.g., `git diff`) and return a map of
/// file path -> list of added line numbers (in the new file).
pub fn parse_diff(diff_text: &str) -> HashMap<String, Vec<u32>> {
    let mut result: HashMap<String, Vec<u32>> = HashMap::new();
    let mut current_file: Option<String> = None;
    let mut new_line_number: u32 = 0;

    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            if rest == "/dev/null" {
                current_file = None; // File was deleted
            } else {
                // Strip common VCS prefixes: "b/" (default git), "a/" (some tools).
                // Also handles --no-prefix diffs where no prefix is present.
                let path = rest
                    .strip_prefix("b/")
                    .or_else(|| rest.strip_prefix("a/"))
                    .unwrap_or(rest);
                current_file = Some(path.to_string());
            }
        } else if line.starts_with("@@ ") {
            // Hunk header: @@ -old_start[,old_count] +new_start[,new_count] @@
            if let Some(new_range) = parse_hunk_header(line) {
                new_line_number = new_range;
            }
        } else if let Some(ref file) = current_file {
            if line.starts_with('\\') {
                // "\ No newline at end of file" — diff metadata, not a real line
            } else if line.starts_with('+') && !line.starts_with("+++") {
                // Added line
                result
                    .entry(file.clone())
                    .or_default()
                    .push(new_line_number);
                new_line_number += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                // Deleted line — doesn't advance new line counter
            } else {
                // Context line or other
                new_line_number += 1;
            }
        }
    }

    result
}

/// Parse "new" start line from a hunk header like "@@ -10,5 +20,8 @@"
fn parse_hunk_header(line: &str) -> Option<u32> {
    // Find the +N part
    let after_at = line.strip_prefix("@@ ")?;
    let parts: Vec<&str> = after_at.split(' ').collect();
    // parts[0] = "-old_start,old_count"
    // parts[1] = "+new_start,new_count" or "+new_start"
    if parts.len() < 2 {
        return None;
    }
    let new_part = parts[1].strip_prefix('+')?;
    let start_str = new_part.split(',').next()?;
    start_str.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Diff parsing tests -------------------------------------------------

    #[test]
    fn test_parse_hunk_header() {
        assert_eq!(parse_hunk_header("@@ -10,5 +20,8 @@"), Some(20));
        assert_eq!(parse_hunk_header("@@ -0,0 +1,3 @@"), Some(1));
        assert_eq!(parse_hunk_header("@@ -5 +5 @@"), Some(5));
    }

    #[test]
    fn test_parse_diff() {
        let diff = include_str!("../tests/fixtures/diffs/modified_file.diff");
        let result = parse_diff(diff);
        assert_eq!(result.len(), 1);
        let lines = result.get("src/main.rs").unwrap();
        // Line 11 (y=2), line 12 (z=x+y), line 14 (println z)
        assert_eq!(lines, &[11, 12, 14]);
    }

    #[test]
    fn test_parse_diff_new_file() {
        let diff = include_str!("../tests/fixtures/diffs/new_file.diff");
        let result = parse_diff(diff);
        let lines = result.get("src/new.rs").unwrap();
        assert_eq!(lines, &[1, 2, 3]);
    }

    #[test]
    fn test_parse_diff_deleted_file() {
        let diff = include_str!("../tests/fixtures/diffs/deleted_file.diff");
        let result = parse_diff(diff);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_diff_no_newline_at_eof() {
        let diff = include_str!("../tests/fixtures/diffs/no_newline_at_eof.diff");
        let result = parse_diff(diff);
        assert_eq!(result.len(), 1);
        let lines = result.get("src/lib.rs").unwrap();
        // The "\ No newline at end of file" marker must not shift line numbers.
        // Added lines are: line 2 (println world), line 3 (closing brace).
        assert_eq!(lines, &[2, 3]);
    }

    #[test]
    fn test_parse_diff_multiple_files() {
        let diff = include_str!("../tests/fixtures/diffs/multiple_files.diff");
        let result = parse_diff(diff);
        assert_eq!(result.len(), 2);
        assert_eq!(result.get("a.rs").unwrap(), &[2]);
        assert_eq!(result.get("b.rs").unwrap(), &[2]);
    }
}
