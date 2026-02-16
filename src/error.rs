use thiserror::Error;

#[derive(Error, Debug)]
pub enum CovrsError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("XML parse error at position {position}: {source}")]
    Xml {
        source: quick_xml::Error,
        position: usize,
    },

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Unknown coverage format")]
    UnknownFormat,

    #[error("Report not found: {0}")]
    ReportNotFound(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CovrsError>;
