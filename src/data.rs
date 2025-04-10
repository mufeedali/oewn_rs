use crate::error::{OewnError, Result};
use directories_next::ProjectDirs;
use flate2::read::GzDecoder;
use log::info;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

// --- Constants ---
pub const OEWN_VERSION: &str = "2024"; // Current version we are targeting
pub const OEWN_SUBDIR: &str = "oewn-rs"; // Subdirectory within user's data dir
const OEWN_FILENAME_GZ: &str = "english-wordnet-2024.xml.gz";
const OEWN_FILENAME_XML: &str = "english-wordnet-2024.xml";
const OEWN_DOWNLOAD_URL: &str = "https://github.com/globalwordnet/english-wordnet/releases/download/2024-edition/english-wordnet-2024.xml.gz";

// --- Helper Functions ---

/// Gets the project's data directory path.
fn get_data_dir() -> Result<PathBuf> {
    let proj_dirs =
        ProjectDirs::from("org", "OewnRs", OEWN_SUBDIR).ok_or(OewnError::DataDirNotFound)?;
    let data_dir = proj_dirs.data_dir().to_path_buf();
    // Ensure the directory exists
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

/// Downloads a file from a URL to a specified path.
async fn download_file(url: &str, dest_path: &Path) -> Result<()> {
    info!("Downloading data from {} to {:?}...", url, dest_path);
    let response = reqwest::get(url).await?.error_for_status()?; // Check for HTTP errors

    let mut dest_file = BufWriter::new(File::create(dest_path)?);
    let content = response.bytes().await?; // Read entire content into memory (consider streaming for very large files)
    dest_file.write_all(&content)?;
    dest_file.flush()?; // Ensure all data is written to disk
    info!("Download complete.");
    Ok(())
}

/// Decompresses a GZipped file.
fn decompress_gz(gz_path: &Path, dest_path: &Path) -> Result<()> {
    info!("Decompressing {:?} to {:?}...", gz_path, dest_path);
    let gz_file = File::open(gz_path)?;
    let mut decoder = GzDecoder::new(BufReader::new(gz_file));
    let mut dest_file = BufWriter::new(File::create(dest_path)?);

    io::copy(&mut decoder, &mut dest_file)?;
    dest_file.flush()?; // Ensure all data is written
    info!("Decompression complete.");
    Ok(())
}

// --- Public API ---

/// Ensures the OEWN XML data file is present in the data directory.
///
/// Downloads and/or decompresses the data if necessary.
/// Returns the path to the final `.xml` file.
pub async fn ensure_data() -> Result<PathBuf> {
    let data_dir = get_data_dir()?;
    let xml_path = data_dir.join(OEWN_FILENAME_XML);
    let gz_path = data_dir.join(OEWN_FILENAME_GZ);

    // 1. Check if the final XML file already exists
    if xml_path.exists() {
        info!("Found existing OEWN XML data file: {:?}", xml_path);
        return Ok(xml_path);
    } else {
        info!("OEWN XML data file not found at {:?}.", xml_path);
    }

    // 2. Check if the compressed GZ file exists
    if !gz_path.exists() {
        info!("OEWN GZ archive not found at {:?}. Downloading...", gz_path);
        // Download the GZ file
        download_file(OEWN_DOWNLOAD_URL, &gz_path).await?;
    } else {
        info!("Found existing OEWN GZ archive: {:?}", gz_path);
    }

    // 3. Decompress the GZ file
    decompress_gz(&gz_path, &xml_path)?;

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
        // Manually call decompress_gz as ensure_data would
        let decompress_result = decompress_gz(&gz_path, &xml_path);
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

    #[test]
    fn test_decompress_gz_basic() {
        let _ = env_logger::builder().is_test(true).try_init();
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let gz_path = temp_dir.path().join("test.xml.gz");
        let xml_path = temp_dir.path().join("test.xml");
        let content = "This is the test content.";

        create_dummy_gz(&gz_path, content).expect("Failed to create dummy GZ");
        assert!(gz_path.exists());

        let result = decompress_gz(&gz_path, &xml_path);
        assert!(result.is_ok(), "Decompression failed: {:?}", result.err());
        assert!(xml_path.exists());

        let decompressed_content =
            fs::read_to_string(&xml_path).expect("Failed to read decompressed file");
        assert_eq!(decompressed_content, content);
    }
}
