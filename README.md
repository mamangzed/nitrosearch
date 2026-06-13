# NitroSearch

**Lightweight Rust-based Search Engine, Ready for Large-Scale Data.**

NitroSearch is a full-text search engine written in Rust. Designed as a lighter and more efficient alternative to Elasticsearch, without requiring a JVM, featuring sub-second startup times and extremely low memory usage.

---

## Features

- **High Performance**: Startup < 1 second, query latency < 200ms (p99) for millions of documents.
- **Resource Efficient**: Binary size ~3MB, idle RAM < 50MB. Uses memory-mapped files (mmap) to avoid RAM exhaustion.
- **Advanced Search**: Supports BM25 ranking, field boosting (`title:rust^2.0`), fuzzy search (`rust~1`), phrase search, and boolean operators (AND, OR, NOT).
- **Fast Indexing**: Supports batch inserts with async flush and compression (VarInt + Delta Encoding).
- **Production Ready**: Write-Ahead Log (WAL) for crash recovery, background segment merging, and sharding support.
- **Observability**: `/metrics` endpoint compatible with Prometheus for real-time monitoring.

---

## Installation

### Option 1: Build from Source (Recommended)

Ensure you have [Rust](https://rustup.rs/) (version 1.70+) installed.

```bash
# 1. Clone the repository
git clone https://github.com/mamangzed/nitrosearch.git
cd nitrosearch

# 2. Build the optimized release version
cargo build --release

# 3. Move the binary to PATH (optional)
sudo cp target/release/nitro /usr/local/bin/
```

### Option 2: Download Pre-built Binary

Download the latest binary from the [Releases](https://github.com/mamangzed/nitrosearch/releases) page according to your operating system (Linux x86_64, macOS, Windows).

```bash
# Example for Linux x86_64
wget https://github.com/mamangzed/nitrosearch/releases/download/v1.0.0/nitro-linux-x86_64.tar.gz
tar -xzf nitro-linux-x86_64.tar.gz
sudo mv nitro /usr/local/bin/
```

---

## Quick Start

### 1. Start the Server

Start the NitroSearch server. By default, the server runs on port `8080` and stores data in the `./data` directory.

```bash
nitro start --data-dir ./data --bind 0.0.0.0:8080
```

> **Tip**: For production, use environment variables: `NITRO_DATA_DIR=/var/lib/nitrosearch nitro start`

### 2. Create a Collection

A collection is similar to a "table" in a database or an "index" in Elasticsearch. We need to define its schema.

```bash
curl -X POST http://localhost:8080/collections \
  -H "Content-Type: application/json" \
  -d '{
    "name": "products",
    "schema": {
      "fields": [
        {"name": "title", "type": "Text"},
        {"name": "description", "type": "Text"},
        {"name": "category", "type": "Keyword"},
        {"name": "price", "type": "Number"}
      ]
    }
  }'
```

### 3. Index Data

You can insert data one by one or in bulk.

**Bulk Insert (Recommended for performance):**

```bash
curl -X POST http://localhost:8080/products/documents/_bulk \
  -H "Content-Type: application/json" \
  -d '{
    "documents": [
      {"id": "1", "title": "MacBook Pro M3", "description": "Professional laptop with M3 chip", "category": "electronics", "price": 25000000},
      {"id": "2", "title": "Rust Programming Book", "description": "Complete guide to learning the Rust programming language", "category": "books", "price": 350000},
      {"id": "3", "title": "Mechanical Keyboard", "description": "Mechanical keyboard with blue switches", "category": "electronics", "price": 1200000}
    ]
  }'
```

### 4. Search

**Basic Search:**
```bash
curl "http://localhost:8080/products/search?q=rust"
```

**Search with Field Boosting** (Makes the `title` field 3x more important):
```bash
curl "http://localhost:8080/products/search?q=title^3 rust"
```

**Search with Sorting and Pagination:**
```bash
curl "http://localhost:8080/products/search?q=*&sort=price:asc&limit=10&offset=0"
```

**Fuzzy Search** (Search with typo tolerance, e.g., "macbok"):
```bash
curl "http://localhost:8080/products/search?q=macbok~1"
```

### 5. Delete Data

**Delete a single document:**
```bash
curl -X DELETE http://localhost:8080/products/documents/1
```

**Delete an entire collection:**
```bash
curl -X DELETE http://localhost:8080/products
```

---

## Advanced Configuration (For Large-Scale Data)

NitroSearch can be configured via environment variables to handle large-scale data (10 Million+ documents).

| Environment Variable | Default | Description |
|----------------------|---------|-------------|
| `NITRO_DATA_DIR` | `./data` | Directory to store segments and WAL. Use SSD/NVMe for best performance. |
| `NITRO_PORT` | `8080` | HTTP server port. |
| `NITRO_FLUSH_THRESHOLD` | `10000` | Number of documents in the buffer before flushing to disk. Increase for faster indexing, decrease for real-time visibility. |
| `NITRO_MEMORY_LIMIT` | `1073741824` (1GB) | Maximum RAM usage limit for the buffer before triggering an automatic flush. |
| `NITRO_API_KEYS` | `""` | List of allowed API keys (comma-separated) for simple authentication. |

**Example running with tuning for a 16GB RAM server:**
```bash
export NITRO_DATA_DIR=/mnt/nvme/nitro_data
export NITRO_FLUSH_THRESHOLD=50000
export NITRO_MEMORY_LIMIT=4294967296 # 4GB
nitro start
```

---

## Monitoring & Health Check

NitroSearch provides built-in endpoints for monitoring:

- **Health Check**: `curl http://localhost:8080/health` (Returns `OK` if the server is running).
- **Prometheus Metrics**: `curl http://localhost:8080/metrics`

Example metrics output:
```text
# HELP nitro_documents_total Total number of indexed documents
nitro_documents_total{collection="products"} 1000000
# HELP nitro_search_latency_ms Search latency in milliseconds
nitro_search_latency_ms{quantile="0.99"} 45
# HELP nitro_segments_total Number of active segments on disk
nitro_segments_total{collection="products"} 12
```

---

## Production Best Practices

1. **Use SSD/NVMe**: Since NitroSearch relies on memory-mapped files, disk speed heavily impacts search performance.
2. **Use Bulk Inserts**: Always use the `/_bulk` endpoint to index large amounts of data; do not loop single inserts.
3. **Monitor Memory**: Ensure `NITRO_MEMORY_LIMIT` is set according to your server's RAM capacity to prevent OOM (Out of Memory) errors.
4. **Configure Reverse Proxy**: If using a reverse proxy (like Nginx), ensure it forwards the `X-Forwarded-For` header so per-IP rate limiting works correctly.
5. **Regular Backups**: Perform regular data snapshots. You can copy the `NITRO_DATA_DIR` folder while the server is stopped, or use the snapshot feature (if enabled).

---

## Contributing

Contributions are welcome! Please open an Issue or create a Pull Request for new features, bug fixes, or documentation improvements.

---

## License

Distributed under the **MIT License**. See the `LICENSE` file for more information.

---

> **Built with Rust.**