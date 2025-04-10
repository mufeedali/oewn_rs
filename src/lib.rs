// Declare modules
pub mod data;
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
use models::parse_members; // Helper from models
use parse::parse_lmf;
use rand::seq::SliceRandom; // For random word selection
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::Arc; // Using Arc for shared ownership of potentially large parsed data

// --- Constants ---
pub(crate) const CACHE_FORMAT_VERSION: u32 = 1; // Start cache versioning

// --- Processed Data Structure ---

/// Holds the processed and indexed WordNet data for efficient querying.
/// This structure is designed to be cached.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessedWordNet {
    pub(crate) lexicon_info: HashMap<String, LexiconInfo>, // Key: Lexicon ID

    // Primary data stores, indexed by their LMF IDs
    pub(crate) lexical_entries: HashMap<String, LexicalEntry>, // Key: LexicalEntry ID (e.g., "w1")
    pub(crate) synsets: HashMap<String, Synset>, // Key: Synset ID (e.g., "example-en-10161911-n")
    pub(crate) senses: HashMap<String, Sense>,   // Key: Sense ID (e.g., "example-en-10161911-n-1")

    // Indices for fast lookups
    // Lemma (lowercase) -> List of LexicalEntry IDs
    pub(crate) lemma_index: HashMap<String, Vec<String>>,
    // PartOfSpeech -> List of LexicalEntry IDs
    pub(crate) pos_index: HashMap<PartOfSpeech, Vec<String>>,
    // Lemma (lowercase) + PartOfSpeech -> List of LexicalEntry IDs
    pub(crate) lemma_pos_index: HashMap<(String, PartOfSpeech), Vec<String>>,
    // Synset ID -> List of Sense IDs belonging to it
    pub(crate) synset_members_index: HashMap<String, Vec<String>>,
    // Sense ID -> Synset ID it belongs to
    pub(crate) sense_to_synset_index: HashMap<String, String>,
    // LexicalEntry ID -> List of Sense IDs it contains
    pub(crate) entry_senses_index: HashMap<String, Vec<String>>,
    // Sense ID -> LexicalEntry ID it belongs to (for synonym lookup)
    pub(crate) sense_to_entry_index: HashMap<String, String>,
    // List of all lexical entry IDs for random selection
    pub(crate) all_entry_ids: Vec<String>,

    // Indices for relations
    // Source Sense ID -> Relation Type -> List of Target Sense IDs
    pub(crate) sense_relations_index: HashMap<String, HashMap<SenseRelType, Vec<String>>>,
    // Source Synset ID -> Relation Type -> List of Target Synset IDs
    pub(crate) synset_relations_index: HashMap<String, HashMap<SynsetRelType, Vec<String>>>,
}

/// Basic metadata about a Lexicon stored in the processed data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LexiconInfo {
    id: String,
    label: String,
    language: String,
    version: String,
}

impl ProcessedWordNet {
    /// Creates a new, empty ProcessedWordNet.
    fn new() -> Self {
        ProcessedWordNet {
            lexicon_info: HashMap::new(),
            lexical_entries: HashMap::new(),
            synsets: HashMap::new(),
            senses: HashMap::new(),
            lemma_index: HashMap::new(),
            pos_index: HashMap::new(),
            lemma_pos_index: HashMap::new(),
            synset_members_index: HashMap::new(),
            sense_to_synset_index: HashMap::new(),
            entry_senses_index: HashMap::new(),
            sense_to_entry_index: HashMap::new(),
            all_entry_ids: Vec::new(),
            sense_relations_index: HashMap::new(),
            synset_relations_index: HashMap::new(),
        }
    }

    /// Builds the processed data and indices from a parsed LexicalResource.
    fn build(resource: LexicalResource) -> Result<Self> {
        info!("Building processed data structures and indices...");
        let mut processed = ProcessedWordNet::new();

        for lexicon in resource.lexicons {
            info!("Processing Lexicon: {} ({})", lexicon.label, lexicon.id);
            processed.lexicon_info.insert(
                lexicon.id.clone(),
                LexiconInfo {
                    id: lexicon.id.clone(),
                    label: lexicon.label.clone(),
                    language: lexicon.language.clone(),
                    version: lexicon.version.clone(),
                },
            );

            for entry in lexicon.lexical_entries {
                let entry_id = entry.id.clone();
                let lemma_text_lower = entry.lemma.written_form.to_lowercase();
                let pos = entry.lemma.part_of_speech;

                // Add to main entry store
                processed
                    .lexical_entries
                    .insert(entry_id.clone(), entry.clone()); // Clone entry here
                processed.all_entry_ids.push(entry_id.clone());

                // Update indices
                processed
                    .lemma_index
                    .entry(lemma_text_lower.clone())
                    .or_default()
                    .push(entry_id.clone());
                processed
                    .pos_index
                    .entry(pos)
                    .or_default()
                    .push(entry_id.clone());
                processed
                    .lemma_pos_index
                    .entry((lemma_text_lower, pos))
                    .or_default()
                    .push(entry_id.clone());

                // Process senses within the entry
                let mut sense_ids_for_entry: Vec<String> = Vec::new();
                for sense in entry.senses {
                    let sense_id = sense.id.clone();
                    let synset_id = sense.synset.clone();

                    sense_ids_for_entry.push(sense_id.clone());
                    // Clone sense when inserting into the map
                    processed.senses.insert(sense_id.clone(), sense.clone());
                    processed
                        .sense_to_synset_index
                        .insert(sense_id.clone(), synset_id.clone()); // Index sense -> synset
                    processed
                        .sense_to_entry_index
                        .insert(sense_id.clone(), entry_id.clone()); // Index sense -> entry

                    // Index sense relations
                    for relation in &sense.sense_relations {
                        processed
                            .sense_relations_index
                            .entry(sense_id.clone())
                            .or_default()
                            .entry(relation.rel_type)
                            .or_default()
                            .push(relation.target.clone());
                    }
                }
                processed
                    .entry_senses_index
                    .insert(entry_id.clone(), sense_ids_for_entry); // Index entry -> senses
            }

            for synset in lexicon.synsets {
                let synset_id = synset.id.clone();
                let member_entry_ids = parse_members(&synset.members); // These are entry IDs, not sense IDs

                // Add to main synset store
                // Clone synset when inserting into the map
                processed.synsets.insert(synset_id.clone(), synset.clone());

                // Build synset_members_index: Find the actual SENSE IDs belonging to this synset
                // by looking up the senses associated with the ENTRY IDs listed in the 'members' attribute
                // and checking if those senses belong to the current synset.
                let mut actual_member_sense_ids: Vec<String> = Vec::new();
                for entry_id in member_entry_ids {
                    // Find senses associated with this entry ID using the already populated index
                    if let Some(senses_for_entry) = processed.entry_senses_index.get(&entry_id) {
                        // Filter these senses to find the one(s) belonging to the *current* synset
                        for sense_id in senses_for_entry {
                            // Check which synset this sense belongs to
                            if let Some(sense_synset_id) =
                                processed.sense_to_synset_index.get(sense_id)
                            {
                                if sense_synset_id == &synset_id {
                                    // Found a sense from this entry that belongs to this synset
                                    actual_member_sense_ids.push(sense_id.clone());
                                }
                            } else {
                                warn!(
                                    "Sense ID '{}' found via entry '{}' has no entry in sense_to_synset_index.",
                                    sense_id, entry_id
                                );
                            }
                        }
                    } else {
                        warn!(
                            "Entry ID '{}' listed in members of synset '{}' not found in entry_senses_index.",
                            entry_id, synset_id
                        );
                    }
                }
                // Update synset -> members index with the *actual* sense IDs
                processed
                    .synset_members_index
                    .insert(synset_id.clone(), actual_member_sense_ids);

                // Index synset relations
                for relation in &synset.synset_relations {
                    processed
                        .synset_relations_index
                        .entry(synset_id.clone())
                        .or_default()
                        .entry(relation.rel_type) // Assuming SynsetRelType is Copy
                        .or_default()
                        .push(relation.target.clone());
                }
            }
        }

        info!(
            "Finished building processed data: {} entries, {} synsets, {} senses.",
            processed.lexical_entries.len(),
            processed.synsets.len(),
            processed.senses.len()
        );
        Ok(processed)
    }
}

// --- WordNet Struct ---

/// Options for loading WordNet data.
#[derive(Debug, Default, Clone)]
pub struct LoadOptions {
    /// Optional path to a specific cache file to use or create.
    /// If None, the default location based on ProjectDirs will be used.
    pub cache_path: Option<PathBuf>,
    /// Force reloading and parsing, ignoring any existing cache.
    pub force_reload: bool,
}

/// The main WordNet interface.
#[derive(Clone)] // Clone is cheap due to Arc
pub struct WordNet {
    // Holds the processed data, shared across clones
    processed_data: Arc<ProcessedWordNet>,
    // Keep the path to the original XML data file if needed later
    _data_file_path: Arc<PathBuf>,
}

impl WordNet {
    /// Loads the WordNet data using default options (automatic cache path).
    ///
    /// Ensures data is downloaded/extracted if needed.
    /// Attempts to load from cache; parses raw data and creates cache if invalid/missing.
    pub async fn load() -> Result<Self> {
        Self::load_with_options(LoadOptions::default()).await
    }

    /// Loads the WordNet data with specific options.
    pub async fn load_with_options(options: LoadOptions) -> Result<Self> {
        // 1. Ensure raw XML data file is present and get its path
        let xml_path = data::ensure_data().await?;
        info!("OEWN XML data available at: {:?}", xml_path);

        // 2. Determine cache file path
        let cache_path = match options.cache_path {
            Some(path) => {
                info!("Using provided cache path: {:?}", path);
                path
            }
            None => Self::get_default_cache_path()?,
        };
        info!("Using cache path: {:?}", cache_path);

        // 3. Try loading from cache (unless force_reload is true)
        if !options.force_reload && cache_path.exists() {
            info!("Attempting to load WordNet data from cache...");
            match Self::load_from_cache(&cache_path) {
                Ok(processed_data) => {
                    info!("Successfully loaded WordNet data from cache.");
                    return Ok(WordNet {
                        processed_data: Arc::new(processed_data),
                        _data_file_path: Arc::new(xml_path),
                    });
                }
                Err(e) => {
                    warn!(
                        "Failed to load from cache file {:?} (error: {}). Regenerating cache.",
                        cache_path, e
                    );
                    // Attempt to delete potentially corrupted cache file
                    if let Err(del_err) = std::fs::remove_file(&cache_path) {
                        warn!(
                            "Failed to delete potentially corrupted cache file {:?}: {}",
                            cache_path, del_err
                        );
                    }
                }
            }
        } else if options.force_reload {
            info!("Force reload requested, skipping cache load attempt.");
            // Clean existing cache if forcing reload
            if cache_path.exists() {
                if let Err(del_err) = std::fs::remove_file(&cache_path) {
                    warn!(
                        "Failed to delete cache file during force reload: {}",
                        del_err
                    );
                } else {
                    info!(
                        "Removed existing cache file due to force reload: {:?}",
                        cache_path
                    );
                }
            }
        } else {
            info!("Cache file not found. Parsing raw data files.");
        }

        // 4. If cache loading failed, cache didn't exist, or force_reload=true, parse raw data
        info!("Reading and parsing XML file: {:?}", xml_path);
        let xml_content = tokio::fs::read_to_string(&xml_path).await?;
        let resource = parse_lmf(&xml_content).await?;

        // 5. Build processed data structures
        let processed_data = ProcessedWordNet::build(resource)?;

        // 6. Save the newly processed data to the cache file
        info!("Saving processed data to cache file: {:?}", cache_path);
        if let Err(e) = Self::save_to_cache(&processed_data, &cache_path) {
            // Log error but don't fail the load operation itself, just caching
            error!("Failed to save data to cache file {:?}: {}", cache_path, e);
        } else {
            info!("Successfully saved data to cache file.");
        }

        Ok(WordNet {
            processed_data: Arc::new(processed_data),
            _data_file_path: Arc::new(xml_path),
        })
    }

    /// Gets the default path for the cache file.
    pub fn get_default_cache_path() -> Result<PathBuf> {
        let project_dirs = ProjectDirs::from("org", "OewnRs", data::OEWN_SUBDIR)
            .ok_or(OewnError::DataDirNotFound)?;
        let cache_dir = project_dirs.cache_dir();
        fs::create_dir_all(cache_dir)?;
        let cache_filename = format!(
            "oewn-{}-lmf-cache-v{}.bin", // Include "lmf" in name
            data::OEWN_VERSION,
            CACHE_FORMAT_VERSION
        );
        Ok(cache_dir.join(cache_filename))
    }

    /// Attempts to load and deserialize ProcessedWordNet data from a cache file.
    fn load_from_cache(cache_path: &PathBuf) -> Result<ProcessedWordNet> {
        let file = File::open(cache_path)?;
        let mut reader = BufReader::new(file);

        // Read and verify cache format version
        let mut version_bytes = [0u8; 4];
        reader.read_exact(&mut version_bytes)?;
        let file_version = u32::from_le_bytes(version_bytes);

        if file_version != CACHE_FORMAT_VERSION {
            return Err(OewnError::ParseError(format!(
                "Cache format version mismatch (expected {}, found {})",
                CACHE_FORMAT_VERSION, file_version
            )));
        }

        // Deserialize the rest of the data
        let processed_data: ProcessedWordNet = bincode::deserialize_from(reader)?;
        Ok(processed_data)
    }

    /// Serializes and saves ProcessedWordNet data to a cache file.
    fn save_to_cache(processed_data: &ProcessedWordNet, cache_path: &PathBuf) -> Result<()> {
        if let Some(parent_dir) = cache_path.parent() {
            fs::create_dir_all(parent_dir)?;
        }

        let file = File::create(cache_path)?;
        let mut writer = BufWriter::new(file);

        // Write cache format version first
        writer.write_all(&CACHE_FORMAT_VERSION.to_le_bytes())?;

        // Serialize and write the data
        bincode::serialize_into(writer, processed_data)?;
        Ok(())
    }

    /// Clears the WordNet cache file(s).
    ///
    /// If `cache_path_override` is `Some`, it attempts to delete that specific file.
    /// If `cache_path_override` is `None`, it calculates the default cache path and attempts to delete that file.
    pub fn clear_cache(cache_path_override: Option<PathBuf>) -> Result<()> {
        let path_to_clear = match cache_path_override {
            Some(path) => {
                info!("Attempting to clear specified cache file: {:?}", path);
                path
            }
            None => {
                let default_path = Self::get_default_cache_path()?;
                info!("Attempting to clear default cache file: {:?}", default_path);
                default_path
            }
        };

        if path_to_clear.exists() {
            match std::fs::remove_file(&path_to_clear) {
                Ok(_) => {
                    info!("Successfully deleted cache file: {:?}", path_to_clear);
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to delete cache file {:?}: {}", path_to_clear, e);
                    Err(OewnError::Io(e))
                }
            }
        } else {
            info!(
                "Cache file not found, nothing to clear: {:?}",
                path_to_clear
            );
            Ok(()) // Not an error if the file doesn't exist
        }
    }

    /// Clears the default WordNet cache file.
    pub fn clear_default_cache() -> Result<()> {
        Self::clear_cache(None)
    }

    /// Looks up lexical entries for a given lemma, optionally filtering by PartOfSpeech.
    pub fn lookup_entries(
        &self,
        lemma: &str,
        pos_filter: Option<PartOfSpeech>,
    ) -> Result<Vec<&LexicalEntry>> {
        let lemma_lower = lemma.to_lowercase();
        let entry_ids = match pos_filter {
            Some(pos) => self
                .processed_data
                .lemma_pos_index
                .get(&(lemma_lower, pos))
                .map(|ids| ids.as_slice())
                .unwrap_or(&[]), // Get IDs for specific lemma+pos
            None => self
                .processed_data
                .lemma_index
                .get(&lemma_lower)
                .map(|ids| ids.as_slice())
                .unwrap_or(&[]), // Get IDs for lemma across all POS
        };

        let entries: Vec<&LexicalEntry> = entry_ids
            .iter()
            .filter_map(|id| self.processed_data.lexical_entries.get(id)) // Look up IDs in main store
            .collect();

        if entries.is_empty() {
            debug!(
                "No entries found for lemma '{}', pos_filter: {:?}",
                lemma, pos_filter
            );
        }

        Ok(entries)
    }

    /// Retrieves a specific Synset by its ID string.
    pub fn get_synset(&self, id: &str) -> Result<&Synset> {
        self.processed_data
            .synsets
            .get(id)
            .ok_or_else(|| OewnError::SynsetNotFound(id.to_string()))
    }

    /// Retrieves a specific Sense by its ID string.
    pub fn get_sense(&self, id: &str) -> Result<&Sense> {
        self.processed_data
            .senses
            .get(id)
            .ok_or_else(|| OewnError::Internal(format!("Sense ID not found in index: {}", id)))
    }

    /// Retrieves all Senses associated with a specific Lexical Entry ID.
    pub fn get_senses_for_entry(&self, entry_id: &str) -> Result<Vec<&Sense>> {
        let sense_ids = self
            .processed_data
            .entry_senses_index
            .get(entry_id)
            .ok_or_else(|| OewnError::LexicalEntryNotFound(entry_id.to_string()))?;

        let senses: Vec<&Sense> = sense_ids
            .iter()
            .filter_map(|id| self.processed_data.senses.get(id))
            .collect();

        Ok(senses)
    }

    /// Retrieves all Senses associated with a specific Synset ID.
    pub fn get_senses_for_synset(&self, synset_id: &str) -> Result<Vec<&Sense>> {
        let sense_ids = self
            .processed_data
            .synset_members_index
            .get(synset_id)
            .ok_or_else(|| OewnError::SynsetNotFound(synset_id.to_string()))?;

        let senses: Vec<&Sense> = sense_ids
            .iter()
            .filter_map(|id| self.processed_data.senses.get(id))
            .collect();

        Ok(senses)
    }

    /// Retrieves a random lexical entry.
    pub fn get_random_entry(&self) -> Result<&LexicalEntry> {
        let mut rng = rand::thread_rng();
        self.processed_data
            .all_entry_ids
            .choose(&mut rng) // Choose a random ID
            .and_then(|id| self.processed_data.lexical_entries.get(id)) // Look up the entry
            .ok_or(OewnError::Internal(
                "Failed to select a random entry".to_string(),
            ))
    }

    /// Returns an iterator over all lexical entries in the WordNet data.
    pub fn all_entries(&self) -> std::collections::hash_map::Values<'_, String, LexicalEntry> {
        self.processed_data.lexical_entries.values()
    }

    // --- Public Helper Methods for Accessing Processed Data ---

    /// Retrieves the entry ID for a given sense ID.
    pub fn get_entry_id_for_sense(&self, sense_id: &str) -> Option<&String> {
        self.processed_data.sense_to_entry_index.get(sense_id)
    }

    /// Retrieves an entry by its ID.
    pub fn get_entry_by_id(&self, entry_id: &str) -> Option<&LexicalEntry> {
        self.processed_data.lexical_entries.get(entry_id)
    }

    /// Retrieves target Sense IDs for a given source Sense ID and relation type.
    fn get_related_sense_ids(
        &self,
        sense_id: &str,
        rel_type: SenseRelType,
    ) -> Option<&Vec<String>> {
        self.processed_data
            .sense_relations_index
            .get(sense_id)
            .and_then(|relations| relations.get(&rel_type))
    }

    /// Retrieves target Synset IDs for a given source Synset ID and relation type.
    fn get_related_synset_ids(
        &self,
        synset_id: &str,
        rel_type: SynsetRelType,
    ) -> Option<&Vec<String>> {
        self.processed_data
            .synset_relations_index
            .get(synset_id)
            .and_then(|relations| relations.get(&rel_type))
    }

    /// Retrieves related Senses for a given source Sense ID and relation type.
    pub fn get_related_senses(
        &self,
        sense_id: &str,
        rel_type: SenseRelType,
    ) -> Result<Vec<&Sense>> {
        let target_ids = match self.get_related_sense_ids(sense_id, rel_type) {
            Some(ids) => ids,
            None => return Ok(Vec::new()), // No relations of this type found
        };

        let senses: Vec<&Sense> = target_ids
            .iter()
            .filter_map(|id| self.processed_data.senses.get(id))
            .collect();

        // Check if we found all expected senses (optional sanity check)
        if senses.len() != target_ids.len() {
            warn!(
                "Could not find all target senses for sense {} relation {:?}",
                sense_id, rel_type
            );
        }

        Ok(senses)
    }

    /// Retrieves related Synsets for a given source Synset ID and relation type.
    pub fn get_related_synsets(
        &self,
        synset_id: &str,
        rel_type: SynsetRelType,
    ) -> Result<Vec<&Synset>> {
        let target_ids = match self.get_related_synset_ids(synset_id, rel_type) {
            Some(ids) => ids,
            None => return Ok(Vec::new()), // No relations of this type found
        };

        let synsets: Vec<&Synset> = target_ids
            .iter()
            .filter_map(|id| self.processed_data.synsets.get(id))
            .collect();

        // Check if we found all expected synsets (optional sanity check)
        if synsets.len() != target_ids.len() {
            warn!(
                "Could not find all target synsets for synset {} relation {:?}",
                synset_id, rel_type
            );
        }

        Ok(synsets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    // Helper to create a minimal ProcessedWordNet for testing save/load cache
    fn create_dummy_processed_data() -> ProcessedWordNet {
        let mut processed = ProcessedWordNet::new();
        processed.lexicon_info.insert(
            "test-en".to_string(),
            LexiconInfo {
                id: "test-en".to_string(),
                label: "Test".to_string(),
                language: "en".to_string(),
                version: "1.0".to_string(),
            },
        );
        // Add more dummy data if needed for more thorough tests
        processed
    }

    // Helper to create a WordNet instance with predictable test data
    fn create_test_wordnet() -> WordNet {
        let mut processed = ProcessedWordNet::new();

        // Lexicon Info
        processed.lexicon_info.insert(
            "test-en".to_string(),
            LexiconInfo {
                id: "test-en".to_string(),
                label: "Test English".to_string(),
                language: "en".to_string(),
                version: "1.0".to_string(),
            },
        );

        // Entries
        let entry_cat = LexicalEntry {
            id: "w1".to_string(),
            lemma: Lemma {
                written_form: "cat".to_string(),
                part_of_speech: PartOfSpeech::N,
            },
            pronunciations: vec![],
            senses: vec![],
            syntactic_behaviours: vec![],
        };
        let entry_dog = LexicalEntry {
            id: "w2".to_string(),
            lemma: Lemma {
                written_form: "dog".to_string(),
                part_of_speech: PartOfSpeech::N,
            },
            pronunciations: vec![],
            senses: vec![],
            syntactic_behaviours: vec![],
        };
        let entry_run = LexicalEntry {
            id: "w3".to_string(),
            lemma: Lemma {
                written_form: "run".to_string(),
                part_of_speech: PartOfSpeech::V,
            },
            pronunciations: vec![],
            senses: vec![],
            syntactic_behaviours: vec![],
        };
        processed
            .lexical_entries
            .insert("w1".to_string(), entry_cat.clone());
        processed
            .lexical_entries
            .insert("w2".to_string(), entry_dog.clone());
        processed
            .lexical_entries
            .insert("w3".to_string(), entry_run.clone());
        processed.all_entry_ids = vec!["w1".to_string(), "w2".to_string(), "w3".to_string()];

        // Senses
        let sense_cat_1 = Sense {
            id: "s1".to_string(),
            synset: "syn1".to_string(),
            subcat: None,
            sense_relations: vec![],
        };
        let sense_dog_1 = Sense {
            id: "s2".to_string(),
            synset: "syn2".to_string(),
            subcat: None,
            sense_relations: vec![],
        };
        let sense_run_1 = Sense {
            id: "s3".to_string(),
            synset: "syn3".to_string(),
            subcat: None,
            sense_relations: vec![],
        };
        processed.senses.insert("s1".to_string(), sense_cat_1.clone());
        processed.senses.insert("s2".to_string(), sense_dog_1.clone());
        processed.senses.insert("s3".to_string(), sense_run_1.clone());

        // Synsets
        let synset_cat = Synset {
            id: "syn1".to_string(),
            ili: Some("i1".to_string()),
            part_of_speech: PartOfSpeech::N, // (Synset PoS is not optional)
            definitions: vec![Definition {
                dc_source: None,
                text: "A small domesticated carnivorous mammal".to_string(),
            }],
            ili_definition: None,
            synset_relations: vec![SynsetRelation {
                rel_type: SynsetRelType::Hypernym,
                target: "syn_animal".to_string(), // Hypothetical animal synset
            }],
            examples: vec![],
            members: "w1".to_string(), // Simplified for test setup
        };
        let synset_dog = Synset {
            id: "syn2".to_string(),
            ili: Some("i2".to_string()),
            part_of_speech: PartOfSpeech::N,
            definitions: vec![],
            ili_definition: None,
            synset_relations: vec![],
            examples: vec![],
            members: "w2".to_string(),
        };
        let synset_run = Synset {
            id: "syn3".to_string(),
            ili: Some("i3".to_string()),
            part_of_speech: PartOfSpeech::V,
            definitions: vec![],
            ili_definition: None,
            synset_relations: vec![],
            examples: vec![],
            members: "w3".to_string(),
        };
        processed
            .synsets
            .insert("syn1".to_string(), synset_cat.clone());
        processed
            .synsets
            .insert("syn2".to_string(), synset_dog.clone());
        processed
            .synsets
            .insert("syn3".to_string(), synset_run.clone());

        // Indices (Manually populate necessary ones for tests)
        processed
            .lemma_index
            .insert("cat".to_string(), vec!["w1".to_string()]);
        processed
            .lemma_index
            .insert("dog".to_string(), vec!["w2".to_string()]);
        processed
            .lemma_index
            .insert("run".to_string(), vec!["w3".to_string()]);

        processed
            .pos_index
            .insert(PartOfSpeech::N, vec!["w1".to_string(), "w2".to_string()]);
        processed
            .pos_index
            .insert(PartOfSpeech::V, vec!["w3".to_string()]);

        processed
            .lemma_pos_index
            .insert(("cat".to_string(), PartOfSpeech::N), vec!["w1".to_string()]);
        processed
            .lemma_pos_index
            .insert(("dog".to_string(), PartOfSpeech::N), vec!["w2".to_string()]);
        processed
            .lemma_pos_index
            .insert(("run".to_string(), PartOfSpeech::V), vec!["w3".to_string()]);

        processed
            .entry_senses_index
            .insert("w1".to_string(), vec!["s1".to_string()]);
        processed
            .entry_senses_index
            .insert("w2".to_string(), vec!["s2".to_string()]);
        processed
            .entry_senses_index
            .insert("w3".to_string(), vec!["s3".to_string()]);

        processed
            .sense_to_synset_index
            .insert("s1".to_string(), "syn1".to_string());
        processed
            .sense_to_synset_index
            .insert("s2".to_string(), "syn2".to_string());
        processed
            .sense_to_synset_index
            .insert("s3".to_string(), "syn3".to_string());

        processed
            .synset_members_index
            .insert("syn1".to_string(), vec!["s1".to_string()]);
        processed
            .synset_members_index
            .insert("syn2".to_string(), vec!["s2".to_string()]);
        processed
            .synset_members_index
            .insert("syn3".to_string(), vec!["s3".to_string()]);

        processed.synset_relations_index.insert(
            "syn1".to_string(),
            HashMap::from([(SynsetRelType::Hypernym, vec!["syn_animal".to_string()])]), // Hypernym is valid for SynsetRelType
        );

        processed
            .sense_to_entry_index
            .insert("s1".to_string(), "w1".to_string());
        processed
            .sense_to_entry_index
            .insert("s2".to_string(), "w2".to_string());
        processed
            .sense_to_entry_index
            .insert("s3".to_string(), "w3".to_string());

        // Create a dummy path for _data_file_path
        let dummy_path = PathBuf::from("/dummy/path/data.xml");

        WordNet {
            processed_data: Arc::new(processed),
            _data_file_path: Arc::new(dummy_path),
        }
    }

    #[test]
    fn test_cache_save_load() {
        let _ = env_logger::builder().is_test(true).try_init();
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let cache_path = temp_dir.path().join("test-cache.bin");

        let original_data = create_dummy_processed_data();
        // Test saving
        let save_result = WordNet::save_to_cache(&original_data, &cache_path);
        assert!(
            save_result.is_ok(),
            "Failed to save cache: {:?}",
            save_result.err()
        );
        assert!(cache_path.exists());
        // Test loading
        let load_result = WordNet::load_from_cache(&cache_path);
        assert!(
            load_result.is_ok(),
            "Failed to load cache: {:?}",
            load_result.err()
        );
        let loaded_data = load_result.unwrap();
        // Assert data is the same (requires PartialEq on ProcessedWordNet and contained types)
        // We need to implement PartialEq for ProcessedWordNet for this assertion.
        // For now, let's check a simple field.
        assert_eq!(
            loaded_data.lexicon_info.get("test-en").unwrap().label,
            "Test"
        );

        // Test version mismatch
        let mut file = File::create(&cache_path).unwrap();
        let wrong_version: u32 = CACHE_FORMAT_VERSION + 1;
        file.write_all(&wrong_version.to_le_bytes()).unwrap();
        // Intentionally write incomplete data after version
        file.write_all(b"garbage").unwrap();
        drop(file); // Close file
        let load_mismatch_result = WordNet::load_from_cache(&cache_path);
        assert!(load_mismatch_result.is_err());
        assert!(matches!(
            load_mismatch_result.unwrap_err(),
            OewnError::ParseError(_)
        )); // Check it's a parse error due to version

        // Test clear cache
        assert!(WordNet::clear_cache(Some(cache_path.clone())).is_ok());
        assert!(!cache_path.exists());
        // Test clearing non-existent cache
        assert!(WordNet::clear_cache(Some(cache_path.clone())).is_ok());
    }

    // Implement PartialEq for cache testing comparison
    impl PartialEq for ProcessedWordNet {
        fn eq(&self, other: &Self) -> bool {
            self.lexicon_info == other.lexicon_info
            // Add comparisons for other fields if needed for more thorough tests
        }
    }
    impl PartialEq for LexiconInfo {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
                && self.label == other.label
                && self.language == other.language
                && self.version == other.version
        }
    }

    #[test]
    fn test_lookup_entries() {
        let wn = create_test_wordnet();

        // Test basic lookup
        let cat_entries = wn.lookup_entries("cat", None).unwrap();
        assert_eq!(cat_entries.len(), 1);
        assert_eq!(cat_entries[0].id, "w1");
        assert_eq!(cat_entries[0].lemma.written_form, "cat");

        // Test case-insensitivity
        let dog_entries_upper = wn.lookup_entries("Dog", None).unwrap();
        assert_eq!(dog_entries_upper.len(), 1);
        assert_eq!(dog_entries_upper[0].id, "w2");

        // Test POS filtering (Noun)
        let cat_noun_entries = wn
            .lookup_entries("cat", Some(PartOfSpeech::N))
            .unwrap();
        assert_eq!(cat_noun_entries.len(), 1);
        assert_eq!(cat_noun_entries[0].id, "w1");

        // Test POS filtering (Verb - should be empty for "cat")
        let cat_verb_entries = wn
            .lookup_entries("cat", Some(PartOfSpeech::V))
            .unwrap();
        assert!(cat_verb_entries.is_empty());

        // Test POS filtering (Verb - should find "run")
        let run_verb_entries = wn
            .lookup_entries("run", Some(PartOfSpeech::V))
            .unwrap();
        assert_eq!(run_verb_entries.len(), 1);
        assert_eq!(run_verb_entries[0].id, "w3");

        // Test not found
        let unknown_entries = wn.lookup_entries("unknownword", None).unwrap();
        assert!(unknown_entries.is_empty());
    }

    #[test]
    fn test_get_synset() {
        let wn = create_test_wordnet();
        let synset = wn.get_synset("syn1").unwrap();
        assert_eq!(synset.id, "syn1");
        assert_eq!(
            synset.definitions[0].text,
            "A small domesticated carnivorous mammal"
        );

        // Test not found
        let result = wn.get_synset("syn_nonexistent");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OewnError::SynsetNotFound(_)));
    }

    #[test]
    fn test_get_sense() {
        let wn = create_test_wordnet();
        let sense = wn.get_sense("s1").unwrap();
        assert_eq!(sense.id, "s1");
        assert_eq!(sense.synset, "syn1");

        // Test not found (should be Internal error as index should be consistent)
        let result = wn.get_sense("s_nonexistent");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OewnError::Internal(_)));
    }

    #[test]
    fn test_get_senses_for_entry() {
        let wn = create_test_wordnet();
        let senses = wn.get_senses_for_entry("w1").unwrap();
        assert_eq!(senses.len(), 1);
        assert_eq!(senses[0].id, "s1");

        // Test entry with no senses (not in test data, but test index lookup)
        let result = wn.get_senses_for_entry("w_nonexistent");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OewnError::LexicalEntryNotFound(_)
        ));
    }

    #[test]
    fn test_get_senses_for_synset() {
        let wn = create_test_wordnet();
        let senses = wn.get_senses_for_synset("syn1").unwrap();
        assert_eq!(senses.len(), 1);
        assert_eq!(senses[0].id, "s1");

        // Test synset with no senses (not in test data, but test index lookup)
        let result = wn.get_senses_for_synset("syn_nonexistent");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OewnError::SynsetNotFound(_)));
    }

    #[test]
    fn test_get_random_entry() {
        let wn = create_test_wordnet();
        let entry = wn.get_random_entry().unwrap();
        // Check it's one of the known entries
        assert!(["w1", "w2", "w3"].contains(&entry.id.as_str()));
    }

    #[test]
    fn test_get_entry_id_for_sense() {
        let wn = create_test_wordnet();
        assert_eq!(wn.get_entry_id_for_sense("s1"), Some(&"w1".to_string()));
        assert_eq!(wn.get_entry_id_for_sense("s2"), Some(&"w2".to_string()));
        assert_eq!(wn.get_entry_id_for_sense("s_nonexistent"), None);
    }

    #[test]
    fn test_get_entry_by_id() {
        let wn = create_test_wordnet();
        let entry = wn.get_entry_by_id("w1").unwrap();
        assert_eq!(entry.id, "w1");
        assert_eq!(entry.lemma.written_form, "cat");
        assert_eq!(wn.get_entry_by_id("w_nonexistent"), None);
    }

    #[test]
    fn test_get_related_senses() {
        let wn = create_test_wordnet();
        // Test relation lookup - expecting empty because no relations defined in test data
        let related_derivation = wn.get_related_senses("s1", SenseRelType::Derivation).unwrap();
        assert!(related_derivation.is_empty());

        // Test non-existent source sense
        let related_nonexistent = wn
            .get_related_senses("s_nonexistent", SenseRelType::Derivation)
            .unwrap();
        assert!(related_nonexistent.is_empty());
    }

    #[test]
    fn test_get_related_synsets() {
        let wn = create_test_wordnet();
        // Test existing relation
        let related = wn
            .get_related_synsets("syn1", SynsetRelType::Hypernym) // Hypernym is valid here
            .unwrap();
        // Similar to senses, 'syn_animal' isn't in the synsets map.
        assert!(related.is_empty()); // Because target syn_animal isn't in the map

        // Test non-existent relation type for the synset
        let related_hyponym = wn
            .get_related_synsets("syn1", SynsetRelType::Hyponym) // Hyponym is valid here
            .unwrap();
        assert!(related_hyponym.is_empty());

        // Test non-existent source synset
        let related_nonexistent = wn
            .get_related_synsets("syn_nonexistent", SynsetRelType::Hypernym)
            .unwrap();
        assert!(related_nonexistent.is_empty());
    }
}
