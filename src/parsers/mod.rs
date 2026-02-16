pub mod cobertura;
pub mod lcov;

use crate::error::Result;
use crate::model::CoverageData;

/// Every format parser implements this trait.
pub trait Parser {
    /// Parse the input bytes into our uniform coverage model.
    fn parse(&self, input: &[u8]) -> Result<CoverageData>;
}
