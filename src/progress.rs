//! Progress reporting structures and types for long-running operations.
//!
//! This module provides a non-blocking progress reporting system using async channels
//! that allows operations to report their status to calling code, typically for UI updates.
//!
//! ## Usage
//!
//! ```rust
//! use oewn_rs::progress::{ProgressUpdate, create_progress_channel};
//! use tokio::sync::mpsc;
//!
//! # tokio_test::block_on(async {
//! // Create a progress channel
//! let (progress_tx, mut progress_rx) = create_progress_channel(100);
//!
//! // Spawn a task to handle progress updates
//! tokio::spawn(async move {
//!     while let Some(update) = progress_rx.recv().await {
//!         println!("Stage: {}, Progress: {}/{}",
//!             update.stage_description,
//!             update.current_item,
//!             update.total_items.unwrap_or(0)
//!         );
//!     }
//! });
//!
//! // Pass progress_tx to operations that need to report progress
//! # });
//! ```

use tokio::sync::mpsc;

/// Represents a snapshot of progress during a long-running operation.
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    /// A description of the current stage
    pub stage_description: String,
    /// Number of items processed in the current stage.
    pub current_item: u64,
    /// Total number of items expected in the current stage
    pub total_items: Option<u64>,
    /// An optional message providing more context
    pub message: Option<String>,
}

/// Type alias for a non-blocking progress reporter using async channels.
pub type ProgressReporter = mpsc::Sender<ProgressUpdate>;

/// Type alias for the progress callback function (kept for backwards compatibility).
pub type ProgressCallback = Box<dyn FnMut(ProgressUpdate) -> bool + Send + Sync>;

/// Creates a progress channel for non-blocking progress reporting.
pub fn create_progress_channel(
    buffer_size: usize,
) -> (ProgressReporter, mpsc::Receiver<ProgressUpdate>) {
    mpsc::channel(buffer_size)
}

/// Helper function to send a progress update without blocking.
pub fn report_progress_non_blocking(reporter: &ProgressReporter, update: ProgressUpdate) {
    let _ = reporter.try_send(update);
}

/// Helper function to send a progress update with async waiting.
pub async fn report_progress_async(reporter: &ProgressReporter, update: ProgressUpdate) {
    let _ = reporter.send(update).await;
}

impl ProgressUpdate {
    /// Creates a new progress update.
    pub fn new(
        description: String,
        current: u64,
        total: Option<u64>,
        message: Option<String>,
    ) -> Self {
        ProgressUpdate {
            stage_description: description,
            current_item: current,
            total_items: total,
            message,
        }
    }
}
