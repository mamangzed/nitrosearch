use crate::Filter;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Query {
    Term {
        field: String,
        term: String,
        boost: Option<f64>,
    },
    And {
        left: Box<Query>,
        right: Box<Query>,
    },
    Or {
        left: Box<Query>,
        right: Box<Query>,
    },
    Not {
        query: Box<Query>,
    },
    Phrase {
        field: String,
        terms: Vec<String>,
    },
    Prefix {
        field: String,
        prefix: String,
    },
    Wildcard {
        field: String,
        pattern: String,
    },
    Fuzzy {
        field: String,
        term: String,
        distance: u8,
    },
    All,
}

impl Query {
    pub fn term(field: &str, term: &str) -> Self {
        Self::Term {
            field: field.to_string(),
            term: term.to_string(),
            boost: None,
        }
    }

    pub fn phrase(field: &str, terms: Vec<String>) -> Self {
        Self::Phrase {
            field: field.to_string(),
            terms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: Query,
    pub filters: Vec<Filter>,
    pub sort_by: Option<String>,
    pub sort_order: SortOrder,
    pub limit: usize,
    pub offset: usize,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: Query::All,
            filters: Vec::new(),
            sort_by: None,
            sort_order: SortOrder::Desc,
            limit: 10,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SortOrder {
    Asc,
    Desc,
}
