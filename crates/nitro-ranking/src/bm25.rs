use std::collections::HashMap;

pub struct BM25Scorer {
    pub k1: f64,
    pub b: f64,
    pub avg_doc_len: f64,
    pub doc_count: usize,
    pub doc_lengths: HashMap<String, usize>,
    pub term_doc_freq: HashMap<String, usize>,
    // Field boosting support
    pub field_lengths: HashMap<String, HashMap<String, usize>>, // doc_id -> field -> length
    pub field_term_freq: HashMap<String, HashMap<String, HashMap<String, usize>>>, // doc_id -> field -> term -> freq
    pub field_weights: HashMap<String, f64>, // field -> weight
}

impl BM25Scorer {
    pub fn new(k1: f64, b: f64) -> Self {
        Self {
            k1,
            b,
            avg_doc_len: 0.0,
            doc_count: 0,
            doc_lengths: HashMap::new(),
            term_doc_freq: HashMap::new(),
            field_lengths: HashMap::new(),
            field_term_freq: HashMap::new(),
            field_weights: HashMap::new(),
        }
    }

    pub fn add_document(&mut self, doc_id: &str, length: usize) {
        self.doc_lengths.insert(doc_id.to_string(), length);
        self.doc_count += 1;
        self.update_avg_doc_len();
    }

    pub fn add_term_freq(&mut self, term: &str, doc_id: &str) {
        let key = format!("{}:{}", term, doc_id);
        *self.term_doc_freq.entry(key).or_insert(0) += 1;
    }

    /// Add field-specific document length
    pub fn add_field_length(&mut self, doc_id: &str, field: &str, length: usize) {
        self.field_lengths
            .entry(doc_id.to_string())
            .or_default()
            .insert(field.to_string(), length);
    }

    /// Add field-specific term frequency
    pub fn add_field_term_freq(&mut self, doc_id: &str, field: &str, term: &str, freq: usize) {
        self.field_term_freq
            .entry(doc_id.to_string())
            .or_default()
            .entry(field.to_string())
            .or_default()
            .insert(term.to_string(), freq);
    }

    /// Set weight for a field (default 1.0)
    pub fn set_field_weight(&mut self, field: &str, weight: f64) {
        self.field_weights.insert(field.to_string(), weight);
    }

    fn update_avg_doc_len(&mut self) {
        if self.doc_count == 0 {
            self.avg_doc_len = 0.0;
            return;
        }
        let total_len: usize = self.doc_lengths.values().sum();
        self.avg_doc_len = total_len as f64 / self.doc_count as f64;
    }

    pub fn score(&self, term: &str, doc_id: &str, term_freq: usize) -> f64 {
        let doc_len = self.doc_lengths.get(doc_id).copied().unwrap_or(0) as f64;
        let idf = self.idf(term);
        let tf = self.tf(term_freq, doc_len);
        idf * tf
    }

    /// Score with field boosting
    pub fn score_with_fields(&self, term: &str, doc_id: &str) -> f64 {
        if self.field_weights.is_empty() {
            // Fall back to non-field scoring
            let total_freq = self
                .field_term_freq
                .get(doc_id)
                .map(|fields| {
                    fields
                        .values()
                        .filter_map(|terms| terms.get(term))
                        .sum::<usize>()
                })
                .unwrap_or(0);
            return self.score(term, doc_id, total_freq);
        }

        let idf = self.idf(term);
        let mut total_score = 0.0;

        if let Some(field_data) = self.field_term_freq.get(doc_id) {
            for (field, term_freqs) in field_data {
                if let Some(&freq) = term_freqs.get(term) {
                    let field_len = self
                        .field_lengths
                        .get(doc_id)
                        .and_then(|fields| fields.get(field))
                        .copied()
                        .unwrap_or(0) as f64;

                    let weight = self.field_weights.get(field).copied().unwrap_or(1.0);
                    let tf = self.tf(freq, field_len);
                    total_score += weight * idf * tf;
                }
            }
        }

        total_score
    }

    fn idf(&self, term: &str) -> f64 {
        let df = self
            .term_doc_freq
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{}:", term)))
            .count() as f64;
        let n = self.doc_count as f64;
        ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
    }

    fn tf(&self, term_freq: usize, doc_len: f64) -> f64 {
        let numerator = (term_freq as f64) * (self.k1 + 1.0);
        let avg_len = if self.avg_doc_len > 0.0 {
            self.avg_doc_len
        } else {
            1.0
        };
        let denominator =
            term_freq as f64 + self.k1 * (1.0 - self.b + self.b * (doc_len / avg_len));
        numerator / denominator
    }
}

impl Default for BM25Scorer {
    fn default() -> Self {
        Self::new(1.2, 0.75)
    }
}
