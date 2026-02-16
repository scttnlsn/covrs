/// Auto-detection of coverage file formats.
///
/// Strategy:
///   1. Check file extension for strong hints
///   2. Peek at the first bytes of the file content
///   3. Fall back to CLI --format override (handled by caller)
use std::path::Path;

use crate::error::CovrsError;

/// Supported coverage formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Cobertura,
    // JaCoCo,    // TODO
    Lcov,
    // Gcov,      // TODO: JSON format only; old text format not yet supported
    // SimpleCov, // TODO
    // Istanbul,  // TODO
}

impl Format {
    pub fn as_str(&self) -> &'static str {
        match self {
            Format::Cobertura => "cobertura",
            Format::Lcov => "lcov",
        }
    }
}

impl std::str::FromStr for Format {
    type Err = CovrsError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cobertura" => Ok(Format::Cobertura),
            "lcov" => Ok(Format::Lcov),
            _ => Err(CovrsError::Parse(format!(
                "Unknown format: '{}'. Supported: cobertura, lcov",
                s
            ))),
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Detect the coverage format from filename and file content.
pub fn detect_format(path: &Path, content: &[u8]) -> Option<Format> {
    // 1. Try extension-based detection
    if let Some(fmt) = detect_by_extension(path) {
        return Some(fmt);
    }

    // 2. Content-based detection
    detect_by_content(content)
}

fn detect_by_extension(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "info" | "lcov" => Some(Format::Lcov),
        "xml" => None, // Could be Cobertura or JaCoCo — need content inspection
        _ => None,
    }
}

fn detect_by_content(content: &[u8]) -> Option<Format> {
    // We only need to look at the first few KB
    let head_len = content.len().min(4096);
    let head = String::from_utf8_lossy(&content[..head_len]);

    // LCOV: lines start with TN:, SF:, DA:, etc.
    // Check that lines actually start with these tags to avoid false positives
    // on files that merely contain these strings.
    let has_sf = head.lines().any(|l| l.starts_with("SF:"));
    let has_da_or_fn = head.lines().any(|l| l.starts_with("DA:") || l.starts_with("FN:"));
    if has_sf && has_da_or_fn {
        return Some(Format::Lcov);
    }

    // XML-based formats
    if head.contains("<?xml") || head.trim_start().starts_with('<') {
        // Cobertura: root element is <coverage> with specific attributes
        if head.contains("<coverage") {
            // JaCoCo also uses XML but has <report> as root element.
            // TODO: when JaCoCo parser is added, check for <report> here.
            return Some(Format::Cobertura);
        }
    }

    // TODO: JSON-based formats (Istanbul, SimpleCov, gcov JSON)
    // if head.starts_with('{') { ... }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_lcov_by_extension() {
        let path = Path::new("coverage.info");
        assert_eq!(detect_format(path, b""), Some(Format::Lcov));

        let path = Path::new("coverage.lcov");
        assert_eq!(detect_format(path, b""), Some(Format::Lcov));
    }

    #[test]
    fn test_detect_lcov_by_content() {
        let content = b"TN:test\nSF:/src/lib.rs\nDA:1,5\nend_of_record\n";
        let path = Path::new("coverage.txt");
        assert_eq!(detect_format(path, content), Some(Format::Lcov));
    }

    #[test]
    fn test_detect_cobertura_by_content() {
        let content = b"<?xml version=\"1.0\"?>\n<coverage version=\"1.0\">";
        let path = Path::new("coverage.xml");
        assert_eq!(detect_format(path, content), Some(Format::Cobertura));
    }

    #[test]
    fn test_detect_unknown() {
        let path = Path::new("random.dat");
        assert_eq!(detect_format(path, b"hello world"), None);
    }
}
