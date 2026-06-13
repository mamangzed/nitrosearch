//! Parallel search execution across segments using rayon
//!
//! Phase 4: Efficient search execution with parallel segment scanning

use crate::metrics::METRICS;
use nitro_core::{Document, Highlighter, SearchResult, SearchResults};
use nitro_index::Tokenizer;
use nitro_query::Query;
use nitro_ranking::BM25Scorer;
use nitro_storage::Segment;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

pub struct SearchExecutor {
    highlighter: Highlighter,
}

impl SearchExecutor {
    pub fn new() -> Self {
        Self {
            highlighter: Highlighter::new(),
        }
    }

    /// Execute query across multiple segments in parallel
    pub fn execute_parallel(
        &self,
        query: &Query,
        segments: &[Arc<Segment>],
        tokenizer: &Tokenizer,
        scorer: &BM25Scorer,
        limit: usize,
    ) -> SearchResults {
        let start = std::time::Instant::now();

        // Search all segments in parallel
        let segment_results: Vec<HashMap<String, (f64, Vec<String>)>> = segments
            .par_iter()
            .map(|segment| {
                let mut doc_scores = HashMap::new();
                let mut matched_terms = HashMap::new();

                Self::execute_query_on_segment(
                    query,
                    segment,
                    tokenizer,
                    scorer,
                    &mut doc_scores,
                    &mut matched_terms,
                );

                // Merge doc_scores and matched_terms
                doc_scores
                    .into_iter()
                    .map(|(doc_id, score)| {
                        let terms = matched_terms.remove(&doc_id).unwrap_or_default();
                        (doc_id, (score, terms))
                    })
                    .collect()
            })
            .collect();

        // Merge results from all segments
        let mut global_scores: HashMap<String, f64> = HashMap::new();
        let mut global_terms: HashMap<String, Vec<String>> = HashMap::new();

        for segment_result in segment_results {
            for (doc_id, (score, terms)) in segment_result {
                *global_scores.entry(doc_id.clone()).or_insert(0.0) += score;
                global_terms.entry(doc_id).or_default().extend(terms);
            }
        }

        // Build results with highlights
        let mut hits: Vec<SearchResult> = global_scores
            .into_iter()
            .map(|(doc_id, score)| {
                let doc = Document::new(&doc_id);
                let terms = global_terms.get(&doc_id).cloned().unwrap_or_default();
                let highlights = self.highlighter.highlight(&doc, &terms);

                SearchResult {
                    doc,
                    score,
                    highlights,
                }
            })
            .collect();

        // Sort by score descending
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

        // Truncate to limit
        hits.truncate(limit);

        let elapsed = start.elapsed().as_millis() as u64;
        METRICS.record_search_latency(elapsed);

        SearchResults {
            total: hits.len(),
            hits,
            time_ms: elapsed,
        }
    }

    fn execute_query_on_segment(
        query: &Query,
        segment: &Segment,
        tokenizer: &Tokenizer,
        scorer: &BM25Scorer,
        doc_scores: &mut HashMap<String, f64>,
        matched_terms: &mut HashMap<String, Vec<String>>,
    ) {
        match query {
            Query::Term { term, boost, .. } => {
                if let Ok(Some(doc_ids)) = segment.search_term(term) {
                    for doc_id in doc_ids {
                        let tf = 1; // Simplified, would calculate actual term frequency
                        let mut score = scorer.score(term, &doc_id.to_string(), tf);

                        // Apply field boost if present
                        if let Some(boost_value) = boost {
                            score *= boost_value;
                        }

                        *doc_scores.entry(doc_id.to_string()).or_insert(0.0) += score;
                        matched_terms
                            .entry(doc_id.to_string())
                            .or_default()
                            .push(term.clone());
                    }
                }
            }

            Query::And { left, right } => {
                let mut left_scores = HashMap::new();
                let mut left_matched = HashMap::new();
                Self::execute_query_on_segment(
                    left,
                    segment,
                    tokenizer,
                    scorer,
                    &mut left_scores,
                    &mut left_matched,
                );

                let mut right_scores = HashMap::new();
                let mut right_matched = HashMap::new();
                Self::execute_query_on_segment(
                    right,
                    segment,
                    tokenizer,
                    scorer,
                    &mut right_scores,
                    &mut right_matched,
                );

                // Intersect results
                for (doc_id, left_score) in left_scores {
                    if let Some(&right_score) = right_scores.get(&doc_id) {
                        *doc_scores.entry(doc_id.clone()).or_insert(0.0) +=
                            left_score + right_score;

                        let mut terms = left_matched.get(&doc_id).cloned().unwrap_or_default();
                        terms.extend(right_matched.get(&doc_id).cloned().unwrap_or_default());
                        matched_terms.insert(doc_id, terms);
                    }
                }
            }

            Query::Or { left, right } => {
                let mut left_scores = HashMap::new();
                let mut left_matched = HashMap::new();
                Self::execute_query_on_segment(
                    left,
                    segment,
                    tokenizer,
                    scorer,
                    &mut left_scores,
                    &mut left_matched,
                );

                let mut right_scores = HashMap::new();
                let mut right_matched = HashMap::new();
                Self::execute_query_on_segment(
                    right,
                    segment,
                    tokenizer,
                    scorer,
                    &mut right_scores,
                    &mut right_matched,
                );

                // Union results
                for (doc_id, score) in left_scores {
                    *doc_scores.entry(doc_id.clone()).or_insert(0.0) += score;
                    if let Some(terms) = left_matched.get(&doc_id) {
                        matched_terms
                            .entry(doc_id.clone())
                            .or_default()
                            .extend(terms.clone());
                    }
                }

                for (doc_id, score) in right_scores {
                    *doc_scores.entry(doc_id.clone()).or_insert(0.0) += score;
                    if let Some(terms) = right_matched.get(&doc_id) {
                        matched_terms
                            .entry(doc_id.clone())
                            .or_default()
                            .extend(terms.clone());
                    }
                }
            }

            Query::Not { query: inner } => {
                let mut inner_scores = HashMap::new();
                Self::execute_query_on_segment(
                    inner,
                    segment,
                    tokenizer,
                    scorer,
                    &mut inner_scores,
                    &mut HashMap::new(),
                );

                // Remove documents that matched inner query
                for doc_id in inner_scores.keys() {
                    doc_scores.remove(doc_id);
                    matched_terms.remove(doc_id);
                }
            }

            Query::Phrase { terms, .. } => {
                // Simplified phrase search: check if all terms appear in document
                if let Some(first_term) = terms.first() {
                    if let Ok(Some(doc_ids)) = segment.search_term(first_term) {
                        for doc_id in doc_ids {
                            // Verify all terms exist (simplified)
                            let all_terms_exist = terms.iter().all(|term| {
                                segment
                                    .search_term(term)
                                    .ok()
                                    .flatten()
                                    .map_or(false, |ids| ids.contains(&doc_id))
                            });

                            if all_terms_exist {
                                *doc_scores.entry(doc_id.to_string()).or_insert(0.0) += 1.0;
                                matched_terms
                                    .entry(doc_id.to_string())
                                    .or_default()
                                    .extend(terms.clone());
                            }
                        }
                    }
                }
            }

            Query::Fuzzy {
                term, distance: _, ..
            } => {
                // Simplified fuzzy: exact match for now
                // Full implementation would use Levenshtein distance on term dictionary
                if let Ok(Some(doc_ids)) = segment.search_term(term) {
                    for doc_id in doc_ids {
                        let score = scorer.score(term, &doc_id.to_string(), 1);
                        *doc_scores.entry(doc_id.to_string()).or_insert(0.0) += score;
                        matched_terms
                            .entry(doc_id.to_string())
                            .or_default()
                            .push(term.clone());
                    }
                }
            }

            _ => {}
        }
    }
}
