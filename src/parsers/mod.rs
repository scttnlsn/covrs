pub mod cobertura;
pub mod lcov;

use std::path::Path;

use anyhow::Result;

use crate::model::CoverageData;

/// Parser for a specific coverage format.
pub trait CoverageParser {
    /// The format this parser handles.
    fn format(&self) -> Format;

    /// Whether this parser can handle the given file, based on its path and
    /// content. Implementations should be cheap — only inspect the extension
    /// and/or the first few KB of content.
    fn can_parse(&self, path: &Path, content: &[u8]) -> bool;

    /// Parse coverage data from raw bytes.
    fn parse(&self, input: &[u8]) -> Result<CoverageData>;
}

/// Supported coverage formats, used for the `--format` CLI override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Cobertura,
    Lcov,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Format::Cobertura => f.write_str("cobertura"),
            Format::Lcov => f.write_str("lcov"),
        }
    }
}

impl std::str::FromStr for Format {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cobertura" => Ok(Format::Cobertura),
            "lcov" => Ok(Format::Lcov),
            _ => Err(anyhow::anyhow!(
                "Unknown format: '{s}'. Supported: cobertura, lcov"
            )),
        }
    }
}

/// All registered parsers, in detection priority order.
///
/// LCOV is checked first because its content markers are very specific
/// (lines starting with `SF:`, `DA:`, etc.) so false positives are unlikely.
/// Cobertura (XML-based) is checked second since `<coverage` is a broader match.
pub fn all() -> Vec<Box<dyn CoverageParser>> {
    vec![
        Box::new(lcov::LcovParser),
        Box::new(cobertura::CoberturaParser),
    ]
}

/// Detect the format and return the matching parser, or `None`.
pub fn detect(path: &Path, content: &[u8]) -> Option<Box<dyn CoverageParser>> {
    all().into_iter().find(|p| p.can_parse(path, content))
}

/// Get the appropriate parser for an explicit format name.
pub fn for_format(format: Format) -> Box<dyn CoverageParser> {
    match format {
        Format::Cobertura => Box::new(cobertura::CoberturaParser),
        Format::Lcov => Box::new(lcov::LcovParser),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_lcov_by_extension() {
        let parser = detect(Path::new("coverage.info"), b"").unwrap();
        assert_eq!(parser.format(), Format::Lcov);

        let parser = detect(Path::new("coverage.lcov"), b"").unwrap();
        assert_eq!(parser.format(), Format::Lcov);
    }

    #[test]
    fn test_detect_lcov_by_content() {
        let content = b"TN:test\nSF:/src/lib.rs\nDA:1,5\nend_of_record\n";
        let parser = detect(Path::new("coverage.txt"), content).unwrap();
        assert_eq!(parser.format(), Format::Lcov);
    }

    #[test]
    fn test_detect_cobertura_by_content() {
        let content = b"<?xml version=\"1.0\"?>\n<coverage version=\"1.0\">";
        let parser = detect(Path::new("coverage.xml"), content).unwrap();
        assert_eq!(parser.format(), Format::Cobertura);
    }

    #[test]
    fn test_detect_unknown() {
        assert!(detect(Path::new("random.dat"), b"hello world").is_none());
    }
}
