use crate::error::{OewnError, Result};
use crate::models::LexicalResource;
use log::debug;
use quick_xml::de::from_str;
use tokio::task;

/// Parses WN-LMF XML content into a LexicalResource struct using spawn_blocking.
pub async fn parse_lmf(xml_content: String) -> Result<LexicalResource> {
    debug!("Starting WN-LMF XML parsing (using spawn_blocking)...");
    // Wrap the synchronous parsing in spawn_blocking
    let resource = task::spawn_blocking(move || -> Result<LexicalResource> {
        from_str(&xml_content).map_err(OewnError::from) // Map quick_xml::DeError to OewnError
    })
    .await??;
    debug!("Successfully parsed WN-LMF XML into LexicalResource.");
    Ok(resource)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic test with a minimal valid LMF structure
    const MINIMAL_LMF_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE LexicalResource SYSTEM "http://globalwordnet.github.io/schemas/WN-LMF-1.3.dtd">
<LexicalResource xmlns:dc="http://purl.org/dc/elements/1.1/">
  <Lexicon id="test-en"
           label="Test Wordnet (English)"
           language="en"
           email="test@example.com"
           license="https://example.com/license"
           version="1.0">
    <LexicalEntry id="w1">
      <Lemma writtenForm="cat" partOfSpeech="n"/>
      <Sense id="test-en-1-n-1" synset="test-en-1-n"/>
    </LexicalEntry>
    <Synset id="test-en-1-n" partOfSpeech="n" members="test-en-1-n-1">
      <Definition>A small domesticated carnivorous mammal.</Definition>
    </Synset>
  </Lexicon>
</LexicalResource>
"#;

    #[tokio::test]
    async fn test_parse_minimal_lmf() {
        let result = parse_lmf(MINIMAL_LMF_XML.to_string()).await;
        assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
        let resource = result.unwrap();
        assert_eq!(resource.lexicons.len(), 1);
        let lexicon = &resource.lexicons[0];
        assert_eq!(lexicon.id, "test-en");
        assert_eq!(lexicon.lexical_entries.len(), 1);
        assert_eq!(lexicon.synsets.len(), 1);
        assert_eq!(lexicon.lexical_entries[0].lemma.written_form, "cat");
        assert_eq!(
            lexicon.synsets[0].definitions[0].text,
            "A small domesticated carnivorous mammal."
        );
    }

    const LMF_WITH_PRONUNCIATION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE LexicalResource SYSTEM "http://globalwordnet.github.io/schemas/WN-LMF-1.3.dtd">
<LexicalResource xmlns:dc="http://purl.org/dc/elements/1.1/">
  <Lexicon id="test-en"
           label="Test Wordnet (English)"
           language="en"
           email="test@example.com"
           license="https://example.com/license"
           version="1.0">
    <LexicalEntry id="w1">
      <Lemma writtenForm="rabbit" partOfSpeech="n"/>
      <Pronunciation variety="en-GB-fonipa" audio="http://example.com/rabbit.flac">'ræbɪt</Pronunciation>
      <Pronunciation variety="en-US-fonipa" phonemic="false">'ɹæbɪt</Pronunciation>
      <Sense id="test-en-2-n-1" synset="test-en-2-n"/>
    </LexicalEntry>
    <Synset id="test-en-2-n" partOfSpeech="n" members="test-en-2-n-1">
      <Definition>A burrowing mammal.</Definition>
    </Synset>
  </Lexicon>
</LexicalResource>
"#;

    #[tokio::test]
    async fn test_parse_pronunciation() {
        let result = parse_lmf(LMF_WITH_PRONUNCIATION.to_string()).await;
        assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
        let resource = result.unwrap();
        let lexicon = &resource.lexicons[0];
        let entry = &lexicon.lexical_entries[0];
        assert_eq!(entry.pronunciations.len(), 2);
        assert_eq!(entry.pronunciations[0].variety, "en-GB-fonipa");
        assert_eq!(entry.pronunciations[0].text, "'ræbɪt");
        assert_eq!(
            entry.pronunciations[0].audio,
            Some("http://example.com/rabbit.flac".to_string())
        );
        assert!(entry.pronunciations[0].phonemic); // Default
        assert_eq!(entry.pronunciations[1].variety, "en-US-fonipa");
        assert!(!entry.pronunciations[1].phonemic);
        assert_eq!(entry.pronunciations[1].audio, None);
    }
}
