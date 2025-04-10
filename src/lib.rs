// Declare modules
pub mod data;
pub mod db;
pub mod error;
pub mod models;
pub mod parse;

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
    SyntacticBehaviour,
};

use directories_next::ProjectDirs;
use log::{debug, error, info, warn};
use crate::db::{string_to_part_of_speech, string_to_sense_rel_type, string_to_synset_rel_type};
use parse::parse_lmf;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row}; // Import rusqlite types
use std::fs;
use std::path::{Path, PathBuf}; // Keep PathBuf
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
    // If only used in single-threaded async context, Arc<Connection> might suffice,
    // but Mutex is safer for broader usability.
    // Consider a connection pool (like r2d2) for high-concurrency scenarios.
    conn: Arc<Mutex<Connection>>,
    // Keep the path to the database file for reference or potential future operations
    #[allow(dead_code)] // Allow dead code for now, might be used later
    db_file_path: Arc<PathBuf>,
}

// Helper function to open/create the database connection
// This encapsulates the logic of setting flags and pragmas
fn open_db_connection(path: &Path) -> Result<Connection> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| OewnError::Io(e))?;
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
    /// Loads the WordNet data using default options (automatic database path).
    ///
    /// Ensures data is downloaded/extracted if needed.
    /// Opens/creates the database, initializes schema, and populates from XML if necessary.
    pub async fn load() -> Result<Self> {
        Self::load_with_options(LoadOptions::default()).await
    }

    /// Loads the WordNet data with specific options.
    pub async fn load_with_options(options: LoadOptions) -> Result<Self> {
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
            let lexicon_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM lexicons",
                [],
                |row| row.get(0),
            )?;
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
                 info!("Force reload requested. Clearing existing database data before population...");
                 // Use a transaction to clear data efficiently
                 let tx = conn.transaction()?;
                 db::clear_database_data(&tx)?;
                 tx.commit()?; // Commit the clearing transaction
                 info!("Existing data cleared.");
            } else {
                info!("Database needs population (first run or empty).");
            }

            // Ensure raw XML data file is present
            let xml_path = data::ensure_data().await?;
            info!("OEWN XML data available at: {:?}", xml_path);

            // Parse raw XML data
            info!("Reading and parsing XML file: {:?}", xml_path);
            let xml_content = tokio::fs::read_to_string(&xml_path).await?;
            let resource = parse_lmf(&xml_content).await?; // parse_lmf remains synchronous internally

            // Populate the database tables
            // populate_database handles its own transaction
            db::populate_database(&mut conn, resource)?;

        } else {
            info!("Using existing populated database: {:?}", db_path);
        }

        Ok(WordNet {
            conn: Arc::new(Mutex::new(conn)), // Wrap connection in Arc<Mutex>
            db_file_path: Arc::new(db_path),
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
                info!("Attempting to clear default database file: {:?}", default_path);
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

    /// Looks up lexical entries for a given lemma, optionally filtering by PartOfSpeech.
    /// Returns owned LexicalEntry structs fetched from the DB.
    pub fn lookup_entries(
        &self,
        lemma: &str,
        pos_filter: Option<PartOfSpeech>,
    ) -> Result<Vec<LexicalEntry>> {
        debug!("lookup_entries: lemma='{}', pos={:?}", lemma, pos_filter);
        // Use Internal error for Mutex poisoning
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard; // Dereference the guard

        let sql = "
            SELECT id, lemma_written_form, part_of_speech
            FROM lexical_entries
            WHERE lemma_written_form_lower = ?1 AND (?2 IS NULL OR part_of_speech = ?2)
        ";

        let pos_str_filter = pos_filter.map(db::part_of_speech_to_string);

        let mut stmt = conn.prepare(sql)?;
        let entry_ids_iter = stmt.query_map(
            params![lemma.to_lowercase(), pos_str_filter],
            |row| row.get::<_, String>(0), // Get just the ID
        )?;

        let mut entries = Vec::new();
        for entry_id_result in entry_ids_iter {
            let entry_id = entry_id_result?;
            // Fetch the full entry details using the ID
            match self.fetch_full_entry_by_id(conn, &entry_id)? {
                Some(entry) => entries.push(entry),
                None => warn!("Entry ID {} found in index but not fetchable.", entry_id),
            }
        }

        if entries.is_empty() {
            debug!(
                "No entries found for lemma '{}', pos_filter: {:?}",
                lemma, pos_filter
            );
        }
        Ok(entries)
    }


    /// Retrieves a specific Synset by its ID string.
    /// Returns an owned Synset struct fetched from the DB.
    pub fn get_synset(&self, id: &str) -> Result<Synset> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_full_synset_by_id(&*conn_guard, id)?
            .ok_or_else(|| OewnError::SynsetNotFound(id.to_string()))
    }

    /// Retrieves a specific Sense by its ID string.
    /// Returns an owned Sense struct fetched from the DB.
    pub fn get_sense(&self, id: &str) -> Result<Sense> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_full_sense_by_id(&*conn_guard, id)?
            .ok_or_else(|| OewnError::Internal(format!("Sense ID not found: {}", id))) // Should not happen if DB is consistent
    }

    /// Retrieves all Senses associated with a specific Lexical Entry ID.
    /// Returns owned Sense structs fetched from the DB.
    pub fn get_senses_for_entry(&self, entry_id: &str) -> Result<Vec<Sense>> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_senses_for_entry_internal(&*conn_guard, entry_id)
    }

    /// Retrieves all Senses associated with a specific Synset ID.
    /// Returns owned Sense structs fetched from the DB.
    pub fn get_senses_for_synset(&self, synset_id: &str) -> Result<Vec<Sense>> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let mut stmt = conn.prepare("SELECT id FROM senses WHERE synset_id = ?1")?;
        let sense_ids_iter = stmt.query_map(params![synset_id], |row| row.get::<_, String>(0))?;

        let mut senses = Vec::new();
        for sense_id_result in sense_ids_iter {
            let sense_id = sense_id_result?;
            match self.fetch_full_sense_by_id(conn, &sense_id)? {
                Some(sense) => senses.push(sense),
                None => warn!("Sense ID {} found for synset {} but not fetchable.", sense_id, synset_id),
            }
        }
        Ok(senses)
    }

    /// Retrieves a random lexical entry.
    /// Returns an owned LexicalEntry struct fetched from the DB.
    pub fn get_random_entry(&self) -> Result<LexicalEntry> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        // Get a random entry ID first
        let mut stmt_id = conn.prepare("SELECT id FROM lexical_entries ORDER BY RANDOM() LIMIT 1")?;
        let random_id_opt: Option<String> = stmt_id.query_row([], |row| row.get(0)).optional()?;

        match random_id_opt {
            Some(id) => self.fetch_full_entry_by_id(conn, &id)?
                .ok_or_else(|| OewnError::Internal(format!("Random entry ID {} not found.", id))), // Should not happen
            None => Err(OewnError::Internal("No entries found in database.".to_string())),
        }
    }

    /// Returns an iterator over all lexical entries in the WordNet data.
    /// Note: This is potentially very inefficient as it fetches *all* entries and their related data.
    /// Use with caution or consider alternative approaches like iterators if needed for large-scale processing.
    /// Returns owned LexicalEntry structs fetched from the DB.
    pub fn all_entries(&self) -> Result<Vec<LexicalEntry>> {
        warn!("all_entries() called: Fetching all entries from DB, this might be slow and memory-intensive.");
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let mut stmt = conn.prepare("SELECT id FROM lexical_entries")?;
        let entry_ids_iter = stmt.query_map([], |row| row.get::<_, String>(0))?;

        let mut entries = Vec::new();
        for entry_id_result in entry_ids_iter {
             let entry_id = entry_id_result?;
             match self.fetch_full_entry_by_id(conn, &entry_id)? {
                 Some(entry) => entries.push(entry),
                 None => warn!("Entry ID {} found in all_entries query but not fetchable.", entry_id),
             }
        }
        Ok(entries)
    }

    // --- Public Helper Methods ---

    /// Retrieves the entry ID for a given sense ID.
    pub fn get_entry_id_for_sense(&self, sense_id: &str) -> Result<Option<String>> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;
        let mut stmt = conn.prepare("SELECT entry_id FROM senses WHERE id = ?1")?;
        stmt.query_row(params![sense_id], |row| row.get(0)).optional().map_err(OewnError::from)
    }

    /// Retrieves an entry by its ID.
    /// Returns an owned LexicalEntry struct fetched from the DB.
    pub fn get_entry_by_id(&self, entry_id: &str) -> Result<Option<LexicalEntry>> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        self.fetch_full_entry_by_id(&*conn_guard, entry_id)
    }

    /// Internal helper to fetch full LexicalEntry data including relations.
    fn fetch_full_entry_by_id(&self, conn: &Connection, entry_id: &str) -> Result<Option<LexicalEntry>> {
        let mut stmt = conn.prepare(
            "SELECT id, lemma_written_form, part_of_speech FROM lexical_entries WHERE id = ?1",
        )?;
        let entry_opt = stmt.query_row(params![entry_id], |row| {
            // Explicitly map OewnError from helpers to rusqlite::Error within the closure
            let entry_id_str: String = row.get(0)?;
            // row_to_lemma already returns rusqlite::Error on failure
            let lemma = row_to_lemma(row)?;

            // Call helper and map error if it occurs
            let pronunciations = self.fetch_pronunciations_for_entry(conn, &entry_id_str)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type for clarity
            let senses = self.fetch_senses_for_entry_internal(conn, &entry_id_str)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type for clarity

            Ok(LexicalEntry {
                id: entry_id_str,
                lemma,
                pronunciations,
                senses,
                syntactic_behaviours: Vec::new(), // TODO: Fetch if needed
            })
        }).optional()?; // Use optional() to handle not found case

        Ok(entry_opt)
    }

    /// Internal helper to fetch pronunciations for a given entry ID.
    fn fetch_pronunciations_for_entry(&self, conn: &Connection, entry_id: &str) -> Result<Vec<Pronunciation>> {
        let mut stmt = conn.prepare(
            "SELECT variety, notation, phonemic, audio, text FROM pronunciations WHERE entry_id = ?1",
        )?;
        let pron_iter = stmt.query_map(params![entry_id], |row| {
            Ok(Pronunciation {
                variety: row.get(0)?,
                notation: row.get(1)?,
                phonemic: row.get::<_, i64>(2)? != 0, // Convert integer back to bool
                audio: row.get(3)?,
                text: row.get(4)?,
            })
        })?;

        pron_iter.collect::<std::result::Result<Vec<_>, _>>().map_err(OewnError::from)
    }

    /// Internal helper to fetch senses and their relations for a given entry ID.
    fn fetch_senses_for_entry_internal(&self, conn: &Connection, entry_id: &str) -> Result<Vec<Sense>> {
        let mut stmt = conn.prepare(
            "SELECT id, synset_id, subcat FROM senses WHERE entry_id = ?1",
        )?;
        let sense_iter = stmt.query_map(params![entry_id], |row| {
            let sense_id: String = row.get(0)?;
            let relations = self.fetch_sense_relations(conn, &sense_id)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type
            Ok(Sense {
                id: sense_id.clone(),
                synset: row.get(1)?,
                subcat: row.get(2)?,
                sense_relations: relations,
            })
        })?;

        sense_iter.collect::<std::result::Result<Vec<_>, _>>().map_err(OewnError::from)
    }

     /// Internal helper to fetch sense relations for a given sense ID.
    fn fetch_sense_relations(&self, conn: &Connection, sense_id: &str) -> Result<Vec<SenseRelation>> {
        let mut stmt = conn.prepare(
            "SELECT target_sense_id, rel_type FROM sense_relations WHERE source_sense_id = ?1",
        )?;
        let rel_iter = stmt.query_map(params![sense_id], |row| {
            let rel_type_str: String = row.get(1)?;
            let rel_type = string_to_sense_rel_type(&rel_type_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
            })?;
            Ok(SenseRelation {
                target: row.get(0)?,
                rel_type,
            })
        })?;

        rel_iter.collect::<std::result::Result<Vec<_>, _>>().map_err(OewnError::from)
    }


    /// Retrieves related Senses for a given source Sense ID and relation type.
    /// Returns owned Sense structs fetched from the DB.
    pub fn get_related_senses(
        &self,
        sense_id: &str,
        rel_type: SenseRelType,
    ) -> Result<Vec<Sense>> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let rel_type_str = db::sense_rel_type_to_string(rel_type); // Convert enum to string for query

        let mut stmt = conn.prepare(
            "SELECT target_sense_id FROM sense_relations WHERE source_sense_id = ?1 AND rel_type = ?2",
        )?;
        let target_ids_iter = stmt.query_map(params![sense_id, rel_type_str], |row| row.get::<_, String>(0))?;

        let mut senses = Vec::new();
        for target_id_result in target_ids_iter {
            let target_id = target_id_result?;
            match self.fetch_full_sense_by_id(conn, &target_id)? { // Use a new helper
                Some(sense) => senses.push(sense),
                None => warn!("Target sense ID {} not found for relation.", target_id),
            }
        }
        Ok(senses)
    }

     /// Internal helper to fetch full Sense data including relations.
    fn fetch_full_sense_by_id(&self, conn: &Connection, sense_id: &str) -> Result<Option<Sense>> {
        let mut stmt = conn.prepare(
            "SELECT id, entry_id, synset_id, subcat FROM senses WHERE id = ?1",
        )?;
        let sense_opt = stmt.query_row(params![sense_id], |row| {
            let sense_id_str : String = row.get(0)?;
             let relations = self.fetch_sense_relations(conn, &sense_id_str)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type
            Ok(Sense {
                id: sense_id_str,
                synset: row.get(2)?, // Get synset ID
                subcat: row.get(3)?,
                sense_relations: relations, // Fetch relations
                // Note: We don't store entry_id directly in the Sense struct model,
                // but it's available in the row if needed: row.get(1)?
            })
        }).optional()?;

        Ok(sense_opt)
    }


    /// Retrieves related Synsets for a given source Synset ID and relation type.
    /// Returns owned Synset structs fetched from the DB.
    pub fn get_related_synsets(
        &self,
        synset_id: &str,
        rel_type: SynsetRelType,
    ) -> Result<Vec<Synset>> {
        let conn_guard = self.conn.lock().map_err(|_| OewnError::Internal("Mutex poisoned".to_string()))?;
        let conn = &*conn_guard;

        let rel_type_str = db::synset_rel_type_to_string(rel_type); // Convert enum to string

        let mut stmt = conn.prepare(
            "SELECT target_synset_id FROM synset_relations WHERE source_synset_id = ?1 AND rel_type = ?2",
        )?;
        let target_ids_iter = stmt.query_map(params![synset_id, rel_type_str], |row| row.get::<_, String>(0))?;

        let mut synsets = Vec::new();
        for target_id_result in target_ids_iter {
            let target_id = target_id_result?;
            // Call the internal helper directly with the existing connection lock
            match self.fetch_full_synset_by_id(conn, &target_id)? {
                Some(synset) => synsets.push(synset), // fetch_full_synset_by_id returns Option<Synset>
                None => warn!("Target synset ID {} not found for relation.", target_id),
                // Errors from fetch_full_synset_by_id are propagated by ?
            }
        }
        Ok(synsets)
    }

    // --- Internal Helper Methods ---

    /// Internal helper to fetch full Synset data including relations, definitions, examples.
    fn fetch_full_synset_by_id(&self, conn: &Connection, synset_id: &str) -> Result<Option<Synset>> {
         let mut stmt = conn.prepare(
            "SELECT id, ili, part_of_speech FROM synsets WHERE id = ?1",
        )?;
        let synset_opt = stmt.query_row(params![synset_id], |row| {
            let id: String = row.get(0)?;
            let pos_str: String = row.get(2)?;
            // Explicitly map OewnError from helpers to rusqlite::Error within the closure
            let part_of_speech = string_to_part_of_speech(&pos_str)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?;
            let definitions = self.fetch_definitions_for_synset(conn, &id)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type
            let ili_definition = self.fetch_ili_definition_for_synset(conn, &id)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type
            let examples = self.fetch_examples_for_synset(conn, &id)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type
            let synset_relations = self.fetch_synset_relations(conn, &id)
                 .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?; // Use Text type

            Ok(Synset {
                id: id.clone(),
                ili: row.get(1)?,
                part_of_speech,
                definitions,
                ili_definition,
                examples,
                synset_relations,
                members: String::new(), // 'members' attribute from XML is not directly stored; derived from senses
            })
        }).optional()?;
        Ok(synset_opt)
    }

     /// Internal helper to fetch definitions for a given synset ID.
    fn fetch_definitions_for_synset(&self, conn: &Connection, synset_id: &str) -> Result<Vec<Definition>> {
        let mut stmt = conn.prepare(
            "SELECT text, dc_source FROM definitions WHERE synset_id = ?1",
        )?;
        let def_iter = stmt.query_map(params![synset_id], |row| {
            Ok(Definition {
                text: row.get(0)?,
                dc_source: row.get(1)?,
            })
        })?;
        def_iter.collect::<std::result::Result<Vec<_>, _>>().map_err(OewnError::from)
    }

     /// Internal helper to fetch the ILI definition for a given synset ID.
    fn fetch_ili_definition_for_synset(&self, conn: &Connection, synset_id: &str) -> Result<Option<ILIDefinition>> {
        let mut stmt = conn.prepare(
            "SELECT text, dc_source FROM ili_definitions WHERE synset_id = ?1",
        )?;
        stmt.query_row(params![synset_id], |row| {
            Ok(ILIDefinition {
                text: row.get(0)?,
                dc_source: row.get(1)?,
            })
        }).optional().map_err(OewnError::from)
    }

     /// Internal helper to fetch examples for a given synset ID.
    fn fetch_examples_for_synset(&self, conn: &Connection, synset_id: &str) -> Result<Vec<Example>> {
        let mut stmt = conn.prepare(
            "SELECT text, dc_source FROM examples WHERE synset_id = ?1",
        )?;
        let ex_iter = stmt.query_map(params![synset_id], |row| {
            Ok(Example {
                text: row.get(0)?,
                dc_source: row.get(1)?,
            })
        })?;
        ex_iter.collect::<std::result::Result<Vec<_>, _>>().map_err(OewnError::from)
    }

     /// Internal helper to fetch synset relations for a given synset ID.
    fn fetch_synset_relations(&self, conn: &Connection, synset_id: &str) -> Result<Vec<SynsetRelation>> {
        let mut stmt = conn.prepare(
            "SELECT target_synset_id, rel_type FROM synset_relations WHERE source_synset_id = ?1",
        )?;
        let rel_iter = stmt.query_map(params![synset_id], |row| {
            let rel_type_str: String = row.get(1)?;
            let rel_type = string_to_synset_rel_type(&rel_type_str).map_err(|e| {
                 rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
            })?;
            Ok(SynsetRelation {
                target: row.get(0)?,
                rel_type,
            })
        })?;
        rel_iter.collect::<std::result::Result<Vec<_>, _>>().map_err(OewnError::from)
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
    use std::path::PathBuf;
    use tempfile::tempdir;

    // Placeholder test function
    #[tokio::test]
    #[ignore] // Ignore until tests are rewritten
    async fn test_sqlite_loading_placeholder() {
        // Setup temp dir and db path
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_load.db");

        // Create dummy XML data (or use a small test fixture)
        let dummy_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
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
        let load_options = LoadOptions {
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
