use thiserror::Error;

/// Custom Result type for this crate.
pub type Result<T> = std::result::Result<T, OewnError>;

/// Enum representing all possible errors in the oewn_rs library.
#[derive(Error, Debug)]
pub enum OewnError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("ZIP archive error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("XML parsing error: {0}")]
    XmlParse(quick_xml::DeError),

    #[error("Data directory not found or could not be determined")]
    DataDirNotFound,

    #[error("Required data file not found: {0}")]
    DataFileNotFound(String),

    #[error("Failed to parse data: {0}")]
    ParseError(String), // Generic parsing error for non-XML/cache issues

    #[error("Synset not found: {0}")]
    SynsetNotFound(String), // Use String for Synset ID

    #[error("Lexical entry not found: {0}")]
    LexicalEntryNotFound(String),

    #[error("WordNet data not loaded")]
    NotLoaded,

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Internal error: {0}")]
    Internal(String), // For unexpected situations

    #[error("Database error: {0}")]
    DbError(#[from] rusqlite::Error),

    #[error("Tokio join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}

// Implement From<quick_xml::DeError> manually
impl From<quick_xml::DeError> for OewnError {
    fn from(err: quick_xml::DeError) -> Self {
        OewnError::XmlParse(err)
    }
}
