// Declare modules
pub mod data;
pub mod db;
pub mod error;
pub mod models;
pub mod parse;
pub mod progress;

// Re-export key types for easier use
pub use error::{OewnError, Result};
pub use models::{
    Definition,
    Example,
    ILIDefinition,
    Lemma,
    LexicalEntry,
    LexicalResource,
    Lexicon,
    PartOfSpeech,
    Pronunciation,
    Sense,
    SenseRelType,
    SenseRelation,
    Synset,
    SynsetRelType,
    SynsetRelation,
};
use crate::progress::{ProgressCallback, ProgressUpdate};

use crate::db::{string_to_part_of_speech, string_to_sense_rel_type, string_to_synset_rel_type};
use directories_next::ProjectDirs;
use log::{debug, error, info, warn};
use parse::parse_lmf;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params}; // Import rusqlite types
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex}; // Use Mutex for interior mutability of Connection

// --- Constants ---

// --- Processed Data Structure ---

// --- WordNet Struct ---

/// Options for loading WordNet data.
#[derive(Debug, Default, Clone)]
pub struct LoadOptions {
    /// Optional path to a specific database file to use or create.
    /// If None, the default location based on ProjectDirs will be used.
    pub db_path: Option<PathBuf>,
    /// Force reloading data from XML and repopulating the database,
    /// ignoring any existing database content.
    pub force_reload: bool,
}

/// The main WordNet interface.
#[derive(Clone)] // Clone is cheap due to Arc<Mutex<...>>
pub struct WordNet {
    // Use Arc<Mutex<>> for thread-safe access to the connection if WordNet needs to be Send + Sync
    conn: Arc<Mutex<Connection>>,
}

// Helper function to open/create the database connection
// This encapsulates the logic of setting flags and pragmas
fn open_db_connection(path: &Path) -> Result<Connection> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(OewnError::Io)?;
    }
    // Open the connection with flags for read/write/create
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;

    // Optimize connection for performance (optional but recommended)
    // Use WAL mode for better concurrency (readers don't block writers)
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // Increase cache size (adjust based on available memory and testing)
    conn.pragma_update(None, "cache_size", "-64000")?; // e.g., -64000 = 64MB
    // Use memory-mapped I/O (can improve performance, adjust size)
    // conn.pragma_update(None, "mmap_size", 268435456)?; // e.g., 256MB
    // Synchronous off can be faster but less safe on power loss (use NORMAL or FULL for safety)
    conn.pragma_update(None, "synchronous", "NORMAL")?;

    Ok(conn)
}

impl WordNet {
    /// Loads the WordNet data using default options (automatic database path) and no progress reporting.
    ///
    /// Ensures data is downloaded/extracted if needed.
    /// Opens/creates the database, initializes schema, and populates from XML if necessary.
    pub async fn load() -> Result<Self> {
        Self::load_with_options(LoadOptions::default(), None).await // Pass None for callback
    }

    /// Loads the WordNet data with specific options and an optional progress callback.
    pub async fn load_with_options(
        options: LoadOptions,
        progress_callback: Option<ProgressCallback>, // Keep original parameter type
    ) -> Result<Self> {
        // Wrap the callback in Arc<Mutex<Option<...>>> for safe sharing across async/sync boundaries
        let reporter = Arc::new(Mutex::new(progress_callback));

        // Helper closure to simplify reporting
        let report = |update: ProgressUpdate| {
            if let Some(cb) = reporter.lock().unwrap().as_mut() {
                let _ = cb(update); // Ignore return value for now
            }
        };

        // 1. Determine database file path
        let db_path = match options.db_path {
            Some(path) => {
                info!("Using provided database path: {:?}", path);
                path
            }
            None => Self::get_default_db_path()?,
        };
        info!("Using database path: {:?}", db_path);

        let db_exists = db_path.exists();
        let mut needs_population = !db_exists || options.force_reload;

        // 2. Open/Create Database Connection
        let mut conn = open_db_connection(&db_path)?;

        // 3. Initialize Schema (creates tables/indices if they don't exist)
        // This also checks/updates the schema version metadata.
        // If the schema is old, it currently just warns and updates the version number.
        // A real migration strategy would be needed for schema changes.
        db::initialize_database(&mut conn)?;

        // 4. Check if population is needed (beyond just file existence/force_reload)
        // We can check if a core table (e.g., lexicons) is empty.
        if !needs_population {
            let lexicon_count: i64 =
                conn.query_row("SELECT COUNT(*) FROM lexicons", [], |row| row.get(0))?;
            if lexicon_count == 0 {
                info!("Database exists but appears empty. Triggering population.");
                needs_population = true;
            } else {
                info!("Database exists and contains data. Skipping population.");
            }
        }

        // 5. Populate database if needed
        if needs_population {
            if options.force_reload && db_exists {
                info!(
                    "Force reload requested. Clearing existing database data before population..."
                );
                // Use a transaction to clear data efficiently
                let tx = conn.transaction()?;
                db::clear_database_data(&tx)?;
                tx.commit()?; // Commit the clearing transaction
                info!("Existing data cleared.");
            } else {
                info!("Database needs population (first run or empty).");
            }

            // Ensure raw XML data file is present, passing the reporter Arc
            let xml_path = data::ensure_data(reporter.clone()).await?; // Pass Arc clone
            info!("OEWN XML data available at: {:?}", xml_path);

            // --- Read XML File ---
            let read_stage = "Reading XML file".to_string();
            info!("Reading XML file: {:?}", xml_path);
            report(ProgressUpdate::new_stage(read_stage.clone(), None)); // Indeterminate start
            let xml_content = tokio::fs::read_to_string(&xml_path).await?;
            report(ProgressUpdate { // Indicate completion
                stage_description: read_stage,
                current_item: 1,
                total_items: Some(1),
                message: Some("Read complete.".to_string()),
            });

            // --- Parse XML Data ---
            let parse_stage = "Parsing XML data".to_string();
            info!("Parsing XML data...");
            report(ProgressUpdate::new_stage(parse_stage.clone(), None)); // Indeterminate start
            let resource = parse_lmf(xml_content).await?; // Pass owned String
            report(ProgressUpdate { // Indicate completion
                stage_description: parse_stage,
                current_item: 1,
                total_items: Some(1),
                message: Some("Parsing complete.".to_string()),
            });

            // --- Populate Database ---
            // populate_database handles its own transaction and progress reporting internally
            // Pass the reporter Arc clone.
            db::populate_database(&mut conn, resource, reporter.clone())?; // Pass Arc clone
        } else {
            info!("Using existing populated database: {:?}", db_path);
        }

        Ok(WordNet {
            conn: Arc::new(Mutex::new(conn)), // Wrap connection in Arc<Mutex>
        })
    }

    /// Gets the default path for the SQLite database file.
    pub fn get_default_db_path() -> Result<PathBuf> {
        let project_dirs = ProjectDirs::from("org", "OewnRs", data::OEWN_SUBDIR)
            .ok_or(OewnError::DataDirNotFound)?;
        // Use data_dir instead of cache_dir for the database
        let data_dir = project_dirs.data_dir();
        fs::create_dir_all(data_dir)?; // Ensure the directory exists
        let db_filename = format!(
            "oewn-{}.db", // Simpler filename for the DB
            data::OEWN_VERSION,
            // Schema version is now stored inside the DB (metadata table)
        );
        Ok(data_dir.join(db_filename))
    }

    /// Clears the WordNet database file(s).
    ///
    /// If `db_path_override` is `Some`, it attempts to delete that specific file.
    /// If `db_path_override` is `None`, it calculates the default database path and attempts to delete that file.
    pub fn clear_database(db_path_override: Option<PathBuf>) -> Result<()> {
        let path_to_clear = match db_path_override {
            Some(path) => {
                info!("Attempting to clear specified database file: {:?}", path);
                path
            }
            None => {
                let default_path = Self::get_default_db_path()?;
                info!(
                    "Attempting to clear default database file: {:?}",
                    default_path
                );
                default_path
            }
        };

        if path_to_clear.exists() {
            // Attempt to delete the main database file
            match std::fs::remove_file(&path_to_clear) {
                Ok(_) => {
                    info!("Successfully deleted database file: {:?}", path_to_clear);
                    // Also attempt to delete WAL and SHM files if they exist
                    let wal_path = path_to_clear.with_extension("db-wal");
                    let shm_path = path_to_clear.with_extension("db-shm");
                    if wal_path.exists() {
                        let _ = std::fs::remove_file(wal_path); // Ignore error if deletion fails
                    }
                    if shm_path.exists() {
                        let _ = std::fs::remove_file(shm_path); // Ignore error if deletion fails
                    }
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to delete database file {:?}: {}", path_to_clear, e);
                    Err(OewnError::Io(e))
                }
            }
        } else {
            info!(
                "Database file not found, nothing to clear: {:?}",
                path_to_clear
            );
            Ok(()) // Not an error if the file doesn't exist
        }
    }

    /// Clears the default WordNet database file.
    pub fn clear_default_database() -> Result<()> {
        Self::clear_database(None)
    }

    // --- Query Methods ---

    /// Looks up lexical entries (including pronunciations, senses, and sense relations) for a given lemma,
    /// optionally filtering by PartOfSpeech, using a single optimized query.
    /// Returns owned LexicalEntry structs fetched from the DB.
    pub fn lookup_entries(
        &self,
        lemma: &str,
        pos_filter: Option<PartOfSpeech>,
    ) -> Result<Vec<LexicalEntry>> {
        debug!(
            "lookup_entries (optimized): lemma='{}', pos={:?}",
            lemma, pos_filter
        );
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let pos_str_filter = pos_filter.map(db::part_of_speech_to_string);

        // Single query joining entries, pronunciations, senses, and sense relations
        // Filtered by lemma (lowercase) and optionally POS
        let sql = "
            SELECT
                le.id AS entry_id, le.lemma_written_form, le.part_of_speech,
                p.variety, p.notation, p.phonemic, p.audio, p.text AS pron_text,
                s.id AS sense_id, s.synset_id,
                sr.target_sense_id AS sense_rel_target, sr.rel_type AS sense_rel_type
            FROM lexical_entries le
            LEFT JOIN pronunciations p ON le.id = p.entry_id
            LEFT JOIN senses s ON le.id = s.entry_id
            LEFT JOIN sense_relations sr ON s.id = sr.source_sense_id -- Note: JOINING sense_relations on s.id, not le.id
            WHERE le.lemma_written_form_lower = ?1 AND (?2 IS NULL OR le.part_of_speech = ?2)
            ORDER BY le.id, s.id -- Order is crucial for grouping
        ";
        let mut stmt = conn.prepare(sql)?;

        // Use HashMaps to aggregate data during iteration
        let mut entries_map: std::collections::HashMap<String, LexicalEntry> =
            std::collections::HashMap::new();
        // Temporary storage for multi-valued fields, keyed by entry_id
        let mut temp_pronunciations: std::collections::HashMap<
            String,
            std::collections::HashSet<Pronunciation>,
        > = std::collections::HashMap::new();
        // Temporary storage for senses, keyed by entry_id, then sense_id
        let mut temp_senses: std::collections::HashMap<
            String,
            std::collections::HashMap<String, Sense>,
        > = std::collections::HashMap::new();

        let rows_iter = stmt.query_map(params![lemma.to_lowercase(), pos_str_filter], |row| {
            // --- Extract Core Entry Data ---
            let entry_id: String = row.get("entry_id")?;
            let lemma_written_form: String = row.get("lemma_written_form")?;
            let part_of_speech_str: String = row.get("part_of_speech")?;
            let part_of_speech = string_to_part_of_speech(&part_of_speech_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

            // --- Create or Get Entry in Map ---
            // Prefix with _ as the variable itself isn't used directly after insertion/retrieval
            let _entry_map_entry =
                entries_map
                    .entry(entry_id.clone())
                    .or_insert_with(|| LexicalEntry {
                        id: entry_id.clone(),
                        lemma: Lemma {
                            written_form: lemma_written_form,
                            part_of_speech,
                        },
                        pronunciations: Vec::new(),
                        senses: Vec::new(),
                    });

            // --- Extract and Store Pronunciation ---
            let pron_variety: Option<String> = row.get("variety")?;
            if let Some(var) = pron_variety {
                let pron_notation: Option<String> = row.get("notation")?;
                let pron_phonemic_int: Option<i64> = row.get("phonemic")?;
                let pron_audio: Option<String> = row.get("audio")?;
                let pron_text: Option<String> = row.get("pron_text")?;
                if let (Some(ph_int), Some(txt)) = (pron_phonemic_int, pron_text) {
                    temp_pronunciations
                        .entry(entry_id.clone())
                        .or_default()
                        .insert(Pronunciation {
                            variety: var,
                            notation: pron_notation,
                            phonemic: ph_int != 0,
                            audio: pron_audio,
                            text: txt,
                        });
                } else {
                    warn!(
                        "Incomplete pronunciation data found during lookup for entry {}",
                        entry_id
                    );
                }
            }

            // --- Extract and Store Sense and Sense Relation ---
            let sense_id_opt: Option<String> = row.get("sense_id")?;
            if let Some(sense_id) = sense_id_opt {
                let synset_id: String = row.get("synset_id")?; // Should exist if sense_id exists
                let sense_rel_target: Option<String> = row.get("sense_rel_target")?;
                let sense_rel_type_str: Option<String> = row.get("sense_rel_type")?;

                // Get or create the sense within the entry's sense map
                let entry_senses = temp_senses.entry(entry_id.clone()).or_default();
                let sense_entry = entry_senses
                    .entry(sense_id.clone())
                    .or_insert_with(|| Sense {
                        id: sense_id.clone(),
                        synset: synset_id,
                        sense_relations: Vec::new(),
                    });

                // Add relation if present
                if let (Some(target), Some(rel_str)) = (sense_rel_target, sense_rel_type_str) {
                    let rel_type = string_to_sense_rel_type(&rel_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            11,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                    let new_relation = SenseRelation { target, rel_type };
                    if !sense_entry.sense_relations.contains(&new_relation) {
                        sense_entry.sense_relations.push(new_relation);
                    }
                }
            }

            Ok(())
        })?;

        // Consume iterator to process all rows
        for result in rows_iter {
            result?;
        }

        // Populate the final entries from the aggregated data
        for (entry_id, entry) in entries_map.iter_mut() {
            if let Some(prons) = temp_pronunciations.remove(entry_id) {
                entry.pronunciations = prons.into_iter().collect();
            }
            if let Some(senses) = temp_senses.remove(entry_id) {
                entry.senses = senses.into_values().collect();
                // Sort senses by ID for consistent output (optional)
                entry.senses.sort_by(|a, b| a.id.cmp(&b.id));
            }
        }

        let final_entries: Vec<LexicalEntry> = entries_map.into_values().collect();

        if final_entries.is_empty() {
            debug!(
                "No entries found for lemma '{}', pos_filter: {:?}",
                lemma, pos_filter
            );
        }
        Ok(final_entries)
    }

    /// Retrieves a specific Synset by its ID string.
    /// Returns an owned Synset struct fetched from the DB.
    pub fn get_synset(&self, id: &str) -> Result<Synset> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_full_synset_by_id(&conn_guard, id)?
            .ok_or_else(|| OewnError::SynsetNotFound(id.to_string()))
    }

    /// Retrieves a specific Sense by its ID string.
    /// Returns an owned Sense struct fetched from the DB.
    pub fn get_sense(&self, id: &str) -> Result<Sense> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_full_sense_by_id(&conn_guard, id)?
            .ok_or_else(|| OewnError::Internal(format!("Sense ID not found: {}", id))) // Should not happen if DB is consistent
    }

    /// Retrieves all Senses associated with a specific Lexical Entry ID.
    /// Returns owned Sense structs fetched from the DB.
    pub fn get_senses_for_entry(&self, entry_id: &str) -> Result<Vec<Sense>> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_senses_for_entry_internal(&conn_guard, entry_id)
    }

    /// Retrieves all Senses (including their relations) associated with a specific Synset ID using JOINs.
    /// Returns owned Sense structs fetched from the DB.
    pub fn get_senses_for_synset(&self, synset_id: &str) -> Result<Vec<Sense>> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let sql = "
            SELECT
                s.id, s.synset_id,
                sr.target_sense_id, sr.rel_type
            FROM senses s
            LEFT JOIN sense_relations sr ON s.id = sr.source_sense_id
            WHERE s.synset_id = ?1
            ORDER BY s.id -- Important for grouping results by sense
        ";
        let mut stmt = conn.prepare(sql)?;

        // Use a HashMap to group relations by sense ID during iteration
        let mut senses_map: std::collections::HashMap<String, Sense> =
            std::collections::HashMap::new();

        let rows_iter = stmt.query_map(params![synset_id], |row| {
            // Extract data from the row
            let sense_id: String = row.get(0)?;
            let current_synset_id: String = row.get(1)?; // Should match input synset_id
            let target_sense_id: Option<String> = row.get(2)?;
            let rel_type_str: Option<String> = row.get(3)?;

            // Create or get the Sense struct from the map
            let sense_entry = senses_map.entry(sense_id.clone()).or_insert_with(|| Sense {
                id: sense_id.clone(),
                synset: current_synset_id, // Use the value from the row
                sense_relations: Vec::new(), // Initialize relations vector
            });

            // If relation data exists (due to LEFT JOIN), parse and add it
            if let (Some(target), Some(rel_str)) = (target_sense_id, rel_type_str) {
                let rel_type = string_to_sense_rel_type(&rel_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                // Avoid adding duplicate relations if a sense has multiple relations of the same type to the same target
                let new_relation = SenseRelation { target, rel_type };
                if !sense_entry.sense_relations.contains(&new_relation) {
                    sense_entry.sense_relations.push(new_relation);
                }
            }

            Ok(()) // query_map expects a Result, Ok(()) indicates success for this row processing
        })?;

        // Consume the iterator to process all rows and populate the map
        for result in rows_iter {
            result?; // Propagate any error
        }

        // Convert the map values (Senses) into a Vec
        Ok(senses_map.into_values().collect())
    }

    /// Retrieves a random lexical entry.
    /// Returns an owned LexicalEntry struct fetched from the DB.
    pub fn get_random_entry(&self) -> Result<LexicalEntry> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        // Get a random entry ID first
        let mut stmt_id =
            conn.prepare("SELECT id FROM lexical_entries ORDER BY RANDOM() LIMIT 1")?;
        let random_id_opt: Option<String> = stmt_id.query_row([], |row| row.get(0)).optional()?;

        match random_id_opt {
            Some(id) => self
                .fetch_full_entry_by_id(conn, &id)?
                .ok_or_else(|| OewnError::Internal(format!("Random entry ID {} not found.", id))), // Should not happen
            None => Err(OewnError::Internal(
                "No entries found in database.".to_string(),
            )),
        }
    }

    /// Retrieves all lexical entries (including pronunciations, senses, and sense relations) using a single optimized query.
    /// Note: This fetches the entire dataset into memory and can be very resource-intensive. Use with caution.
    /// Returns owned LexicalEntry structs fetched from the DB.
    pub fn all_entries(&self) -> Result<Vec<LexicalEntry>> {
        warn!(
            "all_entries() (optimized) called: Fetching all entries and related data from DB. This might be slow and very memory-intensive."
        );
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        // Single query joining entries, pronunciations, senses, and sense relations for ALL entries
        let sql = "
            SELECT
                le.id AS entry_id, le.lemma_written_form, le.part_of_speech,
                p.variety, p.notation, p.phonemic, p.audio, p.text AS pron_text,
                s.id AS sense_id, s.synset_id,
                sr.target_sense_id AS sense_rel_target, sr.rel_type AS sense_rel_type
            FROM lexical_entries le
            LEFT JOIN pronunciations p ON le.id = p.entry_id
            LEFT JOIN senses s ON le.id = s.entry_id
            LEFT JOIN sense_relations sr ON s.id = sr.source_sense_id
            ORDER BY le.id, s.id -- Order is crucial for grouping
        ";
        let mut stmt = conn.prepare(sql)?;

        // Use HashMaps to aggregate data during iteration
        let mut entries_map: std::collections::HashMap<String, LexicalEntry> =
            std::collections::HashMap::new();
        // Temporary storage for multi-valued fields, keyed by entry_id
        let mut temp_pronunciations: std::collections::HashMap<
            String,
            std::collections::HashSet<Pronunciation>,
        > = std::collections::HashMap::new();
        // Temporary storage for senses, keyed by entry_id, then sense_id
        let mut temp_senses: std::collections::HashMap<
            String,
            std::collections::HashMap<String, Sense>,
        > = std::collections::HashMap::new();

        let rows_iter = stmt.query_map([], |row| {
            // No parameters needed for all_entries
            // --- Extract Core Entry Data ---
            let entry_id: String = row.get("entry_id")?;
            let lemma_written_form: String = row.get("lemma_written_form")?;
            let part_of_speech_str: String = row.get("part_of_speech")?;
            let part_of_speech = string_to_part_of_speech(&part_of_speech_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

            // --- Create or Get Entry in Map ---
            // Prefix with _ as the variable itself isn't used directly after insertion/retrieval
            let _entry_map_entry =
                entries_map
                    .entry(entry_id.clone())
                    .or_insert_with(|| LexicalEntry {
                        id: entry_id.clone(),
                        lemma: Lemma {
                            written_form: lemma_written_form,
                            part_of_speech,
                        },
                        pronunciations: Vec::new(),
                        senses: Vec::new(),
                    });

            // --- Extract and Store Pronunciation ---
            let pron_variety: Option<String> = row.get("variety")?;
            if let Some(var) = pron_variety {
                let pron_notation: Option<String> = row.get("notation")?;
                let pron_phonemic_int: Option<i64> = row.get("phonemic")?;
                let pron_audio: Option<String> = row.get("audio")?;
                let pron_text: Option<String> = row.get("pron_text")?;
                if let (Some(ph_int), Some(txt)) = (pron_phonemic_int, pron_text) {
                    temp_pronunciations
                        .entry(entry_id.clone())
                        .or_default()
                        .insert(Pronunciation {
                            variety: var,
                            notation: pron_notation,
                            phonemic: ph_int != 0,
                            audio: pron_audio,
                            text: txt,
                        });
                } else {
                    warn!(
                        "Incomplete pronunciation data found during all_entries for entry {}",
                        entry_id
                    );
                }
            }

            // --- Extract and Store Sense and Sense Relation ---
            let sense_id_opt: Option<String> = row.get("sense_id")?;
            if let Some(sense_id) = sense_id_opt {
                let synset_id: String = row.get("synset_id")?; // Should exist if sense_id exists
                let sense_rel_target: Option<String> = row.get("sense_rel_target")?;
                let sense_rel_type_str: Option<String> = row.get("sense_rel_type")?;

                // Get or create the sense within the entry's sense map
                let entry_senses = temp_senses.entry(entry_id.clone()).or_default();
                let sense_entry = entry_senses
                    .entry(sense_id.clone())
                    .or_insert_with(|| Sense {
                        id: sense_id.clone(),
                        synset: synset_id,
                        sense_relations: Vec::new(),
                    });

                // Add relation if present
                if let (Some(target), Some(rel_str)) = (sense_rel_target, sense_rel_type_str) {
                    let rel_type = string_to_sense_rel_type(&rel_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            11,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                    let new_relation = SenseRelation { target, rel_type };
                    if !sense_entry.sense_relations.contains(&new_relation) {
                        sense_entry.sense_relations.push(new_relation);
                    }
                }
            }

            Ok(())
        })?;

        // Consume iterator to process all rows
        info!("Processing rows for all_entries..."); // Add info log
        let mut row_count = 0;
        for result in rows_iter {
            result?;
            row_count += 1;
            if row_count % 100000 == 0 {
                // Log progress periodically
                info!("Processed {} rows for all_entries...", row_count);
            }
        }
        info!("Finished processing {} rows for all_entries.", row_count);

        // Populate the final entries from the aggregated data
        info!("Aggregating final entry data...");
        for (entry_id, entry) in entries_map.iter_mut() {
            if let Some(prons) = temp_pronunciations.remove(entry_id) {
                entry.pronunciations = prons.into_iter().collect();
            }
            if let Some(senses) = temp_senses.remove(entry_id) {
                entry.senses = senses.into_values().collect();
                // Sort senses by ID for consistent output (optional)
                entry.senses.sort_by(|a, b| a.id.cmp(&b.id));
            }
        }
        info!("Aggregation complete.");

        Ok(entries_map.into_values().collect())
    }

    // --- Public Helper Methods ---

    /// Retrieves the entry ID for a given sense ID.
    pub fn get_entry_id_for_sense(&self, sense_id: &str) -> Result<Option<String>> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;
        let mut stmt = conn.prepare("SELECT entry_id FROM senses WHERE id = ?1")?;
        stmt.query_row(params![sense_id], |row| row.get(0))
            .optional()
            .map_err(OewnError::from)
    }

    /// Retrieves an entry by its ID.
    /// Returns an owned LexicalEntry struct fetched from the DB.
    pub fn get_entry_by_id(&self, entry_id: &str) -> Result<Option<LexicalEntry>> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_full_entry_by_id(&conn_guard, entry_id)
    }

    /// Internal helper to fetch full LexicalEntry data including pronunciations and senses.
    /// Fetches entry + pronunciations with JOIN, then calls optimized sense fetcher.
    fn fetch_full_entry_by_id(
        &self,
        conn: &Connection,
        entry_id: &str,
    ) -> Result<Option<LexicalEntry>> {
        let sql = "
            SELECT
                le.id, le.lemma_written_form, le.part_of_speech,
                p.variety, p.notation, p.phonemic, p.audio, p.text AS pron_text
            FROM lexical_entries le
            LEFT JOIN pronunciations p ON le.id = p.entry_id
            WHERE le.id = ?1
        ";
        let mut stmt = conn.prepare(sql)?;

        let mut entry_opt: Option<LexicalEntry> = None;
        let mut pronunciations_temp: std::collections::HashSet<Pronunciation> =
            std::collections::HashSet::new(); // Use HashSet for uniqueness

        let rows_iter = stmt.query_map(params![entry_id], |row| {
            // Extract core entry data (only needed once)
            if entry_opt.is_none() {
                let id: String = row.get(0)?;
                let lemma = row_to_lemma(row)?; // Uses cols 1 and 2

                entry_opt = Some(LexicalEntry {
                    id: id.clone(),
                    lemma,
                    pronunciations: Vec::new(), // Initialize
                    senses: Vec::new(),         // Initialize
                });
            }

            // Extract pronunciation data (if present in this row)
            let variety: Option<String> = row.get(3)?;
            if let Some(var) = variety {
                // Check if pronunciation exists for this row
                let notation: Option<String> = row.get(4)?;
                let phonemic_int: Option<i64> = row.get(5)?;
                let audio: Option<String> = row.get(6)?;
                let text: Option<String> = row.get(7)?; // Renamed to pron_text in SQL

                // Ensure required fields are present (variety and text should be NOT NULL in schema ideally)
                if let (Some(ph_int), Some(txt)) = (phonemic_int, text) {
                    pronunciations_temp.insert(Pronunciation {
                        variety: var,
                        notation,
                        phonemic: ph_int != 0,
                        audio,
                        text: txt,
                    });
                } else {
                    warn!("Incomplete pronunciation data found for entry {}", entry_id);
                }
            }
            Ok(())
        })?;

        // Consume iterator
        for result in rows_iter {
            result?;
        }

        // If an entry was found, assign pronunciations and fetch senses
        if let Some(entry) = entry_opt.as_mut() {
            entry.pronunciations = pronunciations_temp.into_iter().collect();
            // Fetch senses using the already optimized internal function
            entry.senses = self.fetch_senses_for_entry_internal(conn, entry_id)?;
        }

        Ok(entry_opt)
    }

    /// Internal helper to fetch senses and their relations for a given entry ID using a JOIN.
    fn fetch_senses_for_entry_internal(
        &self,
        conn: &Connection,
        entry_id: &str,
    ) -> Result<Vec<Sense>> {
        let sql = "
            SELECT
                s.id, s.synset_id,
                sr.target_sense_id, sr.rel_type
            FROM senses s
            LEFT JOIN sense_relations sr ON s.id = sr.source_sense_id
            WHERE s.entry_id = ?1
            ORDER BY s.id -- Important for grouping results by sense
        ";
        let mut stmt = conn.prepare(sql)?;

        // Use a HashMap to group relations by sense ID during iteration
        let mut senses_map: std::collections::HashMap<String, Sense> =
            std::collections::HashMap::new();

        let rows_iter = stmt.query_map(params![entry_id], |row| {
            // Extract data from the row
            let sense_id: String = row.get(0)?;
            let synset_id: String = row.get(1)?;
            let target_sense_id: Option<String> = row.get(2)?;
            let rel_type_str: Option<String> = row.get(3)?;

            // Create or get the Sense struct from the map
            let sense_entry = senses_map.entry(sense_id.clone()).or_insert_with(|| Sense {
                id: sense_id.clone(),
                synset: synset_id,
                sense_relations: Vec::new(), // Initialize relations vector
            });

            // If relation data exists (due to LEFT JOIN), parse and add it
            if let (Some(target), Some(rel_str)) = (target_sense_id, rel_type_str) {
                let rel_type = string_to_sense_rel_type(&rel_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                sense_entry
                    .sense_relations
                    .push(SenseRelation { target, rel_type });
            }

            Ok(()) // query_map expects a Result, Ok(()) indicates success for this row processing
        })?;

        // Consume the iterator to process all rows and populate the map
        // Explicitly handle potential errors during iteration
        for result in rows_iter {
            result?; // Propagate any error from query_map closure or DB interaction
        }

        // Convert the map values (Senses) into a Vec
        Ok(senses_map.into_values().collect())
    }

    /// Retrieves related Senses (including their relations) for a given source Sense ID and relation type using JOINs.
    /// Returns owned Sense structs fetched from the DB.
    pub fn get_related_senses(&self, sense_id: &str, rel_type: SenseRelType) -> Result<Vec<Sense>> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let rel_type_str = db::sense_rel_type_to_string(rel_type);

        // Query joins sense_relations (sr1) to find targets, then joins senses (s_target) for target sense details,
        // then LEFT JOINs sense_relations (sr_target) again to get relations *of the target sense*.
        let sql = "
            SELECT
                s_target.id, s_target.synset_id,
                sr_target.target_sense_id AS target_rel_target_id,
                sr_target.rel_type AS target_rel_type
            FROM sense_relations sr1
            JOIN senses s_target ON sr1.target_sense_id = s_target.id
            LEFT JOIN sense_relations sr_target ON s_target.id = sr_target.source_sense_id
            WHERE sr1.source_sense_id = ?1 AND sr1.rel_type = ?2
            ORDER BY s_target.id -- Important for grouping
        ";
        let mut stmt = conn.prepare(sql)?;

        let mut senses_map: std::collections::HashMap<String, Sense> =
            std::collections::HashMap::new();

        let rows_iter = stmt.query_map(params![sense_id, rel_type_str], |row| {
            // Extract target sense data
            let target_sense_id: String = row.get(0)?;
            let target_synset_id: String = row.get(1)?;
            let target_rel_target_id: Option<String> = row.get(2)?;
            let target_rel_type_str: Option<String> = row.get(3)?;

            // Create or get the target Sense struct
            let sense_entry = senses_map
                .entry(target_sense_id.clone())
                .or_insert_with(|| Sense {
                    id: target_sense_id.clone(),
                    synset: target_synset_id,
                    sense_relations: Vec::new(),
                });

            // If relation data *for the target sense* exists, parse and add it
            if let (Some(target_rel_target), Some(target_rel_str)) =
                (target_rel_target_id, target_rel_type_str)
            {
                let rel_type = string_to_sense_rel_type(&target_rel_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                let new_relation = SenseRelation {
                    target: target_rel_target,
                    rel_type,
                };
                if !sense_entry.sense_relations.contains(&new_relation) {
                    // Avoid duplicates
                    sense_entry.sense_relations.push(new_relation);
                }
            }
            Ok(())
        })?;

        // Consume iterator
        for result in rows_iter {
            result?;
        }

        Ok(senses_map.into_values().collect())
    }

    /// Internal helper to fetch full Sense data including relations using a JOIN.
    fn fetch_full_sense_by_id(&self, conn: &Connection, sense_id: &str) -> Result<Option<Sense>> {
        let sql = "
            SELECT
                s.id, s.synset_id,
                sr.target_sense_id, sr.rel_type
            FROM senses s
            LEFT JOIN sense_relations sr ON s.id = sr.source_sense_id
            WHERE s.id = ?1
        ";
        let mut stmt = conn.prepare(sql)?;

        let mut sense_opt: Option<Sense> = None;
        let mut relations_temp: Vec<SenseRelation> = Vec::new();

        let rows_iter = stmt.query_map(params![sense_id], |row| {
            // Extract data from the row
            let current_sense_id: String = row.get(0)?; // Should always be the same as input sense_id
            let synset_id: String = row.get(1)?;
            let target_sense_id: Option<String> = row.get(2)?;
            let rel_type_str: Option<String> = row.get(3)?;

            // Initialize the Sense struct on the first row
            if sense_opt.is_none() {
                sense_opt = Some(Sense {
                    id: current_sense_id.clone(),
                    synset: synset_id,
                    sense_relations: Vec::new(), // Initialize relations vector
                });
            }

            // If relation data exists (due to LEFT JOIN), parse and add it to temp vec
            if let (Some(target), Some(rel_str)) = (target_sense_id, rel_type_str) {
                let rel_type = string_to_sense_rel_type(&rel_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                relations_temp.push(SenseRelation { target, rel_type });
            }

            Ok(()) // query_map expects a Result, Ok(()) indicates success for this row processing
        })?;

        // Consume the iterator to process all rows
        for result in rows_iter {
            result?; // Propagate any error
        }

        // If a sense was found, assign the collected relations
        if let Some(sense) = sense_opt.as_mut() {
            sense.sense_relations = relations_temp;
        }

        Ok(sense_opt)
    }

    /// Retrieves related Synsets (including their definitions, examples, relations) for a given source Synset ID and relation type using JOINs.
    /// Returns owned Synset structs fetched from the DB.
    pub fn get_related_synsets(
        &self,
        synset_id: &str,
        rel_type: SynsetRelType,
    ) -> Result<Vec<Synset>> {
        let conn_guard = self
            .conn
            .lock()
            .map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let rel_type_str = db::synset_rel_type_to_string(rel_type);

        // This complex query finds target synsets via sr1, joins to get target synset details (s_target),
        // and then LEFT JOINs definitions, ili_defs, examples, and relations *of the target synset*.
        let sql = "
            SELECT
                s_target.id, s_target.ili, s_target.part_of_speech,
                d.text AS def_text, d.dc_source AS def_source,
                id.text AS ili_def_text, id.dc_source AS ili_def_source,
                e.text AS ex_text, e.dc_source AS ex_source,
                sr_target.target_synset_id AS target_rel_target_id,
                sr_target.rel_type AS target_rel_type
            FROM synset_relations sr1
            JOIN synsets s_target ON sr1.target_synset_id = s_target.id
            LEFT JOIN definitions d ON s_target.id = d.synset_id
            LEFT JOIN ili_definitions id ON s_target.id = id.synset_id
            LEFT JOIN examples e ON s_target.id = e.synset_id
            LEFT JOIN synset_relations sr_target ON s_target.id = sr_target.source_synset_id
            WHERE sr1.source_synset_id = ?1 AND sr1.rel_type = ?2
            ORDER BY s_target.id -- Important for grouping
        ";
        let mut stmt = conn.prepare(sql)?;

        let mut synsets_map: std::collections::HashMap<String, Synset> =
            std::collections::HashMap::new();
        // Temporary storage for multi-valued fields within the map processing closure
        let mut temp_defs: std::collections::HashMap<
            String,
            std::collections::HashSet<Definition>,
        > = std::collections::HashMap::new();
        let mut temp_examples: std::collections::HashMap<
            String,
            std::collections::HashSet<Example>,
        > = std::collections::HashMap::new();
        let mut temp_relations: std::collections::HashMap<
            String,
            std::collections::HashSet<SynsetRelation>,
        > = std::collections::HashMap::new();

        let rows_iter = stmt.query_map(params![synset_id, rel_type_str], |row| {
            // Extract target synset core data
            let target_id: String = row.get(0)?;
            let target_ili: Option<String> = row.get(1)?;
            let target_pos_str: String = row.get(2)?;
            let target_part_of_speech = string_to_part_of_speech(&target_pos_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

            // Create or get the target Synset struct (without multi-valued fields initially)
            let synset_entry = synsets_map
                .entry(target_id.clone())
                .or_insert_with(|| Synset {
                    id: target_id.clone(),
                    ili: target_ili,
                    part_of_speech: target_part_of_speech,
                    definitions: Vec::new(),
                    ili_definition: None, // Will be set below if found
                    examples: Vec::new(),
                    synset_relations: Vec::new(),
                    members: String::new(),
                });

            // Extract ILI definition (only needs to be done once per synset)
            if synset_entry.ili_definition.is_none() {
                let ili_text: Option<String> = row.get(5)?;
                let ili_source: Option<String> = row.get(6)?;
                if let Some(text) = ili_text {
                    synset_entry.ili_definition = Some(ILIDefinition {
                        text,
                        dc_source: ili_source,
                    });
                }
            }

            // Extract and store definitions in temporary HashSet for the current target_id
            let def_text: Option<String> = row.get(3)?;
            let def_source: Option<String> = row.get(4)?;
            if let Some(text) = def_text {
                temp_defs
                    .entry(target_id.clone())
                    .or_default()
                    .insert(Definition {
                        text,
                        dc_source: def_source,
                    });
            }

            // Extract and store examples
            let ex_text: Option<String> = row.get(7)?;
            let ex_source: Option<String> = row.get(8)?;
            if let Some(text) = ex_text {
                temp_examples
                    .entry(target_id.clone())
                    .or_default()
                    .insert(Example {
                        text,
                        dc_source: ex_source,
                    });
            }

            // Extract and store relations
            let target_rel_target_id: Option<String> = row.get(9)?;
            let target_rel_type_str: Option<String> = row.get(10)?;
            if let (Some(target), Some(rel_str)) = (target_rel_target_id, target_rel_type_str) {
                let rel_type = string_to_synset_rel_type(&rel_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                temp_relations
                    .entry(target_id.clone())
                    .or_default()
                    .insert(SynsetRelation { target, rel_type });
            }

            Ok(())
        })?;

        // Consume iterator to process all rows
        for result in rows_iter {
            result?;
        }

        // Populate the multi-valued fields from the temporary HashSets
        for (id, synset) in synsets_map.iter_mut() {
            if let Some(defs) = temp_defs.remove(id) {
                synset.definitions = defs.into_iter().collect();
            }
            if let Some(exs) = temp_examples.remove(id) {
                synset.examples = exs.into_iter().collect();
            }
            if let Some(rels) = temp_relations.remove(id) {
                synset.synset_relations = rels.into_iter().collect();
            }
        }

        Ok(synsets_map.into_values().collect())
    }

    // --- Internal Helper Methods ---

    /// Internal helper to fetch full Synset data including relations, definitions, examples using JOINs.
    fn fetch_full_synset_by_id(
        &self,
        conn: &Connection,
        synset_id: &str,
    ) -> Result<Option<Synset>> {
        // This query joins synsets with definitions, ili_definitions, examples, and synset_relations.
        // LEFT JOINs are used to ensure the synset is returned even if it has no definitions, examples, etc.
        let sql = "
            SELECT
                s.id, s.ili, s.part_of_speech,
                d.text AS def_text, d.dc_source AS def_source,
                id.text AS ili_def_text, id.dc_source AS ili_def_source,
                e.text AS ex_text, e.dc_source AS ex_source,
                sr.target_synset_id, sr.rel_type
            FROM synsets s
            LEFT JOIN definitions d ON s.id = d.synset_id
            LEFT JOIN ili_definitions id ON s.id = id.synset_id
            LEFT JOIN examples e ON s.id = e.synset_id
            LEFT JOIN synset_relations sr ON s.id = sr.source_synset_id
            WHERE s.id = ?1
        ";
        let mut stmt = conn.prepare(sql)?;

        let mut synset_opt: Option<Synset> = None;
        // Use HashSets to avoid duplicates when multiple relations/defs/examples exist
        let mut definitions_temp: std::collections::HashSet<Definition> =
            std::collections::HashSet::new();
        let mut examples_temp: std::collections::HashSet<Example> =
            std::collections::HashSet::new();
        let mut relations_temp: std::collections::HashSet<SynsetRelation> =
            std::collections::HashSet::new();
        let mut ili_definition_temp: Option<ILIDefinition> = None;

        let rows_iter = stmt.query_map(params![synset_id], |row| {
            // Extract core synset data (only needed once)
            if synset_opt.is_none() {
                let id: String = row.get(0)?;
                let ili: Option<String> = row.get(1)?;
                let pos_str: String = row.get(2)?;
                let part_of_speech = string_to_part_of_speech(&pos_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

                synset_opt = Some(Synset {
                    id: id.clone(),
                    ili,
                    part_of_speech,
                    definitions: Vec::new(), // Initialize vectors
                    ili_definition: None,
                    examples: Vec::new(),
                    synset_relations: Vec::new(),
                    members: String::new(), // Not fetched directly
                });

                // Extract ILI definition (should only be one row or none)
                let ili_text: Option<String> = row.get(5)?;
                let ili_source: Option<String> = row.get(6)?;
                if let Some(text) = ili_text {
                    ili_definition_temp = Some(ILIDefinition {
                        text,
                        dc_source: ili_source,
                    });
                }
            }

            // Extract definition data (if present in this row)
            let def_text: Option<String> = row.get(3)?;
            let def_source: Option<String> = row.get(4)?;
            if let Some(text) = def_text {
                definitions_temp.insert(Definition {
                    text,
                    dc_source: def_source,
                });
            }

            // Extract example data (if present)
            let ex_text: Option<String> = row.get(7)?;
            let ex_source: Option<String> = row.get(8)?;
            if let Some(text) = ex_text {
                examples_temp.insert(Example {
                    text,
                    dc_source: ex_source,
                });
            }

            // Extract relation data (if present)
            let target_synset_id: Option<String> = row.get(9)?;
            let rel_type_str: Option<String> = row.get(10)?;
            if let (Some(target), Some(rel_str)) = (target_synset_id, rel_type_str) {
                let rel_type = string_to_synset_rel_type(&rel_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                relations_temp.insert(SynsetRelation { target, rel_type });
            }

            Ok(())
        })?;

        // Consume iterator
        for result in rows_iter {
            result?;
        }

        // Assign collected data to the synset if it was found
        if let Some(synset) = synset_opt.as_mut() {
            synset.definitions = definitions_temp.into_iter().collect();
            synset.ili_definition = ili_definition_temp;
            synset.examples = examples_temp.into_iter().collect();
            synset.synset_relations = relations_temp.into_iter().collect();
        }

        Ok(synset_opt)
    }
}

// --- Mapping Helpers (Row -> Struct) ---
// These will be needed when implementing the query methods above.

// Example: Map a rusqlite::Row to a models::Lemma
fn row_to_lemma(row: &Row) -> std::result::Result<Lemma, rusqlite::Error> {
    let pos_str: String = row.get("part_of_speech")?;
    Ok(Lemma {
        written_form: row.get("lemma_written_form")?,
        part_of_speech: string_to_part_of_speech(&pos_str).map_err(|e| {
            // Convert OewnError to rusqlite::Error for query_map compatibility
            rusqlite::Error::FromSqlConversionFailure(
                0, // Placeholder column index
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?,
    })
}

// Add similar row_to_... functions for LexicalEntry, Sense, Synset, etc.
// These functions will often need the Connection to fetch related data (e.g., senses for an entry).

#[cfg(test)]
mod tests {
    // Tests need to be rewritten for the SQLite backend.

    use super::*;

    use tempfile::tempdir;

    // Placeholder test function
    #[tokio::test]
    #[ignore] // Ignore until tests are rewritten
    async fn test_sqlite_loading_placeholder() {
        // Setup temp dir and db path
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_load.db");

        // Create dummy XML data (or use a small test fixture)
        let _dummy_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <LexicalResource>
              <Lexicon id="test-en" label="Test" language="en" email="a@b.c" license="l" version="1">
                <LexicalEntry id="w1">
                  <Lemma writtenForm="cat" partOfSpeech="n"/>
                  <Sense id="s1" synset="syn1"/>
                </LexicalEntry>
                <Synset id="syn1" partOfSpeech="n">
                  <Definition>A feline animal</Definition>
                </Synset>
              </Lexicon>
            </LexicalResource>"#;

        // Mock ensure_data to provide the dummy XML content path (tricky without modifying data.rs)
        // For a real test, might need to write dummy XML to a temp file and point ensure_data there,
        // or refactor ensure_data for testability.

        // Test loading (this will likely fail until ensure_data is handled and methods implemented)
        let _load_options = LoadOptions {
            db_path: Some(db_path.clone()),
            force_reload: true, // Force population for the test
        };
        // let wn_result = WordNet::load_with_options(load_options).await;
        // assert!(wn_result.is_ok());
        // let wn = wn_result.unwrap();

        // Add assertions here once query methods are implemented
        // let entries = wn.lookup_entries("cat", None).unwrap();
        // assert_eq!(entries.len(), 1);
        // assert_eq!(entries[0].lemma.written_form, "cat");

        // Test clear_database
        // assert!(WordNet::clear_database(Some(db_path.clone())).is_ok());
        // assert!(!db_path.exists());
    }
}
