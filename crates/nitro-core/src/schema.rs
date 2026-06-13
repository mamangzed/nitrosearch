use serde::{Deserialize, Serialize};
use crate::Document;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub fields: Vec<FieldDef>,
    #[serde(default)]
    pub synonyms: HashMap<String, Vec<String>>,
}

impl Schema {
    pub fn new(fields: Vec<FieldDef>) -> Self {
        Self { fields, synonyms: HashMap::new() }
    }

    pub fn with_synonyms(fields: Vec<FieldDef>, synonyms: HashMap<String, Vec<String>>) -> Self {
        Self { fields, synonyms }
    }

    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn default() -> Self {
        Self { fields: Vec::new(), synonyms: HashMap::new() }
    }

    pub fn expand_synonyms(&self, query: &str) -> String {
        let mut expanded = query.to_string();
        for (term, syns) in &self.synonyms {
            if expanded.contains(term) {
                let syns_str = syns.join(" OR ");
                expanded = expanded.replace(term, &format!("({} OR {})", term, syns_str));
            }
        }
        expanded
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub field_type: FieldType,
    pub indexed: bool,
    pub stored: bool,
}

impl FieldDef {
    pub fn text(name: &str) -> Self {
        Self {
            name: name.to_string(),
            field_type: FieldType::Text,
            indexed: true,
            stored: true,
        }
    }

    pub fn keyword(name: &str) -> Self {
        Self {
            name: name.to_string(),
            field_type: FieldType::Keyword,
            indexed: true,
            stored: true,
        }
    }

    pub fn number(name: &str) -> Self {
        Self {
            name: name.to_string(),
            field_type: FieldType::Number,
            indexed: true,
            stored: true,
        }
    }

    pub fn boolean(name: &str) -> Self {
        Self {
            name: name.to_string(),
            field_type: FieldType::Boolean,
            indexed: true,
            stored: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FieldType {
    Text,
    Keyword,
    Number,
    Float,
    Boolean,
    Date,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    pub schema: Schema,
    pub config: CollectionConfig,
}

impl Collection {
    pub fn new(name: &str, schema: Schema) -> Self {
        Self {
            name: name.to_string(),
            schema,
            config: CollectionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    pub primary_key: String,
    pub default_search_fields: Vec<String>,
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            primary_key: "id".to_string(),
            default_search_fields: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub doc: Document,
    pub score: f64,
    pub highlights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub hits: Vec<SearchResult>,
    pub total: usize,
    pub time_ms: u64,
}
