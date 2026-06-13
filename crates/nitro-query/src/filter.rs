use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Filter {
    Eq {
        field: String,
        value: FilterValue,
    },
    Neq {
        field: String,
        value: FilterValue,
    },
    Gt {
        field: String,
        value: FilterValue,
    },
    Gte {
        field: String,
        value: FilterValue,
    },
    Lt {
        field: String,
        value: FilterValue,
    },
    Lte {
        field: String,
        value: FilterValue,
    },
    In {
        field: String,
        values: Vec<FilterValue>,
    },
    Range {
        field: String,
        gte: Option<FilterValue>,
        lte: Option<FilterValue>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterValue {
    String(String),
    Number(i64),
    Float(f64),
    Boolean(bool),
}

impl Filter {
    pub fn eq(field: &str, value: &str) -> Self {
        Self::Eq {
            field: field.to_string(),
            value: FilterValue::String(value.to_string()),
        }
    }

    pub fn in_list(field: &str, values: Vec<&str>) -> Self {
        Self::In {
            field: field.to_string(),
            values: values
                .into_iter()
                .map(|v| FilterValue::String(v.to_string()))
                .collect(),
        }
    }
}
