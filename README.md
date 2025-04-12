# oewn_rs

A command-line interface and library for accessing and querying the [Open English WordNet (OEWN)](https://github.com/globalwordnet/english-wordnet) database, written in Rust.

## Features

*   Look up definitions, examples, synonyms, antonyms, hypernyms, and hyponyms for English words.
*   Filter word lookups by part of speech (noun, verb, adjective, adverb).
*   Display a random word entry from the database.
*   Automatically downloads the latest OEWN data (in LMF XML format).
*   Caches the processed data locally in an efficient SQLite database for fast subsequent lookups.

## Installation

### Using Cargo

If you have Rust and Cargo installed, you can install `oewn_rs` directly from the source:

```bash
git clone https://github.com/mufeedali/oewn_rs
cd oewn_rs
cargo install --path .
```

## Usage

The basic command structure is:

```bash
oewn_rs [OPTIONS] <COMMAND>
```

### Global Options

*   `--db-path <PATH>`: Use a specific SQLite database file instead of the default location.
*   `--force-reload`: Download and process the OEWN data again, even if a database file exists.
*   `-v, --verbose`: Increase output verbosity (use `-vv` for more detail).

### Commands

#### `define`

Look up a word.

```bash
# Define the word "rust" (all parts of speech)
oewn_rs define rust

# Define the word "run" only as a verb
oewn_rs define run --pos verb

# Define "set" as a noun, using a custom DB and forcing reload
oewn_rs --db-path /path/to/my/oewn.db --force-reload define set --pos noun
```

#### `random`

Show a random word entry.

```bash
oewn_rs random
```

#### `clear-db`

Remove the local OEWN database cache.

```bash
# Clear the default database
oewn_rs clear-db

# Clear a custom database
oewn_rs --db-path /path/to/my/oewn.db clear-db
```

## Data Source

This tool uses data from the [Open English WordNet](https://github.com/globalwordnet/english-wordnet), which is distributed under the [CC BY 4.0 license](https://creativecommons.org/licenses/by/4.0/). The data is downloaded in LMF XML format and processed into a local SQLite database upon first run (or when `--force-reload` is used).

## Building from Source

1.  Clone the repository: `git clone https://github.com/mufeedali/oewn_rs`
2.  Navigate to the directory: `cd oewn_rs`
3.  Build the project: `cargo build --release`
    The executable will be located at `target/release/oewn_rs`.

## License

This project is licensed under the GNU General Public License v3.0 or later - see the [LICENSE](LICENSE) file for details.
