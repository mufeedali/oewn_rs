//! Data models for OEWN lexical structures.
//!
//! This module defines the core data structures that represent the
//! WordNet-LMF (Lexical Markup Framework) format, including lexical entries,
//! synsets, pronunciations, and various relationships between words.

use serde::{Deserialize, Serialize};

/// Root structure of a WordNet LMF document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexicalResource {
    #[serde(rename = "Lexicon", default)]
    pub lexicons: Vec<Lexicon>,
}

/// A lexicon containing lexical entries and synsets for a specific language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lexicon {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@label")]
    pub label: String,
    #[serde(rename = "@language")]
    pub language: String,
    #[serde(rename = "@email")]
    pub email: String,
    #[serde(rename = "@license")]
    pub license: String,
    #[serde(rename = "@version")]
    pub version: String,
    #[serde(rename = "@url", default)]
    pub url: Option<String>,
    #[serde(rename = "@citation", default)]
    pub citation: Option<String>,
    #[serde(rename = "@logo", default)]
    pub logo: Option<String>,
    #[serde(rename = "@status", default)]
    pub status: Option<String>,
    #[serde(rename = "@confidenceScore", default)]
    pub confidence_score: Option<f32>,
    /// Dublin Core publisher information
    #[serde(rename = "@dc:publisher", default)]
    pub dc_publisher: Option<String>,
    /// Dublin Core contributor information
    #[serde(rename = "@dc:contributor", default)]
    pub dc_contributor: Option<String>,

    /// Dependencies required by this lexicon
    #[serde(rename = "Requires", default)]
    pub requires: Vec<Requires>,

    /// Lexical entries contained in this lexicon
    #[serde(rename = "LexicalEntry", default)]
    pub lexical_entries: Vec<LexicalEntry>,
    /// Synsets contained in this lexicon
    #[serde(rename = "Synset", default)]
    pub synsets: Vec<Synset>,
}

/// Dependency requirement for a lexicon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requires {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@version")]
    pub version: String,
}

/// A lexical entry representing a word form with its senses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexicalEntry {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "Lemma")]
    pub lemma: Lemma,
    /// Pronunciation variants for this entry
    #[serde(rename = "Pronunciation", default)]
    pub pronunciations: Vec<Pronunciation>,
    /// Senses associated with this entry
    #[serde(rename = "Sense", default)]
    pub senses: Vec<Sense>,
}

/// The canonical form of a lexical entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lemma {
    #[serde(rename = "@writtenForm")]
    pub written_form: String,
    #[serde(rename = "@partOfSpeech")]
    pub part_of_speech: PartOfSpeech,
}

/// Part-of-speech enumeration following WordNet conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartOfSpeech {
    N, // Noun (e.g., "dog", "happiness")
    V, // Verb (e.g., "run", "eat")
    A, // Adjective (e.g., "big", "happy")
    R, // Adverb (e.g., "quickly")
    S, // Adjective Satellite (e.g., "biggest")
    C, // Conjunction (e.g., "and", "or")
    P, // Adposition (e.g., prepositions like "in", "on")
    X, // Other (e.g., interjections, particles)
    U, // Unknown (used when part of speech is not specified)
}

/// Pronunciation information for a lexical entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pronunciation {
    /// Language/dialect variety (e.g., "en-GB-fonipa")
    #[serde(rename = "@variety")]
    pub variety: String,
    /// Notation system or dialect information
    #[serde(rename = "@notation", default)]
    pub notation: Option<String>,
    /// Whether this is a phonemic transcription
    #[serde(rename = "@phonemic", default = "default_phonemic")]
    pub phonemic: bool,
    /// Audio file URL
    #[serde(rename = "@audio", default)]
    pub audio: Option<String>,
    /// IPA transcription text
    #[serde(rename = "$text")]
    pub text: String,
}

/// Default value for phonemic field.
fn default_phonemic() -> bool {
    true
}

/// A sense connecting a lexical entry to a synset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sense {
    #[serde(rename = "@id")]
    pub id: String,
    /// Reference to the synset this sense belongs to
    #[serde(rename = "@synset")]
    pub synset: String,
    /// Relations to other senses
    #[serde(rename = "SenseRelation", default)]
    pub sense_relations: Vec<SenseRelation>,
}

/// A relationship between senses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SenseRelation {
    #[serde(rename = "@relType")]
    pub rel_type: SenseRelType,
    /// Reference to the target sense
    #[serde(rename = "@target")]
    pub target: String,
}

/// Types of relationships between senses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenseRelType {
    Antonym,
    Also,
    Participle,
    Pertainym,
    Derivation,
    DomainTopic,
    DomainMemberTopic,
    DomainRegion,
    DomainMemberRegion,
    Exemplifies,
    IsExemplifiedBy,
    /// Catch-all for any other relation types found
    #[serde(other)]
    Other,
}

/// A synset (synonym set) representing a concept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Synset {
    #[serde(rename = "@id")]
    pub id: String,
    /// Optional Inter-Lingual Index identifier
    #[serde(rename = "@ili", default)]
    pub ili: Option<String>,
    #[serde(rename = "@partOfSpeech")]
    pub part_of_speech: PartOfSpeech,
    /// Space-separated list of sense IDs that belong to this synset
    #[serde(rename = "@members", default)]
    pub members: String,
    /// Definitions for this synset
    #[serde(rename = "Definition", default)]
    pub definitions: Vec<Definition>,
    /// Optional ILI definition
    #[serde(rename = "ILIDefinition", default)]
    pub ili_definition: Option<ILIDefinition>,
    /// Relations to other synsets
    #[serde(rename = "SynsetRelation", default)]
    pub synset_relations: Vec<SynsetRelation>,
    /// Usage examples for this synset
    #[serde(rename = "Example", default)]
    pub examples: Vec<Example>,
}

/// A definition of a synset.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Definition {
    #[serde(rename = "@dc:source", default)]
    pub dc_source: Option<String>,
    #[serde(rename = "$text")]
    pub text: String,
}

/// An Inter-Lingual Index definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ILIDefinition {
    #[serde(rename = "@dc:source", default)]
    pub dc_source: Option<String>,
    #[serde(rename = "$text")]
    pub text: String,
}

/// A relationship between synsets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SynsetRelation {
    #[serde(rename = "@relType")]
    pub rel_type: SynsetRelType,
    /// Reference to the target synset
    #[serde(rename = "@target")]
    pub target: String,
}

/// Types of relationships between synsets.
///
/// This enum covers both Princeton WordNet relations and extended relations
/// from the WordNet-LMF specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynsetRelType {
    // Princeton WordNet Properties
    Hypernym,
    Hyponym,
    InstanceHypernym,
    InstanceHyponym,
    MeroMember,
    MeroPart,
    MeroSubstance,
    HoloMember,
    HoloPart,
    HoloSubstance,
    Entails,
    Causes,
    Similar, // Note: 'similar_to' in WNDB, 'similar' in LMF spec text
    Attribute,
    DomainRegion,
    DomainTopic,
    HasDomainRegion,
    HasDomainTopic,
    Exemplifies,
    IsExemplifiedBy,

    // Non-Princeton WordNet Relations (from spec text)
    Agent,
    Also,
    AntoConverse,
    AntoGradable,
    AntoSimple,
    Antonym,
    Augmentative,
    BeInState,
    ClassifiedBy,
    Classifies,
    CoAgentInstrument,
    CoAgentPatient,
    CoAgentResult,
    CoInstrumentAgent,
    CoInstrumentPatient,
    CoInstrumentResult,
    CoPatientAgent,
    CoPatientInstrument,
    CoResultAgent,
    CoResultInstrument,
    CoRole,
    Constitutive,
    Derivation,
    Diminutive,
    Direction,
    Domain, // General domain
    EqSynonym,
    Feminine,
    HasAugmentative,
    HasDiminutive,
    HasDomain, // General has_domain
    HasFeminine,
    HasMasculine,
    HasYoung,
    HoloLocation,
    HoloPortion,
    Holonym, // General holonym
    InManner,
    Instrument,
    Involved,
    InvolvedAgent,
    InvolvedDirection,
    InvolvedInstrument,
    InvolvedLocation,
    InvolvedPatient,
    InvolvedResult,
    InvolvedSourceDirection,
    InvolvedTargetDirection,
    IrSynonym,
    IsCausedBy,
    IsEntailedBy,
    IsSubeventOf,
    Location,
    MannerOf,
    Masculine,
    MeroLocation,
    MeroPortion,
    Meronym, // General meronym
    Other,   // Explicitly listed
    Participle,
    Patient,
    Pertainym,
    RestrictedBy,
    Restricts,
    Result,
    Role,
    SecondaryAspectIp,
    SecondaryAspectPi,
    SimpleAspectIp,
    SimpleAspectPi,
    SourceDirection,
    StateOf,
    Subevent,
    TargetDirection,
    Young,

    // Catch-all for any others not explicitly listed or typos
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Example {
    #[serde(rename = "@dc:source", default)]
    pub dc_source: Option<String>,
    #[serde(rename = "$text")]
    pub text: String,
}

// Helper function for parsing space-separated member lists
pub fn parse_members(members_str: &str) -> Vec<String> {
    members_str.split_whitespace().map(String::from).collect()
}

// Implement Display for PartOfSpeech for easier printing
impl std::fmt::Display for PartOfSpeech {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                PartOfSpeech::N => "noun",
                PartOfSpeech::V => "verb",
                PartOfSpeech::A => "adjective",
                PartOfSpeech::R => "adverb",
                PartOfSpeech::S => "adjective satellite",
                PartOfSpeech::C => "conjunction",
                PartOfSpeech::P => "adposition",
                PartOfSpeech::X => "other",
                PartOfSpeech::U => "unknown",
            }
        )
    }
}

// Implement FromStr for PartOfSpeech for CLI parsing etc.
impl std::str::FromStr for PartOfSpeech {
    type Err = String; // Simple error type
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "n" | "noun" => Ok(PartOfSpeech::N),
            "v" | "verb" => Ok(PartOfSpeech::V),
            "a" | "adj" | "adjective" => Ok(PartOfSpeech::A),
            "r" | "adv" | "adverb" => Ok(PartOfSpeech::R),
            "s" | "adj_sat" | "adjective_satellite" => Ok(PartOfSpeech::S),
            "c" | "conj" | "conjunction" => Ok(PartOfSpeech::C),
            "p" | "adp" | "adposition" => Ok(PartOfSpeech::P),
            "x" | "other" => Ok(PartOfSpeech::X),
            "u" | "unknown" => Ok(PartOfSpeech::U),
            _ => Err(format!("Invalid part of speech: {}", s)),
        }
    }
}
