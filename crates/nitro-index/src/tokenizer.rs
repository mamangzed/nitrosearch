use std::collections::HashSet;

pub struct Tokenizer {
    stop_words: HashSet<String>,
}

impl Tokenizer {
    pub fn new(stop_words: HashSet<String>) -> Self {
        Self { stop_words }
    }

    pub fn english() -> Self {
        let stop_words: HashSet<String> = [
            "the", "is", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of",
            "with", "by", "from", "as", "if", "then", "so", "not", "be", "have",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        Self::new(stop_words)
    }

    pub fn indonesian() -> Self {
        let stop_words: HashSet<String> = [
            "yang", "dan", "atau", "di", "ke", "dari", "pada", "untuk", "dengan", "tidak", "ada",
            "ini", "itu", "juga", "sudah", "akan", "bisa", "dapat",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        Self::new(stop_words)
    }

    pub fn tokenize(&self, text: &str) -> Vec<String> {
        text.to_lowercase()
            .split_whitespace()
            .map(|s| {
                s.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty() && !self.stop_words.contains(s))
            .collect()
    }
}

pub struct Stemmer;

impl Stemmer {
    pub fn stem_english(word: &str) -> String {
        let word = word.to_string();
        if word.ends_with("ing") {
            return word[..word.len() - 3].to_string();
        }
        if word.ends_with("ed") {
            return word[..word.len() - 2].to_string();
        }
        if word.ends_with("s") && word.len() > 3 {
            return word[..word.len() - 1].to_string();
        }
        word
    }

    pub fn stem_indonesian(word: &str) -> String {
        let word = word.to_string();
        if word.starts_with("me") && word.len() > 4 {
            return word[2..].to_string();
        }
        if word.starts_with("ber") && word.len() > 5 {
            return word[3..].to_string();
        }
        if word.ends_with("kan") && word.len() > 5 {
            return word[..word.len() - 3].to_string();
        }
        word
    }
}
