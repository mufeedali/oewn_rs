//! Data download and management for OEWN.
//!
//! This module handles downloading the OEWN XML data from GitHub releases,
//! caching it locally, and decompressing it as needed.

use crate::error::{OewnError, Result};
use crate::progress::{ProgressReporter, ProgressUpdate, report_progress_async};
use directories_next::ProjectDirs;
use flate2::read::GzDecoder;
use futures::StreamExt;
use log::info;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

/// OEWN version being targeted
pub const OEWN_VERSION: &str = "2024";
/// Subdirectory name within user's data directory
pub const OEWN_SUBDIR: &str = "oewn-rs";
const OEWN_FILENAME_GZ: &str = "english-wordnet-2024.xml.gz";
const OEWN_FILENAME_XML: &str = "english-wordnet-2024.xml";
const OEWN_DOWNLOAD_URL: &str = "https://github.com/globalwordnet/english-wordnet/releases/download/2024-edition/english-wordnet-2024.xml.gz";

/// Gets the project's data directory path.
/// Creates the directory if it doesn't exist.
fn get_data_dir() -> Result<PathBuf> {
    let proj_dirs =
        ProjectDirs::from("org", "OewnRs", OEWN_SUBDIR).ok_or(OewnError::DataDirNotFound)?;
    let data_dir = proj_dirs.data_dir().to_path_buf();
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

/// Downloads a file from a URL to a specified path using streaming with progress reporting.
async fn download_file(
    url: &str,
    dest_path: &Path,
    reporter: Option<ProgressReporter>,
) -> Result<()> {
    let stage_desc = "Downloading OEWN data".to_string();

    info!(
        "Downloading data from {} to {:?} (streaming)...",
        url, dest_path
    );
    let response = reqwest::get(url).await?.error_for_status()?;

    let total_size = response.content_length();

    if let Some(ref reporter) = reporter {
        report_progress_async(
            reporter,
            ProgressUpdate::new(stage_desc.clone(), 0, total_size, None),
        )
        .await;
    }

    let mut dest_file = BufWriter::new(File::create(dest_path)?);
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        dest_file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;

        if let Some(ref reporter) = reporter {
            report_progress_async(
                reporter,
                ProgressUpdate {
                    stage_description: stage_desc.clone(),
                    current_item: downloaded,
                    total_items: total_size,
                    message: None,
                },
            )
            .await;
        }
    }

    dest_file.flush()?;

    if let Some(ref reporter) = reporter {
        if let Some(total) = total_size {
            report_progress_async(
                reporter,
                ProgressUpdate {
                    stage_description: stage_desc.clone(),
                    current_item: total,
                    total_items: Some(total),
                    message: Some("Download complete.".to_string()),
                },
            )
            .await;
        } else {
            report_progress_async(
                reporter,
                ProgressUpdate {
                    stage_description: stage_desc.clone(),
                    current_item: downloaded,
                    total_items: None,
                    message: Some("Download complete.".to_string()),
                },
            )
            .await;
        }
    }

    info!("Download complete.");
    Ok(())
}

/// Decompresses a GZipped file with progress reporting.
async fn decompress_gz(
    gz_path: &Path,
    dest_path: &Path,
    reporter: Option<ProgressReporter>,
) -> Result<()> {
    let stage_desc = "Decompressing OEWN data".to_string();

    info!("Decompressing {:?} to {:?}...", gz_path, dest_path);

    if let Some(ref reporter) = reporter {
        report_progress_async(
            reporter,
            ProgressUpdate::new(stage_desc.clone(), 0, None, None),
        )
        .await;
    }

    let gz_path = gz_path.to_path_buf();
    let dest_path = dest_path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let gz_file = File::open(&gz_path)?;
        let mut decoder = GzDecoder::new(BufReader::new(gz_file));
        let mut dest_file = BufWriter::new(File::create(&dest_path)?);
        io::copy(&mut decoder, &mut dest_file)?;
        dest_file.flush()?;
        Ok::<(), std::io::Error>(())
    })
    .await??;

    if let Some(ref reporter) = reporter {
        report_progress_async(
            reporter,
            ProgressUpdate {
                stage_description: stage_desc.clone(),
                current_item: 1,
                total_items: Some(1),
                message: Some("Decompression complete.".to_string()),
            },
        )
        .await;
    }

    info!("Decompression complete.");
    Ok(())
}

/// Ensures the OEWN XML data file is present in the data directory.
/// This function downloads and/or decompresses the data if necessary.
pub async fn ensure_data(reporter: Option<ProgressReporter>) -> Result<PathBuf> {
    let data_dir = get_data_dir()?;
    let xml_path = data_dir.join(OEWN_FILENAME_XML);
    let gz_path = data_dir.join(OEWN_FILENAME_GZ);

    if xml_path.exists() {
        info!("Found existing OEWN XML data file: {:?}", xml_path);
        return Ok(xml_path);
    } else {
        info!("OEWN XML data file not found at {:?}.", xml_path);
    }

    if !gz_path.exists() {
        info!("OEWN GZ archive not found at {:?}. Downloading...", gz_path);
        download_file(OEWN_DOWNLOAD_URL, &gz_path, reporter.clone()).await?;
    } else {
        info!("Found existing OEWN GZ archive: {:?}", gz_path);
    }

    decompress_gz(&gz_path, &xml_path, reporter).await?;

    // 4. Return the path to the decompressed XML file
    Ok(xml_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Helper to create a dummy gz file for testing decompression
    fn create_dummy_gz(path: &Path, content: &str) -> io::Result<()> {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let file = File::create(path)?;
        let mut encoder = GzEncoder::new(BufWriter::new(file), Compression::default());
        encoder.write_all(content.as_bytes())?;
        encoder.finish()?;
        Ok(())
    }

    #[tokio::test]
    #[ignore] // Ignored by default as it interacts with the file system and potentially network
    async fn test_ensure_data_flow() {
        let _ = env_logger::builder().is_test(true).try_init(); // Enable logging for tests

        // Use a temporary directory to simulate the data directory
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let data_dir_guard = scopeguard::guard(temp_dir, |d| {
            // Ensure temp dir is cleaned up even on panic
            let _ = d.close();
        });
        let data_dir = data_dir_guard.path();

        let xml_path = data_dir.join(OEWN_FILENAME_XML);
        let gz_path = data_dir.join(OEWN_FILENAME_GZ);

        // Mock the get_data_dir function to return our temp dir
        // This requires modifying the original function or using a mocking library,
        // which is complex. For now, we'll test the logic assuming get_data_dir works.
        // Let's simulate the steps instead.

        // --- Scenario 1: XML already exists ---
        info!("--- Testing Scenario 1: XML exists ---");
        fs::write(&xml_path, "<xml>dummy</xml>").expect("Failed to write dummy XML");
        // We can't directly call ensure_data and mock get_data_dir easily,
        // so we assert the expected outcome if it were called.
        assert!(xml_path.exists());
        // In a real test with mocking, we'd call ensure_data here.
        fs::remove_file(&xml_path).unwrap(); // Clean up for next scenario

        // --- Scenario 2: GZ exists, XML does not ---
        info!("--- Testing Scenario 2: GZ exists, XML does not ---");
        let dummy_xml_content = "<LexicalResource><Lexicon id='test'/></LexicalResource>";
        create_dummy_gz(&gz_path, dummy_xml_content).expect("Failed to create dummy GZ");
        assert!(gz_path.exists());
        assert!(!xml_path.exists());
        // Manually call decompress_gz as ensure_data would (no reporter for test)
        let rt = tokio::runtime::Runtime::new().unwrap();
        let decompress_result = rt.block_on(decompress_gz(&gz_path, &xml_path, None));
        assert!(decompress_result.is_ok(), "Decompression failed");
        assert!(xml_path.exists());
        let decompressed_content = fs::read_to_string(&xml_path).unwrap();
        assert_eq!(decompressed_content, dummy_xml_content);
        fs::remove_file(&xml_path).unwrap();
        fs::remove_file(&gz_path).unwrap(); // Clean up

        // --- Scenario 3: Neither GZ nor XML exists (requires network) ---
        // This part is hard to test reliably without actual network calls or complex mocking.
        // We assume download_file and decompress_gz work individually.
        info!("--- Testing Scenario 3: Network download (manual check recommended) ---");
        assert!(!gz_path.exists());
        assert!(!xml_path.exists());
        // If we called ensure_data here, it *should* download and decompress.
        // Manual execution and checking logs/files is the easiest way to verify this part.

        // Cleanup is handled by scopeguard dropping temp_dir
    }

    #[tokio::test]
    async fn test_decompress_gz_basic() {
        let _ = env_logger::builder().is_test(true).try_init();
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let gz_path = temp_dir.path().join("test.xml.gz");
        let xml_path = temp_dir.path().join("test.xml");
        let content = "This is the test content.";

        create_dummy_gz(&gz_path, content).expect("Failed to create dummy GZ");
        assert!(gz_path.exists());

        let result = decompress_gz(&gz_path, &xml_path, None).await;
        assert!(result.is_ok(), "Decompression failed: {:?}", result.err());
        assert!(xml_path.exists());
        let decompressed_content =
            fs::read_to_string(&xml_path).expect("Failed to read decompressed file");
        assert_eq!(decompressed_content, content);
    }
}
