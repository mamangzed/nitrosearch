use crate::query::Query;

pub struct QueryParser;

impl QueryParser {
    pub fn parse(query_str: &str) -> Query {
        let query_str = query_str.trim();
        if query_str.is_empty() {
            return Query::All;
        }

        // Check for phrase query: "rust engine"
        if query_str.starts_with('"') && query_str.ends_with('"') && query_str.len() > 2 {
            let phrase = &query_str[1..query_str.len() - 1];
            let terms: Vec<String> = phrase
                .split_whitespace()
                .map(|s| s.to_lowercase())
                .collect();
            return Query::Phrase {
                field: "_all".to_string(),
                terms,
            };
        }

        // Check for fuzzy query: rust~1
        if let Some(pos) = query_str.find('~') {
            let term = &query_str[..pos];
            let distance_str = &query_str[pos + 1..];
            if let Ok(distance) = distance_str.parse::<u8>() {
                return Query::Fuzzy {
                    field: "_all".to_string(),
                    term: term.to_lowercase(),
                    distance,
                };
            }
        }

        // Check for AND query: rust AND engine
        if let Some(pos) = query_str.find(" AND ") {
            let left = query_str[..pos].trim();
            let right = query_str[pos + 5..].trim();
            return Query::And {
                left: Box::new(Self::parse(left)),
                right: Box::new(Self::parse(right)),
            };
        }

        // Check for OR query: rust OR engine
        if let Some(pos) = query_str.find(" OR ") {
            let left = query_str[..pos].trim();
            let right = query_str[pos + 4..].trim();
            return Query::Or {
                left: Box::new(Self::parse(left)),
                right: Box::new(Self::parse(right)),
            };
        }

        // Check for NOT query: rust NOT engine
        if let Some(pos) = query_str.find(" NOT ") {
            let left = query_str[..pos].trim();
            let right = query_str[pos + 5..].trim();
            return Query::And {
                left: Box::new(Self::parse(left)),
                right: Box::new(Query::Not {
                    query: Box::new(Self::parse(right)),
                }),
            };
        }

        // Check for field-specific query with boost: title:rust^2.0
        if let Some(pos) = query_str.find(':') {
            let field = query_str[..pos].trim();
            let mut rest = query_str[pos + 1..].trim().to_string();
            let mut boost = None;

            if let Some(boost_pos) = rest.find('^') {
                if let Ok(b) = rest[boost_pos + 1..].parse::<f64>() {
                    boost = Some(b);
                    rest = rest[..boost_pos].to_string();
                }
            }

            // Check for wildcard/prefix pattern
            let term_lower = rest.to_lowercase();
            if term_lower.contains('*') {
                // If ends with *, treat as prefix search
                if term_lower.ends_with('*') && !term_lower.contains('?') {
                    return Query::Prefix {
                        field: field.to_string(),
                        prefix: term_lower.trim_end_matches('*').to_string(),
                    };
                } else {
                    // Otherwise treat as wildcard
                    return Query::Wildcard {
                        field: field.to_string(),
                        pattern: term_lower,
                    };
                }
            }

            return Query::Term {
                field: field.to_string(),
                term: term_lower,
                boost,
            };
        }

        // Check for wildcard/prefix pattern in default field
        let term_lower = query_str.to_lowercase();
        if term_lower.contains('*') {
            if term_lower.ends_with('*') && !term_lower.contains('?') {
                return Query::Prefix {
                    field: "_all".to_string(),
                    prefix: term_lower.trim_end_matches('*').to_string(),
                };
            } else {
                return Query::Wildcard {
                    field: "_all".to_string(),
                    pattern: term_lower,
                };
            }
        }

        // Default: simple term query
        Query::Term {
            field: "_all".to_string(),
            term: term_lower,
            boost: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_term() {
        let q = QueryParser::parse("rust");
        assert_eq!(
            q,
            Query::Term {
                field: "_all".to_string(),
                term: "rust".to_string(),
                boost: None
            }
        );
    }

    #[test]
    fn test_parse_phrase() {
        let q = QueryParser::parse("\"rust engine\"");
        assert_eq!(
            q,
            Query::Phrase {
                field: "_all".to_string(),
                terms: vec!["rust".to_string(), "engine".to_string()]
            }
        );
    }

    #[test]
    fn test_parse_fuzzy() {
        let q = QueryParser::parse("rust~1");
        assert_eq!(
            q,
            Query::Fuzzy {
                field: "_all".to_string(),
                term: "rust".to_string(),
                distance: 1
            }
        );
    }

    #[test]
    fn test_parse_and() {
        let q = QueryParser::parse("rust AND engine");
        match q {
            Query::And { left, right } => {
                assert_eq!(
                    *left,
                    Query::Term {
                        field: "_all".to_string(),
                        term: "rust".to_string(),
                        boost: None
                    }
                );
                assert_eq!(
                    *right,
                    Query::Term {
                        field: "_all".to_string(),
                        term: "engine".to_string(),
                        boost: None
                    }
                );
            }
            _ => panic!("Expected And"),
        }
    }

    #[test]
    fn test_parse_field() {
        let q = QueryParser::parse("title:rust");
        assert_eq!(
            q,
            Query::Term {
                field: "title".to_string(),
                term: "rust".to_string(),
                boost: None
            }
        );
    }
}
