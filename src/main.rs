use clap::{Parser, Subcommand};
use colored::*;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{LevelFilter, debug, error, info, warn};
use oewn_rs::{
    LexicalEntry, LoadOptions, SenseRelType, Synset, SynsetRelType, WordNet, error::Result,
    models::PartOfSpeech, progress::{ProgressCallback, ProgressUpdate},
};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// --- CLI Argument Parsing ---

#[derive(Parser, Debug)]
#[command(author, version, about = "Open English WordNet CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to a custom database file (optional)
    #[arg(long, global = true)]
    db_path: Option<String>,

    /// Force reload data, ignoring existing database content
    #[arg(long, global = true, default_value_t = false)]
    force_reload: bool,

    /// Set verbosity level (e.g., -v, -vv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Define a word, optionally filtering by part of speech
    Define {
        /// The word to define
        word: String,
        /// Optional part of speech filter (noun, verb, adj, adv)
        pos: Option<PartOfSpeech>,
    },
    /// Show a random word
    Random,
    /// Clear the WordNet database
    ClearDb,
}

// --- Main Function ---

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // --- Setup Logging ---
    let log_level = match cli.verbose {
        0 => LevelFilter::Warn, // Default
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };
    env_logger::Builder::new()
        .filter(None, log_level) // Use None to apply filter to all modules
        .format(|buf, record| {
            // Simple format: [LEVEL] message
            writeln!(buf, "[{}] {}", record.level(), record.args())
        })
        .init();

    // --- Load WordNet Data with Progress ---
    info!("Loading WordNet data...");

    // Setup Indicatif progress bars
    let multi_progress = MultiProgress::new();
    let progress_bars = Arc::new(Mutex::new(HashMap::<String, ProgressBar>::new())); // Store bars by stage name

    // Define the progress callback
    let mp_clone = multi_progress.clone(); // Clone for use in the callback
    let pb_map_clone = progress_bars.clone();
    let callback: ProgressCallback = Box::new(move |update: ProgressUpdate| {
        let mut bars = pb_map_clone.lock().unwrap(); // Lock the map

        // Check if it's a new stage or an update
        if update.current_item == 0 {
            // New stage or start of existing stage
            let pb = mp_clone.add(ProgressBar::new(update.total_items.unwrap_or(0)));
            let style_template = if update.total_items.is_some() {
                // Template with percentage, bar, count, message
                "{prefix:>12.cyan.bold} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({percent}%) {msg}"
            } else {
                // Template for spinners or unknown length
                "{prefix:>12.cyan.bold} [{elapsed_precise}] {spinner} {msg}"
            };
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(style_template)
                    .unwrap()
                    .progress_chars("##-"),
            );
            pb.set_prefix(update.stage_description.clone());
            pb.set_message(update.message.unwrap_or_default());
            pb.enable_steady_tick(Duration::from_millis(100)); // Update interval
            bars.insert(update.stage_description, pb); // Store the new bar
        } else if let Some(pb) = bars.get(&update.stage_description) {
            // Update existing stage
            pb.set_position(update.current_item);
            if let Some(msg) = update.message {
                 pb.set_message(msg); // Update message if provided
            }
            // Finish the bar if it reaches the total
            if let Some(total) = update.total_items {
                if update.current_item >= total {
                    pb.finish_with_message("Done");
                }
            }
        }
        // In the future, return false here to signal cancellation request
        true
    });


    let load_options = LoadOptions {
        db_path: cli.db_path.as_ref().map(PathBuf::from),
        force_reload: cli.force_reload,
    };

    // Spawn the loading operation into a separate task
    let load_handle = tokio::spawn(async move {
        WordNet::load_with_options(load_options, Some(callback)).await // Pass the callback
    });

    // Await the result of the loading task
    let wn_result = load_handle.await.unwrap_or_else(|e| {
        eprintln!("Error awaiting loading task: {}", e);
        std::process::exit(1); // Exit if the spawned task itself failed
    });

    // Ensure all progress bars are finished and clear them
    // This is important for a clean exit
    multi_progress.clear().unwrap_or_else(|e| eprintln!("Error clearing progress bars: {}", e));


    let wn = match wn_result {
        Ok(wn) => {
            info!("WordNet data loaded successfully.");
            // No need to print success here if progress bars show completion
            wn
        }
        Err(e) => {
            error!("Failed to load WordNet data: {}", e);
            // Error message is already printed by the logger
            eprintln!("{}", format!("Error: {}", e).red()); // Simple error for user
            std::process::exit(1);
        }
    };


    // --- Execute Command ---
    match cli.command {
        Commands::Define { word, pos } => {
            if let Err(e) = handle_define(&wn, &word, pos).await {
                error!("Error during define command: {}", e);
                eprintln!("{}", format!("Error defining '{}': {}", word, e).red());
                std::process::exit(1);
            }
        }
        Commands::Random => {
            if let Err(e) = handle_random(&wn).await {
                error!("Error during random command: {}", e);
                eprintln!("{}", format!("Error getting random word: {}", e).red());
                std::process::exit(1);
            }
        }
        Commands::ClearDb => {
            info!("Clearing database...");
            // Use the same logic as loading to determine which db path to clear
            let db_path_to_clear = if let Some(custom_path) = cli.db_path {
                Some(PathBuf::from(custom_path))
            } else {
                // Try to get default path, ignore error if it fails (e.g., dir not found yet)
                WordNet::get_default_db_path().ok()
            };

            match WordNet::clear_database(db_path_to_clear) {
                Ok(_) => println!("{}", "Database cleared successfully.".green()),
                Err(e) => {
                    error!("Failed to clear database: {}", e);
                    eprintln!("{}", format!("Error clearing database: {}", e).red());
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

// --- Command Handlers ---

async fn handle_define(wn: &WordNet, word: &str, pos_filter: Option<PartOfSpeech>) -> Result<()> {
    info!("Defining word: '{}', PoS filter: {:?}", word, pos_filter);
    let start_lookup = Instant::now();
    let entries = wn.lookup_entries(word, pos_filter)?;
    debug!(
        "lookup_entries for '{}' took: {:?}",
        word,
        start_lookup.elapsed()
    );

    if entries.is_empty() {
        println!("No definitions found for '{}'.", word.yellow());
        return Ok(());
    }

    // Group entries by lemma form and part of speech for structured output
    let mut grouped_entries: HashMap<(String, PartOfSpeech), Vec<LexicalEntry>> = HashMap::new();
    for entry in entries {
        grouped_entries
            .entry((entry.lemma.written_form.clone(), entry.lemma.part_of_speech))
            .or_default()
            .push(entry);
    }

    // Sort groups for consistent output
    let mut sorted_groups: Vec<_> = grouped_entries.into_iter().collect();
    sorted_groups.sort_by(|a, b| a.0.cmp(&b.0));
    for ((lemma_form, pos), entries_for_group) in sorted_groups {
        println!(
            "\n{} ~ {}",
            lemma_form.bold().cyan(),
            pos.to_string().italic() // Italic for POS
        );

        // Print pronunciations (using the first entry in the group)
        if let Some(first_entry) = entries_for_group.first() {
            if !first_entry.pronunciations.is_empty() {
                print!("  Pronunciations: ");
                let pron_strings: Vec<String> = first_entry
                    .pronunciations
                    .iter()
                    .map(|p| format!("{}[{}]", p.text.green(), p.variety.dimmed()))
                    .collect();
                println!("{}", pron_strings.join(", "));
            }
        }

        let mut sense_counter = 1;
        // Iterate through entries for the group
        for entry in entries_for_group {
            let senses = wn.get_senses_for_entry(&entry.id)?;
            for sense in senses {
                let start_sense_processing = Instant::now();
                match wn.get_synset(&sense.synset) {
                    Ok(synset) => {
                        // Pass the current lemma_form and the owned synset
                        print_sense_details(wn, &lemma_form, &synset, sense_counter)?;
                        sense_counter += 1;
                        debug!(
                            "Processing sense {} / synset {} took: {:?}",
                            sense.id,
                            synset.id,
                            start_sense_processing.elapsed()
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Could not find synset {} for sense {}: {}",
                            sense.synset, sense.id, e
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// Helper function to print details for a single sense/synset combination.
fn print_sense_details(
    wn: &WordNet,
    current_lemma: &str,
    synset: &Synset,
    counter: usize,
) -> Result<()> {
    // 1. Print Definition(s)
    for def in &synset.definitions {
        // Indent definition
        println!("  {}: {}", counter.to_string().bold(), def.text.trim());
    }
    if let Some(ili_def) = &synset.ili_definition {
        println!("     ILI: {}", ili_def.text.trim().dimmed()); // Dim ILI definition
    }

    // 2. Print Examples (using synset.examples)
    if !synset.examples.is_empty() {
        for example in &synset.examples {
            // Indent examples
            println!("        {}", example.text.trim().italic());
        }
    }

    // 3. Print Synonyms (using refined logic with sense_to_entry_index)
    let start_synonyms = Instant::now();
    let member_senses = wn.get_senses_for_synset(&synset.id)?;
    let mut synonyms = Vec::new();
    for member_sense in member_senses {
        // Use the helper method to find the entry ID
        if let Some(entry_id) = wn.get_entry_id_for_sense(&member_sense.id)? {
            // Use the helper method to look up the entry
            if let Some(entry) = wn.get_entry_by_id(&entry_id)? {
                // Add the lemma if it's not the one currently being defined
                if entry.lemma.written_form != current_lemma {
                    // Avoid duplicates
                    if !synonyms.contains(&entry.lemma.written_form) {
                        synonyms.push(entry.lemma.written_form.clone());
                    }
                }
            } else {
                warn!(
                    "Entry ID {} found in sense_to_entry_index but not in lexical_entries map.",
                    entry_id
                );
            }
        } // Error from get_entry_id_for_sense or get_entry_by_id is propagated by `?`
    }
    debug!(
        "Synonym lookup for synset {} took: {:?}",
        synset.id,
        start_synonyms.elapsed()
    );

    if !synonyms.is_empty() {
        // Sort synonyms alphabetically
        synonyms.sort();
        println!(
            "        {}: {}",
            "Synonyms".magenta(),
            synonyms.join(", ").green()
        ); // Color label magenta
    }

    // 4. Print selected relations (Antonyms, Hypernyms, Hyponyms)
    // Pass the whole synset to print_relation for Antonym lookup across member senses
    print_relation(wn, synset, SenseRelType::Antonym, "Antonyms")?;
    print_relation(wn, synset, SynsetRelType::Hypernym, "Hypernyms")?;
    print_relation(wn, synset, SynsetRelType::Hyponym, "Hyponyms")?;
    // Other relations can be added here by calling print_relation

    println!(); // Add a blank line after each sense block
    Ok(())
}

/// Helper function to print lemmas for a specific relation type.
/// Handles both SenseRelations (like Antonym) and SynsetRelations (like Hypernym).
/// For SenseRelations, it checks relations across *all* senses within the synset.
fn print_relation(
    wn: &WordNet,
    synset: &Synset, // Takes reference
    rel_type: impl Into<RelTypeMarker>,
    label: &str,
) -> Result<()> {
    let start_relation = Instant::now();
    let rel_type_marker = rel_type.into();
    let mut related_lemmas = Vec::new();

    match rel_type_marker {
        RelTypeMarker::Sense(sense_rel) => {
            // Iterate through ALL senses belonging to this synset
            let member_senses = wn.get_senses_for_synset(&synset.id)?; // Returns Vec<Sense>
            for member_sense in member_senses {
                // member_sense is Sense
                // Get relations for *this specific member sense*
                let related_target_senses = wn.get_related_senses(&member_sense.id, sense_rel)?; // Returns Vec<Sense>
                for target_sense in related_target_senses {
                    // target_sense is Sense
                    // Important: Ensure the target sense is NOT part of the current synset
                    // (e.g., avoid listing members of the same synset as antonyms)
                    if target_sense.synset != synset.id {
                        // Find the entry for this target sense
                        if let Some(entry_id) = wn.get_entry_id_for_sense(&target_sense.id)? {
                            // Returns Result<Option<String>>
                            if let Some(entry) = wn.get_entry_by_id(&entry_id)? {
                                // Returns Result<Option<LexicalEntry>>
                                if !related_lemmas.contains(&entry.lemma.written_form) {
                                    related_lemmas.push(entry.lemma.written_form.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        RelTypeMarker::Synset(synset_rel) => {
            let related_synsets = wn.get_related_synsets(&synset.id, synset_rel)?; // Returns Vec<Synset>
            for target_synset in related_synsets {
                // target_synset is Synset
                // Get all lemmas associated with this target synset
                let target_senses = wn.get_senses_for_synset(&target_synset.id)?; // Returns Vec<Sense>
                for target_sense in target_senses {
                    // target_sense is Sense
                    if let Some(entry_id) = wn.get_entry_id_for_sense(&target_sense.id)? {
                        // Returns Result<Option<String>>
                        if let Some(entry) = wn.get_entry_by_id(&entry_id)? {
                            // Returns Result<Option<LexicalEntry>>
                            if !related_lemmas.contains(&entry.lemma.written_form) {
                                related_lemmas.push(entry.lemma.written_form.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    if !related_lemmas.is_empty() {
        related_lemmas.sort();
        related_lemmas.dedup();
        // Use magenta for relation labels, green for lemmas
        println!(
            "        {}: {}",
            label.magenta(),
            related_lemmas.join(", ").green()
        );
    }
    debug!(
        "Relation lookup for '{}' on synset {} took: {:?}",
        label,
        synset.id,
        start_relation.elapsed()
    );

    Ok(())
}

// Helper enum to dispatch between SenseRelType and SynsetRelType in print_relation
enum RelTypeMarker {
    Sense(SenseRelType),
    Synset(SynsetRelType),
}

impl From<SenseRelType> for RelTypeMarker {
    fn from(rel_type: SenseRelType) -> Self {
        RelTypeMarker::Sense(rel_type)
    }
}

impl From<SynsetRelType> for RelTypeMarker {
    fn from(rel_type: SynsetRelType) -> Self {
        RelTypeMarker::Synset(rel_type)
    }
}

async fn handle_random(wn: &WordNet) -> Result<()> {
    info!("Getting random word...");
    match wn.get_random_entry() {
        Ok(entry) => {
            println!(
                "Random word: {} ({})",
                entry.lemma.written_form.bold().cyan(),
                entry.lemma.part_of_speech.to_string().italic()
            );
        }
        Err(e) => {
            error!("Failed to get random entry: {}", e);
            eprintln!("{}", "Could not retrieve a random word.".red());
        }
    }
    Ok(())
}
