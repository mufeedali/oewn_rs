//! Defines structures and types for progress reporting.

/// Represents a snapshot of the progress during a long-running operation.
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    /// A description of the current stage (e.g., "Pass 1/3: Inserting Lexicons").
    pub stage_description: String,
    /// Number of items processed in the current stage.
    pub current_item: u64,
    /// Total number of items expected in the current stage (if calculable).
    pub total_items: Option<u64>,
    /// An optional message providing more context (e.g., "Processing lexicon: XYZ").
    pub message: Option<String>,
}

/// Type alias for the progress callback function.
///
/// The callback receives a `ProgressUpdate` and should return `true` to continue the operation,
/// or `false` to request cancellation (cancellation support is not yet implemented in the caller).
///
/// The callback must be `Send` and `Sync` to be safely passed between threads if needed,
/// and `FnMut` allows it to modify its captured state (e.g., update a UI element).
pub type ProgressCallback = Box<dyn FnMut(ProgressUpdate) -> bool + Send + Sync>;

impl ProgressUpdate {
    /// Creates a new progress update for the start of a stage.
    pub fn new_stage(description: String, total_items: Option<u64>) -> Self {
        ProgressUpdate {
            stage_description: description,
            current_item: 0,
            total_items,
            message: None,
        }
    }
}