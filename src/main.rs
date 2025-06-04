//! Command-line interface for the Open English WordNet (OEWN) library.
//!
//! This CLI provides commands for looking up word definitions, viewing random words,
//! and managing the WordNet database.

use clap::{Parser, Subcommand};
use colored::*;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{LevelFilter, debug, error, info, warn};
use oewn_rs::{
    LexicalEntry, LoadOptions, SenseRelType, Synset, SynsetRelType, WordNet,
    error::Result,
    models::PartOfSpeech,
    progress::{ProgressCallback, ProgressUpdate},
};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

    /// Set verbosity level (use -v, -vv, or -vvv for increasing verbosity)
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

/// Sets up logging based on verbosity level.
fn setup_logging(verbose: u8) {
    let log_level = match verbose {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    env_logger::Builder::new()
        .filter(None, log_level)
        .format(|buf, record| writeln!(buf, "[{}] {}", record.level(), record.args()))
        .init();
}

/// Creates a progress callback for displaying download and processing progress.
fn create_progress_callback(
    multi_progress: MultiProgress,
    progress_bars: Arc<Mutex<HashMap<String, ProgressBar>>>,
) -> ProgressCallback {
    Box::new(move |update: ProgressUpdate| {
        let mut bars = progress_bars.lock().unwrap();

        if update.current_item == 0 && !bars.contains_key(&update.stage_description) {
            // Create new progress bar for this stage
            let pb = multi_progress.add(ProgressBar::new(update.total_items.unwrap_or(0)));
            let style_template = if update.total_items.is_some() {
                "{prefix:>12.cyan.bold} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({percent}%) {msg}"
            } else {
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
            pb.enable_steady_tick(Duration::from_millis(100));
            bars.insert(update.stage_description.clone(), pb);
        } else if let Some(pb) = bars.get(&update.stage_description) {
            // Update existing progress bar
            pb.set_position(update.current_item);
            if let Some(msg) = update.message {
                pb.set_message(msg);
            }
            if let Some(total) = update.total_items {
                if update.current_item >= total {
                    pb.finish_and_clear();
                }
            }
        }
        true
    })
}

/// Main entry point for the CLI application.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    setup_logging(cli.verbose);

    info!("Loading WordNet data...");

    let multi_progress = MultiProgress::new();
    let progress_bars = Arc::new(Mutex::new(HashMap::<String, ProgressBar>::new()));

    let callback = create_progress_callback(multi_progress.clone(), progress_bars.clone());

    let load_options = LoadOptions {
        db_path: cli.db_path.as_ref().map(PathBuf::from),
        force_reload: cli.force_reload,
    };

    let load_handle =
        tokio::spawn(async move { WordNet::load_with_options(load_options, Some(callback)).await });

    let wn_result = load_handle.await.unwrap_or_else(|e| {
        eprintln!("Error awaiting loading task: {}", e);
        std::process::exit(1);
    });

    // Clean up progress bars
    {
        let bars = progress_bars.lock().unwrap();
        for (_, pb) in bars.iter() {
            pb.finish_and_clear();
        }
    }
    drop(multi_progress); // Explicitly drop to ensure cleanup
    std::io::stdout().flush().ok();

    let wn = match wn_result {
        Ok(wn) => {
            info!("WordNet data loaded successfully.");
            wn
        }
        Err(e) => {
            error!("Failed to load WordNet data: {}", e);
            eprintln!("{}", format!("Error: {}", e).red());
            std::process::exit(1);
        }
    };

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
            let db_path_to_clear = if let Some(custom_path) = cli.db_path {
                Some(PathBuf::from(custom_path))
            } else {
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
/// Handles the define command by looking up and displaying word definitions.
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

    let mut grouped_entries: HashMap<(String, PartOfSpeech), Vec<LexicalEntry>> = HashMap::new();
    for entry in entries {
        grouped_entries
            .entry((entry.lemma.written_form.clone(), entry.lemma.part_of_speech))
            .or_default()
            .push(entry);
    }

    let mut sorted_groups: Vec<_> = grouped_entries.into_iter().collect();
    sorted_groups.sort_by(|a, b| a.0.cmp(&b.0));

    for ((lemma_form, pos), entries_for_group) in sorted_groups {
        println!(
            "\n{} ~ {}",
            lemma_form.bold().cyan(),
            pos.to_string().italic()
        );

        // Print pronunciations
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
        for entry in entries_for_group {
            let senses = wn.get_senses_for_entry(&entry.id)?;
            for sense in senses {
                let start_sense_processing = Instant::now();
                match wn.get_synset(&sense.synset) {
                    Ok(synset) => {
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

/// Prints details for a single sense/synset combination.
fn print_sense_details(
    wn: &WordNet,
    current_lemma: &str,
    synset: &Synset,
    counter: usize,
) -> Result<()> {
    // Print definition(s)
    for def in &synset.definitions {
        println!("  {}: {}", counter.to_string().bold(), def.text.trim());
    }
    if let Some(ili_def) = &synset.ili_definition {
        println!("     ILI: {}", ili_def.text.trim().dimmed());
    }

    // Print examples
    if !synset.examples.is_empty() {
        for example in &synset.examples {
            println!("        {}", example.text.trim().italic());
        }
    }

    // Print synonyms
    let start_synonyms = Instant::now();
    let member_senses = wn.get_senses_for_synset(&synset.id)?;
    let mut synonyms = Vec::new();
    for member_sense in member_senses {
        if let Some(entry_id) = wn.get_entry_id_for_sense(&member_sense.id)? {
            if let Some(entry) = wn.get_entry_by_id(&entry_id)? {
                if entry.lemma.written_form != current_lemma
                    && !synonyms.contains(&entry.lemma.written_form)
                {
                    synonyms.push(entry.lemma.written_form.clone());
                }
            } else {
                warn!(
                    "Entry ID {} found in sense_to_entry_index but not in lexical_entries map.",
                    entry_id
                );
            }
        }
    }
    debug!(
        "Synonym lookup for synset {} took: {:?}",
        synset.id,
        start_synonyms.elapsed()
    );

    if !synonyms.is_empty() {
        synonyms.sort();
        println!(
            "        {}: {}",
            "Synonyms".magenta(),
            synonyms.join(", ").green()
        );
    }

    // Print selected relations
    print_relation(wn, synset, SenseRelType::Antonym, "Antonyms")?;
    print_relation(wn, synset, SynsetRelType::Hypernym, "Hypernyms")?;
    print_relation(wn, synset, SynsetRelType::Hyponym, "Hyponyms")?;

    println!();
    Ok(())
}

/// Prints lemmas for a specific relation type.
/// Handles both SenseRelations (like Antonym) and SynsetRelations (like Hypernym).
fn print_relation(
    wn: &WordNet,
    synset: &Synset,
    rel_type: impl Into<RelTypeMarker>,
    label: &str,
) -> Result<()> {
    let start_relation = Instant::now();
    let rel_type_marker = rel_type.into();
    let mut related_lemmas = Vec::new();

    match rel_type_marker {
        RelTypeMarker::Sense(sense_rel) => {
            let member_senses = wn.get_senses_for_synset(&synset.id)?;
            for member_sense in member_senses {
                let related_target_senses = wn.get_related_senses(&member_sense.id, sense_rel)?;
                for target_sense in related_target_senses {
                    if target_sense.synset != synset.id {
                        if let Some(entry_id) = wn.get_entry_id_for_sense(&target_sense.id)? {
                            if let Some(entry) = wn.get_entry_by_id(&entry_id)? {
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
            let related_synsets = wn.get_related_synsets(&synset.id, synset_rel)?;
            for target_synset in related_synsets {
                let target_senses = wn.get_senses_for_synset(&target_synset.id)?;
                for target_sense in target_senses {
                    if let Some(entry_id) = wn.get_entry_id_for_sense(&target_sense.id)? {
                        if let Some(entry) = wn.get_entry_by_id(&entry_id)? {
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

// Helper enum to dispatch between SenseRelType and SynsetRelType
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
