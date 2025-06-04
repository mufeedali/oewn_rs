//! Error types and handling for the OEWN library.
//!
//! This module defines the main error type `OewnError` and a convenience
//! `Result` type alias for use throughout the library.

use thiserror::Error;

/// Custom Result type for this crate.
pub type Result<T> = std::result::Result<T, OewnError>;

/// Comprehensive error type representing all possible errors in the oewn_rs library.
///
/// This enum uses the `thiserror` crate to provide detailed error messages
/// and automatic conversions from common error types.
#[derive(Error, Debug)]
pub enum OewnError {
    /// I/O operations failed (file read/write, network, etc.)
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// HTTP request failed
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    /// ZIP archive extraction failed
    #[error("ZIP archive error: {0}")]
    Zip(#[from] zip::result::ZipError),

    /// XML parsing failed
    #[error("XML parsing error: {0}")]
    XmlParse(quick_xml::DeError),

    /// Could not determine the user's data directory
    #[error("Data directory not found or could not be determined")]
    DataDirNotFound,

    /// A required data file was not found
    #[error("Required data file not found: {0}")]
    DataFileNotFound(String),

    /// Generic parsing error for non-XML data
    #[error("Failed to parse data: {0}")]
    ParseError(String),

    /// Synset lookup failed
    #[error("Synset not found: {0}")]
    SynsetNotFound(String),

    /// Lexical entry lookup failed
    #[error("Lexical entry not found: {0}")]
    LexicalEntryNotFound(String),

    /// WordNet data has not been loaded
    #[error("WordNet data not loaded")]
    NotLoaded,

    /// Invalid argument provided to a function
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    /// Unexpected internal error
    #[error("Internal error: {0}")]
    Internal(String),

    /// SQLite database operation failed
    #[error("Database error: {0}")]
    DbError(#[from] rusqlite::Error),

    /// Async task join failed
    #[error("Tokio join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}

/// Manual implementation for quick_xml::DeError conversion.
impl From<quick_xml::DeError> for OewnError {
    fn from(err: quick_xml::DeError) -> Self {
        OewnError::XmlParse(err)
    }
}
