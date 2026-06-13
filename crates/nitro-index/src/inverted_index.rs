use std::collections::HashMap;

pub struct InvertedIndex {
    index: HashMap<String, Vec<(String, Vec<u32>)>>,
}

impl InvertedIndex {
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
        }
    }

    pub fn index_document(&mut self, doc_id: &str, _field: &str, tokens: &[String]) {
        let mut positions: HashMap<String, Vec<u32>> = HashMap::new();

        for (pos, token) in tokens.iter().enumerate() {
            positions.entry(token.clone()).or_default().push(pos as u32);
        }

        for (token, pos_list) in positions {
            self.index
                .entry(token)
                .or_default()
                .push((doc_id.to_string(), pos_list));
        }
    }

    pub fn search(&self, term: &str) -> Vec<(String, Vec<u32>)> {
        self.index.get(term).cloned().unwrap_or_default()
    }

    pub fn search_phrase(&self, terms: &[String]) -> Vec<String> {
        if terms.is_empty() {
            return Vec::new();
        }

        let first_term = &terms[0];
        let first_results = self.search(first_term);

        if terms.len() == 1 {
            return first_results.into_iter().map(|(id, _)| id).collect();
        }

        let mut matching_docs = Vec::new();

        for (doc_id, first_positions) in first_results {
            let mut all_match = true;

            for (i, term) in terms.iter().enumerate().skip(1) {
                let term_results = self.search(term);
                let doc_positions = term_results
                    .into_iter()
                    .find(|(id, _)| id == &doc_id)
                    .map(|(_, pos)| pos)
                    .unwrap_or_default();

                let mut found = false;
                for first_pos in &first_positions {
                    if doc_positions.contains(&(first_pos + i as u32)) {
                        found = true;
                        break;
                    }
                }

                if !found {
                    all_match = false;
                    break;
                }
            }

            if all_match {
                matching_docs.push(doc_id);
            }
        }

        matching_docs
    }

    pub fn doc_count(&self, term: &str) -> usize {
        self.index.get(term).map(|v| v.len()).unwrap_or(0)
    }

    /// Search for terms within edit distance `max_distance` of `term`.
    pub fn search_fuzzy(&self, term: &str, max_distance: u8) -> Vec<(String, Vec<u32>)> {
        let mut results: HashMap<String, Vec<u32>> = HashMap::new();

        for (indexed_term, postings) in &self.index {
            if levenshtein(term, indexed_term) <= max_distance as usize {
                for (doc_id, positions) in postings {
                    results
                        .entry(doc_id.clone())
                        .or_default()
                        .extend_from_slice(positions);
                }
            }
        }

        results.into_iter().collect()
    }

    /// Get all indexed terms (for synonym expansion, etc.)
    pub fn terms(&self) -> Vec<&String> {
        self.index.keys().collect()
    }
}

/// Levenshtein distance between two strings.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for i in 1..=a_len {
        curr_row[0] = i;
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr_row[j] = std::cmp::min(
                std::cmp::min(prev_row[j] + 1, curr_row[j - 1] + 1),
                prev_row[j - 1] + cost,
            );
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

impl Default for InvertedIndex {
    fn default() -> Self {
        Self::new()
    }
}
