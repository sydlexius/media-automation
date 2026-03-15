use crate::config::DetectionConfig;
use regex::Regex;
use std::sync::LazyLock;

#[allow(dead_code)]
static WORD_TOKENIZER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z']+").unwrap());

#[allow(dead_code)]
pub struct DetectionEngine {
    r_stems: Vec<String>,
    r_exact_patterns: Vec<(String, Regex)>,
    pg13_stems: Vec<String>,
    pg13_exact_patterns: Vec<(String, Regex)>,
    false_positives: Vec<String>,
    g_genres: Vec<String>,
}

#[allow(dead_code)]
fn compile_exact_patterns(words: &[String]) -> Vec<(String, Regex)> {
    words
        .iter()
        .map(|w| {
            let pattern = format!(r"(?i)\b{}\b", regex::escape(w));
            (w.clone(), Regex::new(&pattern).unwrap())
        })
        .collect()
}

#[allow(dead_code)]
fn detect_stems(word_tokens: &[&str], stems: &[String], false_positives: &[String]) -> Vec<String> {
    let mut matched = Vec::new();
    for stem in stems {
        for &word in word_tokens {
            if word.contains(stem.as_str()) {
                let is_fp = false_positives.iter().any(|fp| word.contains(fp.as_str()));
                if !is_fp {
                    matched.push(word.to_string());
                    break;
                }
            }
        }
    }
    matched
}

#[allow(dead_code)]
fn detect_exact(text: &str, patterns: &[(String, Regex)]) -> Vec<String> {
    patterns
        .iter()
        .filter(|(_, pat)| pat.is_match(text))
        .map(|(word, _)| word.clone())
        .collect()
}

#[allow(dead_code)]
fn dedup_matched(stem_hits: Vec<String>, exact_hits: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    for word in stem_hits.into_iter().chain(exact_hits) {
        if !result.contains(&word) {
            result.push(word);
        }
    }
    result
}

impl DetectionEngine {
    #[allow(dead_code)]
    pub fn new(config: &DetectionConfig) -> Self {
        Self {
            r_stems: config.r_stems.iter().map(|s| s.to_lowercase()).collect(),
            r_exact_patterns: compile_exact_patterns(&config.r_exact),
            pg13_stems: config.pg13_stems.iter().map(|s| s.to_lowercase()).collect(),
            pg13_exact_patterns: compile_exact_patterns(&config.pg13_exact),
            false_positives: config
                .false_positives
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            g_genres: config.g_genres.iter().map(|s| s.to_lowercase()).collect(),
        }
    }

    #[allow(dead_code)]
    pub fn match_g_genre<'a>(&self, genres: &'a [String]) -> Option<&'a str> {
        for genre in genres {
            if self.g_genres.contains(&genre.to_lowercase()) {
                return Some(genre.as_str());
            }
        }
        None
    }

    #[allow(dead_code)]
    pub fn classify_lyrics(&self, text: &str) -> (Option<&'static str>, Vec<String>) {
        if text.trim().is_empty() {
            return (None, vec![]);
        }

        let lowered = text.to_lowercase();
        let word_tokens: Vec<&str> = WORD_TOKENIZER
            .find_iter(&lowered)
            .map(|m| m.as_str())
            .collect();

        // Check R tier first
        let r_stem_hits = detect_stems(&word_tokens, &self.r_stems, &self.false_positives);
        let r_exact_hits = detect_exact(text, &self.r_exact_patterns);
        if !r_stem_hits.is_empty() || !r_exact_hits.is_empty() {
            return (Some("R"), dedup_matched(r_stem_hits, r_exact_hits));
        }

        // Then PG-13
        let pg13_stem_hits = detect_stems(&word_tokens, &self.pg13_stems, &self.false_positives);
        let pg13_exact_hits = detect_exact(text, &self.pg13_exact_patterns);
        if !pg13_stem_hits.is_empty() || !pg13_exact_hits.is_empty() {
            return (
                Some("PG-13"),
                dedup_matched(pg13_stem_hits, pg13_exact_hits),
            );
        }

        (None, vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> DetectionConfig {
        DetectionConfig {
            r_stems: vec!["fuck".into(), "shit".into()],
            r_exact: vec!["blowjob".into()],
            pg13_stems: vec!["bitch".into()],
            pg13_exact: vec!["hoe".into()],
            false_positives: vec!["Cocktail".into()],
            g_genres: vec!["Classical".into(), "Ambient".into()],
        }
    }

    #[test]
    fn stems_basic_match() {
        let tokens: Vec<&str> = vec!["motherfucker", "is", "a", "word"];
        let stems = vec!["fuck".into()];
        let fp: Vec<String> = vec![];
        let result = detect_stems(&tokens, &stems, &fp);
        assert_eq!(result, vec!["motherfucker"]);
    }

    #[test]
    fn stems_false_positive_filtered() {
        let tokens: Vec<&str> = vec!["cocktail", "party"];
        let stems = vec!["cock".into()];
        let fp = vec!["cocktail".into()];
        let result = detect_stems(&tokens, &stems, &fp);
        assert!(result.is_empty());
    }

    #[test]
    fn stems_no_matches() {
        let tokens: Vec<&str> = vec!["hello", "world"];
        let stems = vec!["fuck".into()];
        let fp: Vec<String> = vec![];
        let result = detect_stems(&tokens, &stems, &fp);
        assert!(result.is_empty());
    }

    #[test]
    fn stems_one_match_per_stem() {
        let tokens: Vec<&str> = vec!["fuck", "fucker", "fucking"];
        let stems = vec!["fuck".into()];
        let fp: Vec<String> = vec![];
        let result = detect_stems(&tokens, &stems, &fp);
        assert_eq!(result, vec!["fuck"]); // first match only
    }

    #[test]
    fn stems_multiple_stems() {
        let tokens: Vec<&str> = vec!["bullshit", "fucker"];
        let stems = vec!["shit".into(), "fuck".into()];
        let fp: Vec<String> = vec![];
        let result = detect_stems(&tokens, &stems, &fp);
        assert_eq!(result, vec!["bullshit", "fucker"]);
    }

    #[test]
    fn stems_case_handling() {
        // stems are pre-lowercased by DetectionEngine::new; verify via the engine
        let mut config = test_config();
        config.r_stems = vec!["SHIT".into()]; // uppercase in config
        let engine = DetectionEngine::new(&config);
        // constructor lowercases stems, so classify_lyrics should still match
        let (tier, words) = engine.classify_lyrics("shitty behavior");
        assert_eq!(tier, Some("R"));
        assert!(words.contains(&"shitty".to_string()));
    }

    #[test]
    fn exact_word_boundary() {
        let patterns = compile_exact_patterns(&vec!["hoe".into()]);
        let result = detect_exact("garden hoe for sale", &patterns);
        assert_eq!(result, vec!["hoe"]);
    }

    #[test]
    fn exact_no_partial_match() {
        let patterns = compile_exact_patterns(&vec!["hoe".into()]);
        let result = detect_exact("nice shoes", &patterns);
        assert!(result.is_empty());
    }

    #[test]
    fn exact_case_insensitive() {
        let patterns = compile_exact_patterns(&vec!["blowjob".into()]);
        let result = detect_exact("a BLOWJOB reference", &patterns);
        assert_eq!(result, vec!["blowjob"]); // returns original word, not match text
    }

    #[test]
    fn exact_multiple_matches() {
        let patterns = compile_exact_patterns(&vec!["hoe".into(), "piss".into()]);
        let result = detect_exact("hoe and piss", &patterns);
        assert_eq!(result, vec!["hoe", "piss"]);
    }

    #[test]
    fn exact_no_matches() {
        let patterns = compile_exact_patterns(&vec!["blowjob".into()]);
        let result = detect_exact("clean text here", &patterns);
        assert!(result.is_empty());
    }

    #[test]
    fn classify_r_tier() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("this is fucking great");
        assert_eq!(tier, Some("R"));
        assert!(words.contains(&"fucking".to_string()));
    }

    #[test]
    fn classify_pg13_tier() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("you are a bitch");
        assert_eq!(tier, Some("PG-13"));
        assert!(words.contains(&"bitch".to_string()));
    }

    #[test]
    fn classify_r_priority_over_pg13() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, _words) = engine.classify_lyrics("fuck that bitch");
        assert_eq!(tier, Some("R")); // R takes priority, PG-13 not checked
    }

    #[test]
    fn classify_clean() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("beautiful sunny day");
        assert_eq!(tier, None);
        assert!(words.is_empty());
    }

    #[test]
    fn classify_empty() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("");
        assert_eq!(tier, None);
        assert!(words.is_empty());
    }

    #[test]
    fn classify_whitespace_only() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("   \n\t  ");
        assert_eq!(tier, None);
        assert!(words.is_empty());
    }

    #[test]
    fn classify_exact_match_r() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("that was a blowjob");
        assert_eq!(tier, Some("R"));
        assert!(words.contains(&"blowjob".to_string()));
    }

    #[test]
    fn classify_mixed_dedup() {
        // "fuck" matches as stem AND "fuck" could match as exact if it were in the list
        // Test that stem+exact results are deduped
        let config = DetectionConfig {
            r_stems: vec!["fuck".into()],
            r_exact: vec!["fuck".into()], // same word in both lists
            pg13_stems: vec![],
            pg13_exact: vec![],
            false_positives: vec![],
            g_genres: vec![],
        };
        let engine = DetectionEngine::new(&config);
        let (_tier, words) = engine.classify_lyrics("fuck this");
        // "fuck" should appear only once despite matching both stem and exact
        assert_eq!(words.iter().filter(|w| *w == "fuck").count(), 1);
    }

    #[test]
    fn classify_false_positive_no_trigger() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("enjoy a cocktail tonight");
        assert_eq!(tier, None);
        assert!(words.is_empty());
    }

    #[test]
    fn classify_non_ascii_passthrough() {
        let engine = DetectionEngine::new(&test_config());
        let (tier, words) = engine.classify_lyrics("café résumé naïve");
        assert_eq!(tier, None);
        assert!(words.is_empty());
    }

    #[test]
    fn genre_match() {
        let engine = DetectionEngine::new(&test_config());
        let genres = vec!["Classical".into(), "Rock".into()];
        assert_eq!(engine.match_g_genre(&genres), Some("Classical"));
    }

    #[test]
    fn genre_no_match() {
        let engine = DetectionEngine::new(&test_config());
        let genres = vec!["Rock".into(), "Metal".into()];
        assert_eq!(engine.match_g_genre(&genres), None);
    }

    #[test]
    fn genre_empty_genres() {
        let engine = DetectionEngine::new(&test_config());
        let genres: Vec<String> = vec![];
        assert_eq!(engine.match_g_genre(&genres), None);
    }

    #[test]
    fn genre_empty_g_genres() {
        let config = DetectionConfig {
            r_stems: vec![],
            r_exact: vec![],
            pg13_stems: vec![],
            pg13_exact: vec![],
            false_positives: vec![],
            g_genres: vec![], // no genres configured
        };
        let engine = DetectionEngine::new(&config);
        let genres = vec!["Classical".into()];
        assert_eq!(engine.match_g_genre(&genres), None);
    }

    #[test]
    fn genre_first_match_wins() {
        let engine = DetectionEngine::new(&test_config());
        // Both "Ambient" and "Classical" are in g_genres
        let genres = vec!["Ambient".into(), "Classical".into()];
        assert_eq!(engine.match_g_genre(&genres), Some("Ambient")); // first in item's list
    }

    #[test]
    fn engine_construction() {
        let config = test_config();
        let engine = DetectionEngine::new(&config);
        assert_eq!(engine.r_stems.len(), 2);
        assert_eq!(engine.r_exact_patterns.len(), 1);
        assert_eq!(engine.pg13_stems.len(), 1);
        assert_eq!(engine.pg13_exact_patterns.len(), 1);
        // false positives should be lowercased
        assert_eq!(engine.false_positives, vec!["cocktail"]);
        // g_genres should be lowercased
        assert_eq!(engine.g_genres, vec!["classical", "ambient"]);
    }
}
