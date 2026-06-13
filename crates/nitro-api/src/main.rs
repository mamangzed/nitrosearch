mod engine;
mod metrics;
mod persistence;
mod search_executor;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use moka::future::Cache;
use nitro_core::{Collection, Document, FieldValue, Schema};
use persistence::PersistenceManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing_subscriber::EnvFilter;

use crate::engine::SearchEngine;

#[derive(Parser)]
#[command(name = "nitro")]
#[command(about = "NitroSearch - Lightweight Search Engine Without JVM")]
struct Cli {
    #[arg(short, long, default_value = "http://localhost:8080")]
    url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the NitroSearch server
    Start {
        #[arg(short, long, default_value = "./data")]
        data_dir: String,
        #[arg(short, long, default_value = "0.0.0.0:8080")]
        bind: String,
    },

    /// Create a collection
    Create {
        #[arg(short, long)]
        name: String,
    },

    /// Index a JSON file
    Index {
        #[arg(short, long)]
        file: String,
        #[arg(short, long)]
        collection: String,
    },

    /// Search documents
    Search {
        #[arg(short, long)]
        query: String,
        #[arg(short, long)]
        collection: String,
        #[arg(short, long, default_value = "10")]
        limit: usize,
        #[arg(short, long, default_value = "0")]
        offset: usize,
        #[arg(short, long)]
        sort: Option<String>,
    },

    /// Create a snapshot
    SnapshotCreate {
        #[arg(short, long)]
        name: String,
    },

    /// Restore a snapshot
    SnapshotRestore {
        #[arg(short, long)]
        name: String,
    },
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    engine: Arc<SearchEngine>,
    cache: Cache<String, String>,
    persistence: Arc<PersistenceManager>,
    rate_limiter: Arc<tokio::sync::Mutex<HashMap<String, RateLimitEntry>>>,
    api_keys: Arc<tokio::sync::Mutex<HashMap<String, String>>>, // api_key -> user_id
}

#[derive(Clone)]
#[allow(dead_code)]
struct RateLimitEntry {
    count: u32,
    last_reset: std::time::Instant,
}

#[allow(dead_code)]
impl AppState {
    pub fn new(data_dir: &str) -> Self {
        let persistence = Arc::new(PersistenceManager::new(data_dir));
        let engine = Arc::new(SearchEngine::new());

        if let Err(e) = persistence.load_all(&engine) {
            eprintln!("Failed to load data: {}", e);
        }

        let cache = Cache::builder()
            .max_capacity(1000)
            .time_to_live(Duration::from_secs(300))
            .build();

        // Initialize API keys from environment or config
        let api_keys = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        if let Ok(keys) = std::env::var("NITRO_API_KEYS") {
            let mut keys_map = HashMap::new();
            for (i, key) in keys.split(',').enumerate() {
                keys_map.insert(key.trim().to_string(), format!("user_{}", i));
            }
            let keys_clone = api_keys.clone();
            tokio::spawn(async move {
                let mut guard = keys_clone.lock().await;
                *guard = keys_map;
            });
        }

        Self {
            engine,
            cache,
            persistence,
            rate_limiter: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            api_keys,
        }
    }

    async fn check_rate_limit(&self, client_ip: &str) -> Result<(), ApiError> {
        let mut limiter = self.rate_limiter.lock().await;
        let now = std::time::Instant::now();
        let entry = limiter
            .entry(client_ip.to_string())
            .or_insert(RateLimitEntry {
                count: 0,
                last_reset: now,
            });

        // Reset counter every minute
        if now.duration_since(entry.last_reset).as_secs() >= 60 {
            entry.count = 0;
            entry.last_reset = now;
        }

        // Limit to 100 requests per minute
        if entry.count >= 100 {
            return Err(ApiError::RateLimitExceeded);
        }

        entry.count += 1;
        Ok(())
    }

    async fn validate_api_key(&self, api_key: &str) -> Result<String, ApiError> {
        let keys = self.api_keys.lock().await;
        keys.get(api_key).cloned().ok_or(ApiError::Unauthorized)
    }
}

#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
}

impl ApiResponse<()> {
    fn error(msg: &str) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.to_string()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
enum ApiError {
    #[error("Collection not found: {0}")]
    CollectionNotFound(String),
    #[error("Document not found: {0}")]
    DocumentNotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Unauthorized: invalid API key")]
    Unauthorized,
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            ApiError::CollectionNotFound(name) => (
                StatusCode::NOT_FOUND,
                format!("Collection '{}' not found", name),
            ),
            ApiError::DocumentNotFound(id) => (
                StatusCode::NOT_FOUND,
                format!("Document '{}' not found", id),
            ),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Invalid API key".to_string()),
            ApiError::RateLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded".to_string(),
            ),
        };

        let body = Json(ApiResponse::<()>::error(&msg));
        (status, body).into_response()
    }
}

#[derive(Deserialize)]
struct CreateCollectionRequest {
    name: String,
    schema: Option<Schema>,
}

#[derive(Deserialize)]
struct InsertDocumentRequest {
    id: String,
    #[serde(flatten)]
    fields: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct BulkInsertRequest {
    documents: Vec<InsertDocumentRequest>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SearchQuery {
    q: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    cursor: Option<String>,
    sort: Option<String>,
    tenant: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SearchResponse {
    hits: Vec<nitro_core::SearchResult>,
    total: usize,
    time_ms: u64,
    facets: Option<HashMap<String, HashMap<String, usize>>>,
    next_cursor: Option<String>,
}

fn json_to_field_value(val: &serde_json::Value) -> FieldValue {
    match val {
        serde_json::Value::String(s) => FieldValue::Text(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                FieldValue::Number(i)
            } else {
                FieldValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::Bool(b) => FieldValue::Boolean(*b),
        serde_json::Value::Array(arr) => {
            FieldValue::Array(arr.iter().map(json_to_field_value).collect())
        }
        _ => FieldValue::Text(val.to_string()),
    }
}

async fn retry_with_backoff<F, Fut, T>(
    mut operation: F,
    max_retries: u32,
    initial_delay_ms: u64,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut delay = Duration::from_millis(initial_delay_ms);
    for attempt in 1..=max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt == max_retries {
                    return Err(e);
                }
                eprintln!(
                    "Attempt {} failed: {}. Retrying in {}ms...",
                    attempt,
                    e,
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, Duration::from_secs(10));
            }
        }
    }
    unreachable!()
}

async fn create_collection_api(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    let schema = req.schema.unwrap_or_else(Schema::default);
    let collection = Collection::new(&req.name, schema);
    state
        .engine
        .create_collection(&req.name, collection.clone());
    state
        .persistence
        .save_collection(&req.name, &collection)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ApiResponse::success(())))
}

async fn insert_document_api(
    State(state): State<Arc<AppState>>,
    Path(collection_name): Path<String>,
    Json(req): Json<InsertDocumentRequest>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    if state.engine.get_collection(&collection_name).is_none() {
        return Err(ApiError::CollectionNotFound(collection_name));
    }
    let mut doc = Document::new(&req.id);
    for (k, v) in &req.fields {
        doc.set(k, json_to_field_value(v));
    }
    state.engine.insert_document(&collection_name, doc.clone());
    state
        .persistence
        .save_document(&collection_name, &doc)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    state.cache.invalidate_all();
    Ok(Json(ApiResponse::success(())))
}

async fn bulk_insert_api(
    State(state): State<Arc<AppState>>,
    Path(collection_name): Path<String>,
    Json(req): Json<BulkInsertRequest>,
) -> Result<Json<ApiResponse<HashMap<String, usize>>>, ApiError> {
    if state.engine.get_collection(&collection_name).is_none() {
        return Err(ApiError::CollectionNotFound(collection_name));
    }
    let mut success_count = 0;
    let mut error_count = 0;
    for doc_req in req.documents {
        let mut doc = Document::new(&doc_req.id);
        for (k, v) in &doc_req.fields {
            doc.set(k, json_to_field_value(v));
        }
        state.engine.insert_document(&collection_name, doc.clone());
        if state
            .persistence
            .save_document(&collection_name, &doc)
            .is_err()
        {
            error_count += 1;
        } else {
            success_count += 1;
        }
    }
    state.cache.invalidate_all();
    let mut result = HashMap::new();
    result.insert("success".to_string(), success_count);
    result.insert("errors".to_string(), error_count);
    Ok(Json(ApiResponse::success(result)))
}

async fn delete_document_api(
    State(state): State<Arc<AppState>>,
    Path((collection_name, doc_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    if state.engine.get_collection(&collection_name).is_none() {
        return Err(ApiError::CollectionNotFound(collection_name));
    }
    state.engine.delete_document(&collection_name, &doc_id);
    state
        .persistence
        .delete_document(&collection_name, &doc_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    state.cache.invalidate_all();
    Ok(Json(ApiResponse::success(())))
}

#[derive(Deserialize)]
struct BulkDeleteRequest {
    ids: Vec<String>,
}

async fn bulk_delete_api(
    State(state): State<Arc<AppState>>,
    Path(collection_name): Path<String>,
    Json(req): Json<BulkDeleteRequest>,
) -> Result<Json<ApiResponse<HashMap<String, usize>>>, ApiError> {
    if state.engine.get_collection(&collection_name).is_none() {
        return Err(ApiError::CollectionNotFound(collection_name));
    }

    let mut success_count = 0;
    let mut error_count = 0;

    for doc_id in req.ids {
        state.engine.delete_document(&collection_name, &doc_id);
        if state
            .persistence
            .delete_document(&collection_name, &doc_id)
            .is_err()
        {
            error_count += 1;
        } else {
            success_count += 1;
        }
    }

    state.cache.invalidate_all();

    let mut result = HashMap::new();
    result.insert("deleted".to_string(), success_count);
    result.insert("errors".to_string(), error_count);

    Ok(Json(ApiResponse::success(result)))
}

async fn delete_collection_api(
    State(state): State<Arc<AppState>>,
    Path(collection_name): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    if state.engine.get_collection(&collection_name).is_none() {
        return Err(ApiError::CollectionNotFound(collection_name));
    }

    state.engine.delete_collection(&collection_name);
    state
        .persistence
        .delete_collection(&collection_name)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    state.cache.invalidate_all();

    Ok(Json(ApiResponse::success(())))
}

async fn search_api(
    State(state): State<Arc<AppState>>,
    Path(collection_name): Path<String>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<ApiResponse<SearchResponse>>, ApiError> {
    if state.engine.get_collection(&collection_name).is_none() {
        return Err(ApiError::CollectionNotFound(collection_name));
    }

    let cache_key = format!(
        "{}:{}:{}:{}:{}",
        collection_name,
        params.q.clone().unwrap_or_default(),
        params.limit.unwrap_or(10),
        params.offset.unwrap_or(0),
        params.sort.clone().unwrap_or_default()
    );

    if let Some(cached) = state.cache.get(&cache_key).await {
        let response: SearchResponse =
            serde_json::from_str(&cached).map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(Json(ApiResponse::success(response)));
    }

    let start = Instant::now();
    let query = params.q.unwrap_or_default();
    let limit = params.limit.unwrap_or(10);
    let offset = params.offset.unwrap_or(0);

    let results = state.engine.search(&collection_name, &query, limit);
    let mut hits = results.hits;

    if let Some(sort_field) = params.sort {
        let (field, order) = if let Some(stripped) = sort_field.strip_prefix('-') {
            (stripped.to_string(), "desc")
        } else {
            (sort_field, "asc")
        };
        hits.sort_by(|a, b| {
            let a_val = a.doc.get(&field);
            let b_val = b.doc.get(&field);
            match (a_val, b_val) {
                (Some(FieldValue::Number(a)), Some(FieldValue::Number(b))) => {
                    if order == "desc" {
                        b.cmp(a)
                    } else {
                        a.cmp(b)
                    }
                }
                (Some(FieldValue::Float(a)), Some(FieldValue::Float(b))) => {
                    if order == "desc" {
                        b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal)
                    } else {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    }
                }
                (Some(FieldValue::Text(a)), Some(FieldValue::Text(b))) => {
                    if order == "desc" {
                        b.cmp(a)
                    } else {
                        a.cmp(b)
                    }
                }
                _ => std::cmp::Ordering::Equal,
            }
        });
    }

    let total = hits.len();

    // Cursor pagination: if cursor provided, skip until we find cursor doc
    let (start_idx, end_idx) = if let Some(ref cursor) = params.cursor {
        let mut found = false;
        let mut s = 0;
        for (i, hit) in hits.iter().enumerate() {
            if found {
                s = i;
                break;
            }
            if hit.doc.id == *cursor {
                found = true;
                s = i + 1;
            }
        }
        (s, (s + limit).min(hits.len()))
    } else {
        (offset, (offset + limit).min(hits.len()))
    };

    let hits: Vec<_> = hits
        .into_iter()
        .skip(start_idx)
        .take(end_idx - start_idx)
        .collect();
    let next_cursor = if end_idx < total {
        hits.last().map(|h| h.doc.id.clone())
    } else {
        None
    };

    // Compute facets across ALL text/keyword fields
    let mut facets: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for hit in &hits {
        for (field_name, value) in &hit.doc.fields {
            match value {
                FieldValue::Keyword(keyword) => {
                    let field_facets = facets.entry(field_name.clone()).or_default();
                    *field_facets.entry(keyword.clone()).or_insert(0) += 1;
                }
                // Numeric range faceting: bucket numbers into ranges
                FieldValue::Number(n) => {
                    let range = format!("{}-{}", (n / 100) * 100, ((n / 100) + 1) * 100 - 1);
                    let field_facets = facets.entry(format!("{}_range", field_name)).or_default();
                    *field_facets.entry(range).or_insert(0) += 1;
                }
                _ => {}
            }
        }
    }

    let time_ms = start.elapsed().as_millis() as u64;
    crate::metrics::METRICS.inc_search_requests();
    crate::metrics::METRICS.record_search_latency(time_ms);

    let response = SearchResponse {
        hits,
        total,
        time_ms,
        facets: if facets.is_empty() {
            None
        } else {
            Some(facets)
        },
        next_cursor,
    };

    if let Ok(json) = serde_json::to_string(&response) {
        state.cache.insert(cache_key, json).await;
    }

    Ok(Json(ApiResponse::success(response)))
}

async fn list_collections_api(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<String>>> {
    Json(ApiResponse::success(state.engine.list_collections()))
}

async fn get_document_api(
    State(state): State<Arc<AppState>>,
    Path((collection_name, doc_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<Document>>, ApiError> {
    if state.engine.get_collection(&collection_name).is_none() {
        return Err(ApiError::CollectionNotFound(collection_name));
    }
    let docs = state.engine.get_documents(&collection_name);
    match docs.get(&doc_id) {
        Some(doc) => Ok(Json(ApiResponse::success(doc.clone()))),
        None => Err(ApiError::DocumentNotFound(doc_id)),
    }
}

async fn health_check() -> &'static str {
    "OK"
}

async fn metrics_handler() -> String {
    crate::metrics::METRICS.to_prometheus_format()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { data_dir, bind } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()),
                )
                .init();

            let state = Arc::new(AppState::new(&data_dir));
            let app = Router::new()
                .route("/health", get(health_check))
                .route("/metrics", get(metrics_handler))
                .route(
                    "/collections",
                    get(list_collections_api).post(create_collection_api),
                )
                .route(
                    "/{collection}/documents",
                    post(insert_document_api).put(bulk_insert_api),
                )
                .route(
                    "/{collection}/documents/_bulk",
                    post(bulk_insert_api)
                        .put(bulk_insert_api)
                        .delete(bulk_delete_api),
                )
                .route(
                    "/{collection}/documents/{id}",
                    get(get_document_api).delete(delete_document_api),
                )
                .route("/{collection}", delete(delete_collection_api))
                .route("/{collection}/search", get(search_api))
                .with_state(state);

            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!("Server listening on http://{}", bind);
            axum::serve(listener, app).await?;
        }

        Commands::Create { name } => {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?;
            let result = retry_with_backoff(
                || async {
                    let res = client
                        .post(format!("{}/collections", cli.url))
                        .json(&serde_json::json!({"name": name, "schema": {"fields": []}}))
                        .send()
                        .await?;
                    if res.status().is_success() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("Failed: {}", res.status()))
                    }
                },
                3,
                500,
            )
            .await;
            match result {
                Ok(_) => println!("✓ Collection '{}' created", name),
                Err(e) => println!("✗ {}", e),
            }
        }

        Commands::Index { file, collection } => {
            let content = std::fs::read_to_string(&file)?;
            let documents: Vec<serde_json::Value> = serde_json::from_str(&content)?;
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?;
            let mut success = 0;
            let mut errors = 0;

            for (i, doc) in documents.into_iter().enumerate() {
                let result = retry_with_backoff(
                    || async {
                        let res = client
                            .post(format!("{}/{}/documents", cli.url, collection))
                            .json(&doc)
                            .send()
                            .await?;
                        if res.status().is_success() {
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("Failed doc {}: {}", i + 1, res.status()))
                        }
                    },
                    3,
                    500,
                )
                .await;
                match result {
                    Ok(_) => success += 1,
                    Err(e) => {
                        errors += 1;
                        eprintln!("✗ {}", e);
                    }
                }
            }
            println!("✓ Indexed {} documents ({} errors)", success, errors);
        }

        Commands::Search {
            query,
            collection,
            limit,
            offset,
            sort,
        } => {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?;
            let url = format!("{}/{}/search", cli.url, collection);
            let mut params = vec![("limit", limit.to_string()), ("offset", offset.to_string())];
            if !query.is_empty() {
                params.push(("q", query));
            }
            if let Some(s) = sort {
                params.push(("sort", s));
            }

            let result = retry_with_backoff(
                || async {
                    let res = client.get(&url).query(&params).send().await?;
                    if res.status().is_success() {
                        Ok(res.text().await?)
                    } else {
                        Err(anyhow::anyhow!("Search failed: {}", res.status()))
                    }
                },
                3,
                500,
            )
            .await;
            match result {
                Ok(body) => println!("{}", body),
                Err(e) => println!("✗ {}", e),
            }
        }

        Commands::SnapshotCreate { name } => {
            let persistence = PersistenceManager::new("./data");
            match persistence.create_snapshot(&name) {
                Ok(_) => println!("✓ Snapshot '{}' created successfully", name),
                Err(e) => println!("✗ Failed to create snapshot: {}", e),
            }
        }

        Commands::SnapshotRestore { name } => {
            let persistence = PersistenceManager::new("./data");
            match persistence.restore_snapshot(&name) {
                Ok(_) => println!("✓ Snapshot '{}' restored successfully", name),
                Err(e) => println!("✗ Failed to restore snapshot: {}", e),
            }
        }
    }
    Ok(())
}
