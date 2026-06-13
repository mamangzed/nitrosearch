use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub fields: HashMap<String, FieldValue>,
}

impl Document {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            fields: HashMap::new(),
        }
    }

    pub fn get(&self, field: &str) -> Option<&FieldValue> {
        self.fields.get(field)
    }

    pub fn set(&mut self, field: &str, value: FieldValue) {
        self.fields.insert(field.to_string(), value);
    }

    pub fn text_field(&self, field: &str) -> Option<String> {
        self.fields.get(field).and_then(|v| match v {
            FieldValue::Text(s) | FieldValue::Keyword(s) => Some(s.clone()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldValue {
    Text(String),
    Keyword(String),
    Number(i64),
    Float(f64),
    Boolean(bool),
    Date(String),
    Array(Vec<FieldValue>),
}

impl FieldValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            FieldValue::Text(s) | FieldValue::Keyword(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        match self {
            FieldValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FieldValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}
