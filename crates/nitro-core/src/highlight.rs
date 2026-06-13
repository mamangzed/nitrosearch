use crate::document::Document;

/// Generate highlighted snippets for search results.
pub struct Highlighter {
    pub prefix: String,
    pub suffix: String,
    pub fragment_size: usize,
}

impl Highlighter {
    pub fn new() -> Self {
        Self {
            prefix: "<em>".to_string(),
            suffix: "</em>".to_string(),
            fragment_size: 150,
        }
    }

    pub fn with_style(prefix: &str, suffix: &str, fragment_size: usize) -> Self {
        Self {
            prefix: prefix.to_string(),
            suffix: suffix.to_string(),
            fragment_size,
        }
    }

    /// Generate highlights for a document based on matched terms.
    pub fn highlight(&self, doc: &Document, matched_terms: &[String]) -> Vec<String> {
        let mut highlights = Vec::new();

        for (field_name, field_value) in &doc.fields {
            if let Some(text) = field_value.as_text() {
                if let Some(highlight) = self.highlight_text(text, matched_terms) {
                    highlights.push(format!("{}: {}", field_name, highlight));
                }
            }
        }

        highlights
    }

    fn highlight_text(&self, text: &str, terms: &[String]) -> Option<String> {
        let text_lower = text.to_lowercase();
        let mut positions: Vec<(usize, usize)> = Vec::new();

        // Find all term positions
        for term in terms {
            let term_lower = term.to_lowercase();
            let mut start = 0;
            while let Some(pos) = text_lower[start..].find(&term_lower) {
                let abs_pos = start + pos;
                positions.push((abs_pos, abs_pos + term.len()));
                start = abs_pos + 1;
            }
        }

        if positions.is_empty() {
            return None;
        }

        // Sort and merge overlapping positions
        positions.sort_by_key(|&(start, _)| start);
        let mut merged = vec![positions[0]];
        for &(start, end) in &positions[1..] {
            let last = merged.last_mut().unwrap();
            if start <= last.1 {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }

        // Build highlighted text
        let mut result = String::new();
        let mut last_end = 0;

        // Find the best fragment window
        let first_match = merged[0].0;
        let window_start = first_match.saturating_sub(30);
        let window_end = (window_start + self.fragment_size).min(text.len());

        for &(start, end) in &merged {
            if start >= window_end {
                break;
            }
            if end <= window_start {
                continue;
            }

            let effective_start = start.max(window_start);
            let effective_end = end.min(window_end);

            if last_end < effective_start {
                result.push_str(&text[last_end..effective_start]);
            }
            result.push_str(&self.prefix);
            result.push_str(&text[effective_start..effective_end]);
            result.push_str(&self.suffix);
            last_end = effective_end;
        }

        if last_end < window_end {
            result.push_str(&text[last_end..window_end]);
        }

        Some(result)
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FieldValue;
    use std::collections::HashMap;

    #[test]
    fn test_highlight_basic() {
        let highlighter = Highlighter::new();
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), FieldValue::Text("Rust is awesome".to_string()));
        let doc = Document { id: "1".to_string(), fields };

        let highlights = highlighter.highlight(&doc, &["rust".to_string()]);
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].contains("<em>Rust</em>"));
    }

    #[test]
    fn test_highlight_multiple_terms() {
        let highlighter = Highlighter::new();
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), FieldValue::Text("Rust search engine".to_string()));
        let doc = Document { id: "1".to_string(), fields };

        let highlights = highlighter.highlight(&doc, &["rust".to_string(), "engine".to_string()]);
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].contains("<em>Rust</em>"));
        assert!(highlights[0].contains("<em>engine</em>"));
    }
}
