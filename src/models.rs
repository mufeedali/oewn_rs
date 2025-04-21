use serde::{Deserialize, Serialize};

// --- Top Level ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexicalResource {
    #[serde(rename = "Lexicon", default)]
    pub lexicons: Vec<Lexicon>,
}

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
    // Dublin Core attributes (optional)
    #[serde(rename = "@dc:publisher", default)]
    pub dc_publisher: Option<String>,
    #[serde(rename = "@dc:contributor", default)]
    pub dc_contributor: Option<String>,
    // ... other dc elements as needed

    // Requires element for dependencies
    #[serde(rename = "Requires", default)]
    pub requires: Vec<Requires>,

    // Lexical Entries and Synsets
    #[serde(rename = "LexicalEntry", default)]
    pub lexical_entries: Vec<LexicalEntry>,
    #[serde(rename = "Synset", default)]
    pub synsets: Vec<Synset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requires {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@version")]
    pub version: String,
}

// --- Lexical Entry ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexicalEntry {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "Lemma")]
    pub lemma: Lemma,
    #[serde(rename = "Pronunciation", default)]
    pub pronunciations: Vec<Pronunciation>,
    #[serde(rename = "Sense", default)]
    pub senses: Vec<Sense>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lemma {
    #[serde(rename = "@writtenForm")]
    pub written_form: String,
    #[serde(rename = "@partOfSpeech")]
    pub part_of_speech: PartOfSpeech,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartOfSpeech {
    N, // Noun
    V, // Verb
    A, // Adjective
    R, // Adverb
    S, // Adjective Satellite
    C, // Conjunction
    P, // Adposition
    X, // Other
    U, // Unknown
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pronunciation {
    #[serde(rename = "@variety")]
    pub variety: String, // e.g., "en-GB-fonipa"
    #[serde(rename = "@notation", default)]
    pub notation: Option<String>, // e.g., "fonxsamp" or dialect info
    #[serde(rename = "@phonemic", default = "default_phonemic")]
    pub phonemic: bool, // Default true
    #[serde(rename = "@audio", default)]
    pub audio: Option<String>, // URL
    #[serde(rename = "$text")]
    pub text: String, // IPA text
}

fn default_phonemic() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sense {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@synset")]
    pub synset: String, // Reference to Synset ID
    #[serde(rename = "SenseRelation", default)]
    pub sense_relations: Vec<SenseRelation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SenseRelation {
    #[serde(rename = "@relType")]
    pub rel_type: SenseRelType,
    #[serde(rename = "@target")]
    pub target: String, // Reference to another Sense ID
}

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
    #[serde(other)] // Catch-all for any other relation types found
    Other,
}

// --- Synset ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Synset {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@ili", default)] // Optional, might be "in" or empty
    pub ili: Option<String>,
    #[serde(rename = "@partOfSpeech")]
    pub part_of_speech: PartOfSpeech,
    #[serde(rename = "@members", default)] // Space-separated list of Sense IDs
    pub members: String,
    #[serde(rename = "Definition", default)]
    pub definitions: Vec<Definition>,
    #[serde(rename = "ILIDefinition", default)]
    pub ili_definition: Option<ILIDefinition>,
    #[serde(rename = "SynsetRelation", default)]
    pub synset_relations: Vec<SynsetRelation>,
    #[serde(rename = "Example", default)]
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Definition {
    #[serde(rename = "@dc:source", default)]
    pub dc_source: Option<String>,
    #[serde(rename = "$text")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ILIDefinition {
    #[serde(rename = "@dc:source", default)]
    pub dc_source: Option<String>,
    #[serde(rename = "$text")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SynsetRelation {
    #[serde(rename = "@relType")]
    pub rel_type: SynsetRelType,
    #[serde(rename = "@target")]
    pub target: String, // Reference to another Synset ID
}

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
