//! Database management for Open English WordNet data.
//!
//! This module provides SQLite database schema definition, initialization, and data population
//! functionality for storing and querying WordNet lexical data.
//!
//! ## Database Schema
//!
//! The database consists of several interconnected tables:
//!
//! - `lexicons` - WordNet lexicon metadata
//! - `lexical_entries` - Word entries with lemmas and part-of-speech information
//! - `synsets` - Synonym sets (concepts) with ILI (Inter-Lingual Index) mappings
//! - `senses` - Links between lexical entries and synsets
//! - `definitions` - Textual definitions for synsets
//! - `examples` - Usage examples for synsets
//! - `pronunciations` - Pronunciation data for lexical entries
//! - `sense_relations` - Semantic relationships between senses
//! - `synset_relations` - Semantic relationships between synsets
//!
//! ## Usage
//!
//! ```rust,no_run
//! use oewn_rs::db::{initialize_database, populate_database};
//! use oewn_rs::progress::create_progress_channel;
//! use oewn_rs::models::LexicalResource;
//! use rusqlite::Connection;
//!
//! let mut conn = Connection::open("wordnet.db")?;
//! initialize_database(&mut conn)?;
//!
//! // After parsing WordNet XML data...
//! let lexical_resource = LexicalResource { lexicons: vec![] }; // minimal placeholder
//! let (progress_reporter, _progress_receiver) = create_progress_channel(100);
//! populate_database(&mut conn, lexical_resource, Some(progress_reporter))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::error::{OewnError, Result};
use crate::models::{LexicalResource, PartOfSpeech, SenseRelType, SynsetRelType};
use crate::progress::{ProgressReporter, ProgressUpdate, report_progress_non_blocking};
use log::{debug, info, warn};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::time::Instant;

fn report_progress(reporter: &Option<ProgressReporter>, update: ProgressUpdate) {
    if let Some(reporter) = reporter {
        report_progress_non_blocking(reporter, update);
    }
}

const SCHEMA_VERSION: u32 = 1;

const CREATE_METADATA_TABLE: &str = "
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);";

const CREATE_LEXICONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS lexicons (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    language TEXT NOT NULL,
    email TEXT NOT NULL,
    license TEXT NOT NULL,
    version TEXT NOT NULL,
    url TEXT,
    citation TEXT,
    logo TEXT,
    status TEXT,
    confidence_score REAL,
    dc_publisher TEXT,
    dc_contributor TEXT
);";

const CREATE_LEXICAL_ENTRIES_TABLE: &str = "
CREATE TABLE IF NOT EXISTS lexical_entries (
    id TEXT PRIMARY KEY,
    lexicon_id TEXT NOT NULL,
    lemma_written_form TEXT NOT NULL,
    lemma_written_form_lower TEXT NOT NULL, -- For case-insensitive search
    part_of_speech TEXT NOT NULL, -- Stored as TEXT (e.g., 'n', 'v')
    FOREIGN KEY (lexicon_id) REFERENCES lexicons(id)
);";

const CREATE_PRONUNCIATIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS pronunciations (
    entry_id TEXT NOT NULL,
    variety TEXT NOT NULL,
    notation TEXT,
    phonemic INTEGER NOT NULL, -- 0 for false, 1 for true
    audio TEXT,
    text TEXT NOT NULL,
    FOREIGN KEY (entry_id) REFERENCES lexical_entries(id)
);";

const CREATE_SYNSETS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS synsets (
    id TEXT PRIMARY KEY,
    lexicon_id TEXT NOT NULL,
    ili TEXT,
    part_of_speech TEXT NOT NULL,
    -- 'members' from XML is implicitly handled by senses.entry_id + senses.synset_id
    FOREIGN KEY (lexicon_id) REFERENCES lexicons(id)
);";

const CREATE_SENSES_TABLE: &str = "
CREATE TABLE IF NOT EXISTS senses (
    id TEXT PRIMARY KEY,
    entry_id TEXT NOT NULL,
    synset_id TEXT NOT NULL,
    FOREIGN KEY (entry_id) REFERENCES lexical_entries(id),
    FOREIGN KEY (synset_id) REFERENCES synsets(id)
);";

const CREATE_DEFINITIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS definitions (
    synset_id TEXT NOT NULL,
    text TEXT NOT NULL,
    dc_source TEXT,
    FOREIGN KEY (synset_id) REFERENCES synsets(id)
);";

const CREATE_ILI_DEFINITIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS ili_definitions (
    synset_id TEXT PRIMARY KEY, -- Assuming one ILI def per synset
    text TEXT NOT NULL,
    dc_source TEXT,
    FOREIGN KEY (synset_id) REFERENCES synsets(id)
);";

const CREATE_EXAMPLES_TABLE: &str = "
CREATE TABLE IF NOT EXISTS examples (
    synset_id TEXT NOT NULL,
    text TEXT NOT NULL,
    dc_source TEXT,
    FOREIGN KEY (synset_id) REFERENCES synsets(id)
);";

const CREATE_SENSE_RELATIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS sense_relations (
    source_sense_id TEXT NOT NULL,
    target_sense_id TEXT NOT NULL,
    rel_type TEXT NOT NULL, -- Stored as TEXT (e.g., 'antonym')
    PRIMARY KEY (source_sense_id, target_sense_id, rel_type),
    FOREIGN KEY (source_sense_id) REFERENCES senses(id),
    FOREIGN KEY (target_sense_id) REFERENCES senses(id)
);";

const CREATE_SYNSET_RELATIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS synset_relations (
    source_synset_id TEXT NOT NULL,
    target_synset_id TEXT NOT NULL,
    rel_type TEXT NOT NULL, -- Stored as TEXT (e.g., 'hypernym')
    PRIMARY KEY (source_synset_id, target_synset_id, rel_type),
    FOREIGN KEY (source_synset_id) REFERENCES synsets(id),
    FOREIGN KEY (target_synset_id) REFERENCES synsets(id)
);";

// Database performance optimization indices
macro_rules! create_index {
    ($name:ident, $index_name:expr, $table:expr, $columns:expr) => {
        const $name: &str = concat!(
            "CREATE INDEX IF NOT EXISTS ",
            $index_name,
            " ON ",
            $table,
            " (",
            $columns,
            ");"
        );
    };
}

create_index!(
    CREATE_ENTRY_LEMMA_LOWER_INDEX,
    "idx_entry_lemma_lower",
    "lexical_entries",
    "lemma_written_form_lower"
);
create_index!(
    CREATE_ENTRY_POS_INDEX,
    "idx_entry_pos",
    "lexical_entries",
    "part_of_speech"
);
create_index!(
    CREATE_ENTRY_LEMMA_POS_INDEX,
    "idx_entry_lemma_pos",
    "lexical_entries",
    "lemma_written_form_lower, part_of_speech"
);
create_index!(
    CREATE_SENSE_SYNSET_INDEX,
    "idx_sense_synset",
    "senses",
    "synset_id"
);
create_index!(
    CREATE_SENSE_ENTRY_INDEX,
    "idx_sense_entry",
    "senses",
    "entry_id"
);
create_index!(
    CREATE_SENSE_REL_SOURCE_TYPE_INDEX,
    "idx_sense_rel_source_type",
    "sense_relations",
    "source_sense_id, rel_type"
);
create_index!(
    CREATE_SYNSET_REL_SOURCE_TYPE_INDEX,
    "idx_synset_rel_source_type",
    "synset_relations",
    "source_synset_id, rel_type"
);
create_index!(
    CREATE_DEFINITION_SYNSET_INDEX,
    "idx_definition_synset",
    "definitions",
    "synset_id"
);
create_index!(
    CREATE_EXAMPLE_SYNSET_INDEX,
    "idx_example_synset",
    "examples",
    "synset_id"
);
create_index!(
    CREATE_PRONUNCIATION_ENTRY_INDEX,
    "idx_pronunciation_entry",
    "pronunciations",
    "entry_id"
);

/// Creates all necessary tables and indices in the database if they don't exist.
/// Also checks and sets the schema version.
pub fn initialize_database(conn: &mut Connection) -> Result<()> {
    info!(
        "Initializing database schema (version {})...",
        SCHEMA_VERSION
    );
    let tx = conn.transaction()?;

    // Create tables
    tx.execute(CREATE_METADATA_TABLE, [])?;
    tx.execute(CREATE_LEXICONS_TABLE, [])?;
    tx.execute(CREATE_LEXICAL_ENTRIES_TABLE, [])?;
    tx.execute(CREATE_PRONUNCIATIONS_TABLE, [])?;
    tx.execute(CREATE_SYNSETS_TABLE, [])?;
    tx.execute(CREATE_SENSES_TABLE, [])?;
    tx.execute(CREATE_DEFINITIONS_TABLE, [])?;
    tx.execute(CREATE_ILI_DEFINITIONS_TABLE, [])?;
    tx.execute(CREATE_EXAMPLES_TABLE, [])?;
    tx.execute(CREATE_SENSE_RELATIONS_TABLE, [])?;
    tx.execute(CREATE_SYNSET_RELATIONS_TABLE, [])?;

    // Create indices
    tx.execute(CREATE_ENTRY_LEMMA_LOWER_INDEX, [])?;
    tx.execute(CREATE_ENTRY_POS_INDEX, [])?;
    tx.execute(CREATE_ENTRY_LEMMA_POS_INDEX, [])?;
    tx.execute(CREATE_SENSE_SYNSET_INDEX, [])?;
    tx.execute(CREATE_SENSE_ENTRY_INDEX, [])?;
    tx.execute(CREATE_SENSE_REL_SOURCE_TYPE_INDEX, [])?;
    tx.execute(CREATE_SYNSET_REL_SOURCE_TYPE_INDEX, [])?;
    tx.execute(CREATE_DEFINITION_SYNSET_INDEX, [])?;
    tx.execute(CREATE_EXAMPLE_SYNSET_INDEX, [])?;
    tx.execute(CREATE_PRONUNCIATION_ENTRY_INDEX, [])?;

    // Check schema version
    let existing_version_str: Option<String> = tx
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    match existing_version_str {
        Some(v_str) => {
            let existing_version: u32 = v_str.parse().map_err(|e| {
                OewnError::ParseError(format!(
                    "Failed to parse existing schema version '{}': {}",
                    v_str, e
                ))
            })?;
            match existing_version.cmp(&SCHEMA_VERSION) {
                std::cmp::Ordering::Less => {
                    warn!(
                        "Database schema version ({}) is older than expected ({}). Migration needed.",
                        existing_version, SCHEMA_VERSION
                    );
                    // For now, just update the version
                    tx.execute(
                        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
                        params![SCHEMA_VERSION.to_string()],
                    )?;
                    info!("Updated schema version in metadata table.");
                }
                std::cmp::Ordering::Greater => {
                    warn!(
                        "Database schema version ({}) is newer than expected ({}). Using potentially incompatible schema.",
                        existing_version, SCHEMA_VERSION
                    );
                }
                std::cmp::Ordering::Equal => {
                    debug!(
                        "Database schema version ({}) matches expected version.",
                        existing_version
                    );
                }
            }
        }
        None => {
            // No version found, insert current version
            tx.execute(
                "INSERT INTO metadata (key, value) VALUES ('schema_version', ?1)",
                params![SCHEMA_VERSION.to_string()],
            )?;
            info!("Set initial schema version in metadata table.");
        }
    }

    tx.commit()?;
    info!("Database schema initialization complete.");
    Ok(())
}

// --- Data Population Function ---

/// Populates the database with WordNet data from a parsed LexicalResource.
///
/// This function performs a comprehensive three-pass insertion strategy to handle
/// foreign key dependencies correctly:
///
/// 1. **Pass 1**: Insert core entities (lexicons, lexical entries, synsets)
/// 2. **Pass 2**: Insert detail entities (pronunciations, senses, definitions, examples)
/// 3. **Pass 3**: Insert relationship entities (sense relations, synset relations)
///
/// The operation is performed within a single transaction for data integrity and
/// uses prepared statements for optimal performance.
///
/// # Arguments
///
/// * `conn` - Mutable reference to the SQLite database connection
/// * `resource` - The parsed WordNet lexical resource to insert
/// * `reporter` - Thread-safe progress reporting callback for UI updates
///
/// # Returns
///
/// * `Result<()>` - Success or database error
///
/// # Examples
///
/// ```rust,no_run
/// use oewn_rs::db::populate_database;
/// use oewn_rs::progress::create_progress_channel;
/// use oewn_rs::models::LexicalResource;
/// use rusqlite::Connection;
///
/// # tokio_test::block_on(async {
/// let mut conn = Connection::open(":memory:")?;
/// let lexical_resource = LexicalResource { lexicons: vec![] }; // minimal placeholder
/// let (progress_tx, _progress_rx) = create_progress_channel(100);
/// populate_database(&mut conn, lexical_resource, Some(progress_tx))?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub fn populate_database(
    conn: &mut Connection,
    resource: LexicalResource,
    reporter: Option<ProgressReporter>,
) -> Result<()> {
    info!("Populating database from parsed LexicalResource using prepared statements...");
    let start_time = Instant::now();

    // --- Calculate Totals for Progress Reporting ---
    let total_lexicons = resource.lexicons.len() as u64;
    let total_entries = resource
        .lexicons
        .iter()
        .map(|l| l.lexical_entries.len())
        .sum::<usize>() as u64;
    let total_synsets = resource
        .lexicons
        .iter()
        .map(|l| l.synsets.len())
        .sum::<usize>() as u64;
    let total_pronunciations = resource
        .lexicons
        .iter()
        .flat_map(|l| &l.lexical_entries)
        .map(|e| e.pronunciations.len())
        .sum::<usize>() as u64;
    let total_senses = resource
        .lexicons
        .iter()
        .flat_map(|l| &l.lexical_entries)
        .map(|e| e.senses.len())
        .sum::<usize>() as u64;
    let total_definitions = resource
        .lexicons
        .iter()
        .flat_map(|l| &l.synsets)
        .map(|s| s.definitions.len())
        .sum::<usize>() as u64;
    let total_ili_definitions = resource
        .lexicons
        .iter()
        .flat_map(|l| &l.synsets)
        .filter(|s| s.ili_definition.is_some())
        .count() as u64;
    let total_examples = resource
        .lexicons
        .iter()
        .flat_map(|l| &l.synsets)
        .map(|s| s.examples.len())
        .sum::<usize>() as u64;
    let total_sense_relations = resource
        .lexicons
        .iter()
        .flat_map(|l| &l.lexical_entries)
        .flat_map(|e| &e.senses)
        .map(|s| s.sense_relations.len())
        .sum::<usize>() as u64;
    let total_synset_relations = resource
        .lexicons
        .iter()
        .flat_map(|l| &l.synsets)
        .map(|s| s.synset_relations.len())
        .sum::<usize>() as u64;

    let pass1_total = total_lexicons + total_entries + total_synsets;
    let pass2_total = total_pronunciations
        + total_senses
        + total_definitions
        + total_ili_definitions
        + total_examples;
    let pass3_total = total_sense_relations + total_synset_relations;

    // Helper closure to report progress
    let maybe_report = |update: ProgressUpdate| {
        report_progress(&reporter, update);
    };

    let tx = conn.transaction()?;

    // --- Prepare Statements ---
    let mut lexicon_stmt = tx.prepare(
        "INSERT INTO lexicons (id, label, language, email, license, version, url, citation, logo, status, confidence_score, dc_publisher, dc_contributor)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )?;
    let mut entry_stmt = tx.prepare(
        "INSERT INTO lexical_entries (id, lexicon_id, lemma_written_form, lemma_written_form_lower, part_of_speech)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut synset_stmt = tx.prepare(
        "INSERT INTO synsets (id, lexicon_id, ili, part_of_speech)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut pron_stmt = tx.prepare(
        "INSERT INTO pronunciations (entry_id, variety, notation, phonemic, audio, text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut sense_stmt = tx.prepare(
        "INSERT INTO senses (id, entry_id, synset_id)
         VALUES (?1, ?2, ?3)",
    )?;
    let mut def_stmt = tx.prepare(
        "INSERT INTO definitions (synset_id, text, dc_source)
         VALUES (?1, ?2, ?3)",
    )?;
    let mut ili_def_stmt = tx.prepare(
        "INSERT INTO ili_definitions (synset_id, text, dc_source)
         VALUES (?1, ?2, ?3)",
    )?;
    let mut example_stmt = tx.prepare(
        "INSERT INTO examples (synset_id, text, dc_source)
         VALUES (?1, ?2, ?3)",
    )?;
    let mut sense_rel_stmt = tx.prepare(
        "INSERT OR IGNORE INTO sense_relations (source_sense_id, target_sense_id, rel_type)
         VALUES (?1, ?2, ?3)",
    )?;
    let mut synset_rel_stmt = tx.prepare(
        "INSERT OR IGNORE INTO synset_relations (source_synset_id, target_synset_id, rel_type)
         VALUES (?1, ?2, ?3)",
    )?;

    // --- Pass 1: Insert core entities (Lexicons, Entries, Synsets) ---
    info!("Population Pass 1: Inserting Lexicons, LexicalEntries, Synsets...");
    maybe_report(ProgressUpdate::new(
        "Pass 1/3: Inserting Core Entities".to_string(),
        0,
        Some(pass1_total),
        None,
    ));
    let mut pass1_current = 0;

    for lexicon in &resource.lexicons {
        debug!("Pass 1: Inserting lexicon: {}", lexicon.id);
        lexicon_stmt.execute(params![
            lexicon.id,
            lexicon.label,
            lexicon.language,
            lexicon.email,
            lexicon.license,
            lexicon.version,
            lexicon.url,
            lexicon.citation,
            lexicon.logo,
            lexicon.status,
            lexicon.confidence_score,
            lexicon.dc_publisher,
            lexicon.dc_contributor,
        ])?;
        pass1_current += 1;
        maybe_report(ProgressUpdate {
            stage_description: "Pass 1/3: Inserting Core Entities".to_string(),
            current_item: pass1_current,
            total_items: Some(pass1_total),
            message: Some(format!("Lexicon: {}", lexicon.id)),
        });

        for entry in &lexicon.lexical_entries {
            entry_stmt.execute(params![
                entry.id,
                lexicon.id, // Foreign key
                entry.lemma.written_form,
                entry.lemma.written_form.to_lowercase(), // Store lowercase version
                part_of_speech_to_string(entry.lemma.part_of_speech), // Store POS as string
            ])?;
            pass1_current += 1;
            maybe_report(ProgressUpdate {
                stage_description: "Pass 1/3: Inserting Core Entities".to_string(),
                current_item: pass1_current,
                total_items: Some(pass1_total),
                message: Some(format!("Entry: {}", entry.id)),
            });
        }

        for synset in &lexicon.synsets {
            synset_stmt.execute(params![
                synset.id,
                lexicon.id, // Foreign key
                synset.ili,
                part_of_speech_to_string(synset.part_of_speech), // Store POS as string
            ])?;
            pass1_current += 1;
            maybe_report(ProgressUpdate {
                stage_description: "Pass 1/3: Inserting Core Entities".to_string(),
                current_item: pass1_current,
                total_items: Some(pass1_total),
                message: Some(format!("Synset: {}", synset.id)),
            });
        }
    }
    info!("Pass 1 complete.");

    // --- Pass 2: Insert entities referencing core entities (Senses, Definitions, Examples, Pronunciations) ---
    info!("Population Pass 2: Inserting Senses, Definitions, Examples, Pronunciations...");
    maybe_report(ProgressUpdate::new(
        "Pass 2/3: Inserting Details".to_string(),
        0,
        Some(pass2_total),
        None,
    ));
    let mut pass2_current = 0;

    for lexicon in &resource.lexicons {
        for entry in &lexicon.lexical_entries {
            for pron in &entry.pronunciations {
                pron_stmt.execute(params![
                    entry.id, // Foreign key
                    pron.variety,
                    pron.notation,
                    pron.phonemic, // Store bool as integer
                    pron.audio,
                    pron.text,
                ])?;
                pass2_current += 1;
                maybe_report(ProgressUpdate {
                    stage_description: "Pass 2/3: Inserting Details".to_string(),
                    current_item: pass2_current,
                    total_items: Some(pass2_total),
                    message: Some(format!("Pronunciation for Entry: {}", entry.id)),
                });
            }

            for sense in &entry.senses {
                sense_stmt.execute(params![
                    sense.id,
                    entry.id,     // Foreign key
                    sense.synset, // Foreign key (references synset.id)
                ])?;
                pass2_current += 1;
                maybe_report(ProgressUpdate {
                    stage_description: "Pass 2/3: Inserting Details".to_string(),
                    current_item: pass2_current,
                    total_items: Some(pass2_total),
                    message: Some(format!("Sense: {}", sense.id)),
                });
            }
        }

        for synset in &lexicon.synsets {
            for definition in &synset.definitions {
                def_stmt.execute(params![
                    synset.id, // Foreign key
                    definition.text,
                    definition.dc_source,
                ])?;
                pass2_current += 1;
                maybe_report(ProgressUpdate {
                    stage_description: "Pass 2/3: Inserting Details".to_string(),
                    current_item: pass2_current,
                    total_items: Some(pass2_total),
                    message: Some(format!("Definition for Synset: {}", synset.id)),
                });
            }

            if let Some(ili_def) = &synset.ili_definition {
                ili_def_stmt.execute(params![
                    synset.id, // Primary key
                    ili_def.text,
                    ili_def.dc_source,
                ])?;
                pass2_current += 1;
                maybe_report(ProgressUpdate {
                    stage_description: "Pass 2/3: Inserting Details".to_string(),
                    current_item: pass2_current,
                    total_items: Some(pass2_total),
                    message: Some(format!("ILI Definition for Synset: {}", synset.id)),
                });
            }

            for example in &synset.examples {
                example_stmt.execute(params![
                    synset.id, // Foreign key
                    example.text,
                    example.dc_source,
                ])?;
                pass2_current += 1;
                maybe_report(ProgressUpdate {
                    stage_description: "Pass 2/3: Inserting Details".to_string(),
                    current_item: pass2_current,
                    total_items: Some(pass2_total),
                    message: Some(format!("Example for Synset: {}", synset.id)),
                });
            }
        }
    }
    info!("Pass 2 complete.");

    // --- Pass 3: Insert relations (SenseRelations, SynsetRelations) ---
    info!("Population Pass 3: Inserting SenseRelations, SynsetRelations...");
    maybe_report(ProgressUpdate::new(
        "Pass 3/3: Inserting Relations".to_string(),
        0,
        Some(pass3_total),
        None,
    ));
    let mut pass3_current = 0;

    for lexicon in &resource.lexicons {
        for entry in &lexicon.lexical_entries {
            for sense in &entry.senses {
                for relation in &sense.sense_relations {
                    sense_rel_stmt.execute(params![
                        sense.id,                                    // Source sense
                        relation.target,                             // Target sense ID
                        sense_rel_type_to_string(relation.rel_type), // Store type as string
                    ])?;
                    pass3_current += 1;
                    maybe_report(ProgressUpdate {
                        stage_description: "Pass 3/3: Inserting Relations".to_string(),
                        current_item: pass3_current,
                        total_items: Some(pass3_total),
                        message: Some(format!("Sense Relation from: {}", sense.id)),
                    });
                }
            }
        }
        for synset in &lexicon.synsets {
            for relation in &synset.synset_relations {
                synset_rel_stmt.execute(params![
                    synset.id,                                    // Source synset
                    relation.target,                              // Target synset ID
                    synset_rel_type_to_string(relation.rel_type), // Store type as string
                ])?;
                pass3_current += 1;
                maybe_report(ProgressUpdate {
                    stage_description: "Pass 3/3: Inserting Relations".to_string(),
                    current_item: pass3_current,
                    total_items: Some(pass3_total),
                    message: Some(format!("Synset Relation from: {}", synset.id)),
                });
            }
        }
    }
    info!("Pass 3 complete.");

    // Drop statements explicitly before committing (optional, but good practice)
    drop(lexicon_stmt);
    drop(entry_stmt);
    drop(synset_stmt);
    drop(pron_stmt);
    drop(sense_stmt);
    drop(def_stmt);
    drop(ili_def_stmt);
    drop(example_stmt);
    drop(sense_rel_stmt);
    drop(synset_rel_stmt);

    tx.commit()?; // Commit the transaction

    info!(
        "Database population complete. Took {:.2?}",
        start_time.elapsed()
    );
    Ok(())
}

/// Clears all WordNet data from the database tables while preserving metadata.
///
/// This function removes all lexical data from the database in the correct order
/// to respect foreign key constraints. The metadata table (including schema version)
/// is preserved to maintain database state information.
///
/// # Arguments
///
/// * `tx` - Database transaction to perform the deletions within
///
/// # Returns
///
/// * `Result<()>` - Success or database error
///
/// # Note
///
/// This function must be called within an existing transaction. The caller is
/// responsible for committing or rolling back the transaction.
pub fn clear_database_data(tx: &Transaction) -> Result<()> {
    info!("Clearing existing data from database tables...");
    // Order matters due to foreign key constraints (delete from referencing tables first)
    tx.execute("DELETE FROM sense_relations", [])?;
    tx.execute("DELETE FROM synset_relations", [])?;
    tx.execute("DELETE FROM definitions", [])?;
    tx.execute("DELETE FROM ili_definitions", [])?;
    tx.execute("DELETE FROM examples", [])?;
    tx.execute("DELETE FROM pronunciations", [])?;
    tx.execute("DELETE FROM senses", [])?;
    tx.execute("DELETE FROM synsets", [])?;
    tx.execute("DELETE FROM lexical_entries", [])?;
    tx.execute("DELETE FROM lexicons", [])?;
    // Don't delete from metadata table
    info!("Finished clearing data.");
    Ok(())
}

/// Converts a PartOfSpeech enum to its string representation for database storage.
///
/// # Arguments
///
/// * `pos` - The part of speech enum value
///
/// # Returns
///
/// * `&'static str` - Single character string representation (e.g., "n" for noun)
pub(crate) fn part_of_speech_to_string(pos: PartOfSpeech) -> &'static str {
    match pos {
        PartOfSpeech::N => "n",
        PartOfSpeech::V => "v",
        PartOfSpeech::A => "a",
        PartOfSpeech::R => "r",
        PartOfSpeech::S => "s",
        PartOfSpeech::C => "c",
        PartOfSpeech::P => "p",
        PartOfSpeech::X => "x",
        PartOfSpeech::U => "u",
    }
}

/// Converts a string representation back to a PartOfSpeech enum.
///
/// # Arguments
///
/// * `s` - String representation of part of speech
///
/// # Returns
///
/// * `Result<PartOfSpeech>` - Parsed enum value or parse error
pub fn string_to_part_of_speech(s: &str) -> Result<PartOfSpeech> {
    match s {
        "n" => Ok(PartOfSpeech::N),
        "v" => Ok(PartOfSpeech::V),
        "a" => Ok(PartOfSpeech::A),
        "r" => Ok(PartOfSpeech::R),
        "s" => Ok(PartOfSpeech::S),
        "c" => Ok(PartOfSpeech::C),
        "p" => Ok(PartOfSpeech::P),
        "x" => Ok(PartOfSpeech::X),
        "u" => Ok(PartOfSpeech::U),
        _ => Err(OewnError::ParseError(format!(
            "Invalid PartOfSpeech string in DB: {}",
            s
        ))),
    }
}

/// Converts a SenseRelType enum to its string representation for database storage.
///
/// # Arguments
///
/// * `rel_type` - The sense relation type enum value
///
/// # Returns
///
/// * `String` - Lowercase string representation of the relation type
pub(crate) fn sense_rel_type_to_string(rel_type: SenseRelType) -> String {
    // Use serde_plain for simple enum-string mapping if preferred,
    // or implement manually like this:
    format!("{:?}", rel_type).to_lowercase() // Simple debug representation to lowercase
}

/// Converts a string representation back to a SenseRelType enum.
///
/// # Arguments
///
/// * `s` - String representation of the sense relation type
///
/// # Returns
///
/// * `Result<SenseRelType>` - Parsed enum value, defaulting to `Other` for unknown types
pub fn string_to_sense_rel_type(s: &str) -> Result<SenseRelType> {
    // This requires a more robust mapping, potentially using serde or match
    match s {
        "antonym" => Ok(SenseRelType::Antonym),
        "also" => Ok(SenseRelType::Also),
        "participle" => Ok(SenseRelType::Participle),
        "pertainym" => Ok(SenseRelType::Pertainym),
        "derivation" => Ok(SenseRelType::Derivation),
        "domain_topic" => Ok(SenseRelType::DomainTopic),
        "domain_member_topic" => Ok(SenseRelType::DomainMemberTopic),
        "domain_region" => Ok(SenseRelType::DomainRegion),
        "domain_member_region" => Ok(SenseRelType::DomainMemberRegion),
        "exemplifies" => Ok(SenseRelType::Exemplifies),
        "is_exemplified_by" => Ok(SenseRelType::IsExemplifiedBy),
        _ => Ok(SenseRelType::Other), // Default to Other for unknown/new types
    }
}

/// Converts a SynsetRelType enum to its string representation for database storage.
///
/// # Arguments
///
/// * `rel_type` - The synset relation type enum value
///
/// # Returns
///
/// * `String` - Lowercase string representation of the relation type
pub(crate) fn synset_rel_type_to_string(rel_type: SynsetRelType) -> String {
    format!("{:?}", rel_type).to_lowercase() // Simple debug representation to lowercase
}

/// Converts a string representation back to a SynsetRelType enum.
///
/// # Arguments
///
/// * `s` - String representation of the synset relation type
///
/// # Returns
///
/// * `Result<SynsetRelType>` - Parsed enum value, defaulting to `Unknown` for unrecognized types
pub fn string_to_synset_rel_type(s: &str) -> Result<SynsetRelType> {
    // This needs a comprehensive match or serde mapping
    match s {
        "hypernym" => Ok(SynsetRelType::Hypernym),
        "hyponym" => Ok(SynsetRelType::Hyponym),
        "instance_hypernym" => Ok(SynsetRelType::InstanceHypernym),
        "instance_hyponym" => Ok(SynsetRelType::InstanceHyponym),
        "mero_member" => Ok(SynsetRelType::MeroMember),
        "mero_part" => Ok(SynsetRelType::MeroPart),
        "mero_substance" => Ok(SynsetRelType::MeroSubstance),
        "holo_member" => Ok(SynsetRelType::HoloMember),
        "holo_part" => Ok(SynsetRelType::HoloPart),
        "holo_substance" => Ok(SynsetRelType::HoloSubstance),
        "entails" => Ok(SynsetRelType::Entails),
        "causes" => Ok(SynsetRelType::Causes),
        "similar" => Ok(SynsetRelType::Similar),
        "attribute" => Ok(SynsetRelType::Attribute),
        "domain_region" => Ok(SynsetRelType::DomainRegion),
        "domain_topic" => Ok(SynsetRelType::DomainTopic),
        "has_domain_region" => Ok(SynsetRelType::HasDomainRegion),
        "has_domain_topic" => Ok(SynsetRelType::HasDomainTopic),
        "exemplifies" => Ok(SynsetRelType::Exemplifies),
        "is_exemplified_by" => Ok(SynsetRelType::IsExemplifiedBy),
        _ => Ok(SynsetRelType::Unknown), // Default to Unknown
    }
}
