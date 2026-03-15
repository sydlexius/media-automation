use crate::config::DetectionConfig;
use regex::Regex;

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
        let stem_l = stem.to_lowercase();
        for &word in word_tokens {
            if word.contains(stem_l.as_str()) {
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

impl DetectionEngine {
    #[allow(dead_code)]
    pub fn new(config: &DetectionConfig) -> Self {
        Self {
            r_stems: config.r_stems.clone(),
            r_exact_patterns: compile_exact_patterns(&config.r_exact),
            pg13_stems: config.pg13_stems.clone(),
            pg13_exact_patterns: compile_exact_patterns(&config.pg13_exact),
            false_positives: config
                .false_positives
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            g_genres: config.g_genres.iter().map(|s| s.to_lowercase()).collect(),
        }
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
        // stems are lowercased internally; tokens are already lowercase from tokenizer
        let tokens: Vec<&str> = vec!["shitty"];
        let stems = vec!["SHIT".into()]; // uppercase stem from config
        let fp: Vec<String> = vec![];
        let result = detect_stems(&tokens, &stems, &fp);
        assert_eq!(result, vec!["shitty"]);
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
