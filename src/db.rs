use crate::error::{OewnError, Result};
use crate::models::{LexicalResource, PartOfSpeech, SenseRelType, SynsetRelType};
use log::{debug, info, warn};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::time::Instant;

// --- Schema Definition ---

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
    subcat TEXT, -- Reference to SyntacticBehaviour ID (if used)
    FOREIGN KEY (entry_id) REFERENCES lexical_entries(id),
    FOREIGN KEY (synset_id) REFERENCES synsets(id)
    -- FOREIGN KEY (subcat) REFERENCES syntactic_behaviours(id) -- Add if behaviours are stored
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

// --- Indices ---

const CREATE_ENTRY_LEMMA_LOWER_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_entry_lemma_lower ON lexical_entries (lemma_written_form_lower);";
const CREATE_ENTRY_POS_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_entry_pos ON lexical_entries (part_of_speech);";
const CREATE_ENTRY_LEMMA_POS_INDEX: &str = "CREATE INDEX IF NOT EXISTS idx_entry_lemma_pos ON lexical_entries (lemma_written_form_lower, part_of_speech);";
const CREATE_SENSE_SYNSET_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_sense_synset ON senses (synset_id);";
const CREATE_SENSE_ENTRY_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_sense_entry ON senses (entry_id);";
const CREATE_SENSE_REL_SOURCE_TYPE_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_sense_rel_source_type ON sense_relations (source_sense_id, rel_type);";
const CREATE_SYNSET_REL_SOURCE_TYPE_INDEX: &str = "CREATE INDEX IF NOT EXISTS idx_synset_rel_source_type ON synset_relations (source_synset_id, rel_type);";
const CREATE_DEFINITION_SYNSET_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_definition_synset ON definitions (synset_id);";
const CREATE_EXAMPLE_SYNSET_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_example_synset ON examples (synset_id);";
const CREATE_PRONUNCIATION_ENTRY_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_pronunciation_entry ON pronunciations (entry_id);";

// --- Initialization Function ---

/// Creates all necessary tables and indices in the database if they don't exist.
/// Also checks and sets the schema version.
pub fn initialize_database(conn: &mut Connection) -> Result<()> {
    info!("Initializing database schema (version {})...", SCHEMA_VERSION);
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
    // tx.execute(CREATE_SYNTACTIC_BEHAVIOURS_TABLE, [])?; // Uncomment if needed

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
                // Use ParseError for parsing issues
                OewnError::ParseError(format!("Failed to parse existing schema version '{}': {}", v_str, e))
            })?;
            if existing_version < SCHEMA_VERSION {
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
            } else if existing_version > SCHEMA_VERSION {
                 warn!(
                    "Database schema version ({}) is newer than expected ({}). Using potentially incompatible schema.",
                    existing_version, SCHEMA_VERSION
                );
            } else {
                debug!("Database schema version ({}) matches expected version.", existing_version);
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

/// Populates the database tables from a parsed LexicalResource.
/// Assumes the database is empty or should be overwritten.
/// Uses a transaction for efficiency.
pub fn populate_database(conn: &mut Connection, resource: LexicalResource) -> Result<()> {
    info!("Populating database from parsed LexicalResource...");
    let start_time = Instant::now();

    // Clear existing data (optional, depends on strategy)
    // clear_database_data(conn)?; // Call a helper to DELETE FROM all tables if needed

    let tx = conn.transaction()?;

    // --- Pass 1: Insert core entities (Lexicons, Entries, Synsets) ---
    info!("Population Pass 1: Inserting Lexicons, LexicalEntries, Synsets...");
    for lexicon in &resource.lexicons { // Iterate by reference first
        debug!("Pass 1: Inserting lexicon: {}", lexicon.id);
        tx.execute(
            "INSERT INTO lexicons (id, label, language, email, license, version, url, citation, logo, status, confidence_score, dc_publisher, dc_contributor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
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
            ],
        )?;

        for entry in &lexicon.lexical_entries { // Iterate by reference
            tx.execute(
                "INSERT INTO lexical_entries (id, lexicon_id, lemma_written_form, lemma_written_form_lower, part_of_speech)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    entry.id,
                    lexicon.id, // Foreign key
                    entry.lemma.written_form,
                    entry.lemma.written_form.to_lowercase(), // Store lowercase version
                    part_of_speech_to_string(entry.lemma.part_of_speech), // Store POS as string
                ],
            )?;
            // Defer pronunciation insertion to Pass 2
        }

        for synset in &lexicon.synsets { // Iterate by reference
            tx.execute(
                "INSERT INTO synsets (id, lexicon_id, ili, part_of_speech)
                 VALUES (?1, ?2, ?3, ?4)",
                // Correct params! macro for synsets table
                params![
                    synset.id,
                    lexicon.id, // Foreign key
                    synset.ili,
                    part_of_speech_to_string(synset.part_of_speech), // Store POS as string
                ],
            )?;
            // Defer definition, ili_definition, example, synset_relation insertion to later passes
        }
    }
    info!("Pass 1 complete.");

    // --- Pass 2: Insert entities referencing core entities (Senses, Definitions, Examples, Pronunciations) ---
    info!("Population Pass 2: Inserting Senses, Definitions, Examples, Pronunciations...");
    for lexicon in &resource.lexicons { // Iterate again
        for entry in &lexicon.lexical_entries {
             // Insert Pronunciations
             for pron in &entry.pronunciations {
                 tx.execute(
                     "INSERT INTO pronunciations (entry_id, variety, notation, phonemic, audio, text)
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                     params![
                         entry.id, // Foreign key
                         pron.variety,
                         pron.notation,
                         pron.phonemic, // Store bool as integer
                         pron.audio,
                         pron.text,
                     ],
                 )?;
             }

            // Insert Senses
            for sense in &entry.senses {
                tx.execute(
                    "INSERT INTO senses (id, entry_id, synset_id, subcat)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        sense.id,
                        entry.id, // Foreign key
                        sense.synset, // Foreign key (references synset.id)
                        sense.subcat,
                    ],
                )?;
                 // Defer sense_relation insertion to Pass 3
            }
             // Insert Syntactic Behaviours if needed (would likely be in this pass too)
        }

        for synset in &lexicon.synsets {
             // Insert Definitions
             for definition in &synset.definitions {
                 tx.execute(
                     "INSERT INTO definitions (synset_id, text, dc_source)
                      VALUES (?1, ?2, ?3)",
                     params![
                         synset.id, // Foreign key
                         definition.text,
                         definition.dc_source,
                     ],
                 )?;
             }

            // Insert ILI Definition
            if let Some(ili_def) = &synset.ili_definition {
                tx.execute(
                    "INSERT INTO ili_definitions (synset_id, text, dc_source)
                     VALUES (?1, ?2, ?3)",
                     params![
                         synset.id, // Primary key
                         ili_def.text,
                         ili_def.dc_source,
                     ],
                 )?;
             }

            // Insert Examples
            for example in &synset.examples {
                tx.execute(
                    "INSERT INTO examples (synset_id, text, dc_source)
                     VALUES (?1, ?2, ?3)",
                     params![
                         synset.id, // Foreign key
                         example.text,
                         example.dc_source,
                     ],
                 )?;
             }
             // Defer synset_relation insertion to Pass 3
        }
    }
     info!("Pass 2 complete.");

     // --- Pass 3: Insert relations (SenseRelations, SynsetRelations) ---
     info!("Population Pass 3: Inserting SenseRelations, SynsetRelations...");
     for lexicon in &resource.lexicons { // Iterate again
         for entry in &lexicon.lexical_entries {
             for sense in &entry.senses {
                 // Insert Sense Relations (using INSERT OR IGNORE to handle potential duplicates in source data)
                 for relation in &sense.sense_relations {
                     tx.execute(
                         "INSERT OR IGNORE INTO sense_relations (source_sense_id, target_sense_id, rel_type)
                          VALUES (?1, ?2, ?3)",
                         params![
                             sense.id, // Source sense
                             relation.target, // Target sense ID
                             sense_rel_type_to_string(relation.rel_type), // Store type as string
                         ],
                     )?;
                 }
             }
         }
         for synset in &lexicon.synsets {
             // Insert Synset Relations (using INSERT OR IGNORE)
             for relation in &synset.synset_relations {
                 tx.execute(
                     "INSERT OR IGNORE INTO synset_relations (source_synset_id, target_synset_id, rel_type)
                      VALUES (?1, ?2, ?3)",
                     params![
                         synset.id, // Source synset
                         relation.target, // Target synset ID
                         synset_rel_type_to_string(relation.rel_type), // Store type as string
                     ],
                 )?;
             }
         }
     }
     info!("Pass 3 complete.");

    tx.commit()?; // Commit the transaction

    info!(
        "Database population complete. Took {:.2?}",
        start_time.elapsed()
    );
    Ok(())
}

/// Helper to clear all data from WordNet tables before repopulating.
pub fn clear_database_data(tx: &Transaction) -> Result<()> {
    info!("Clearing existing data from database tables...");
    // Order matters due to foreign key constraints (delete from referencing tables first)
    tx.execute("DELETE FROM sense_relations", [])?;
    tx.execute("DELETE FROM synset_relations", [])?;
    tx.execute("DELETE FROM definitions", [])?;
    tx.execute("DELETE FROM ili_definitions", [])?;
    tx.execute("DELETE FROM examples", [])?;
    tx.execute("DELETE FROM pronunciations", [])?;
    // tx.execute("DELETE FROM syntactic_behaviours", [])?; // If used
    tx.execute("DELETE FROM senses", [])?;
    tx.execute("DELETE FROM synsets", [])?;
    tx.execute("DELETE FROM lexical_entries", [])?;
    tx.execute("DELETE FROM lexicons", [])?;
    // Don't delete from metadata table
    info!("Finished clearing data.");
    Ok(())
}


// --- Enum to String Conversion Helpers ---

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
        // Use ParseError for invalid string values from DB
        _ => Err(OewnError::ParseError(format!("Invalid PartOfSpeech string in DB: {}", s))),
    }
}

pub(crate) fn sense_rel_type_to_string(rel_type: SenseRelType) -> String {
    // Use serde_plain for simple enum-string mapping if preferred,
    // or implement manually like this:
    format!("{:?}", rel_type).to_lowercase() // Simple debug representation to lowercase
}

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

pub(crate) fn synset_rel_type_to_string(rel_type: SynsetRelType) -> String {
    format!("{:?}", rel_type).to_lowercase() // Simple debug representation to lowercase
}

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
