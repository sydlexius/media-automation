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
