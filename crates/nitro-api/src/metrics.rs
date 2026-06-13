use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub search_requests_total: AtomicU64,
    pub index_requests_total: AtomicU64,
    pub errors_total: AtomicU64,
    pub avg_search_latency_ms: AtomicU64,
}

impl Metrics {
    pub fn inc_requests(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_search_requests(&self) {
        self.search_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_index_requests(&self) {
        self.index_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_errors(&self) {
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_search_latency(&self, ms: u64) {
        // Simple moving average approximation
        let current = self.avg_search_latency_ms.load(Ordering::Relaxed);
        let new_avg = (current + ms) / 2;
        self.avg_search_latency_ms.store(new_avg, Ordering::Relaxed);
    }

    pub fn to_prometheus_format(&self) -> String {
        format!(
            "# HELP nitro_requests_total Total number of requests\n\
             # TYPE nitro_requests_total counter\n\
             nitro_requests_total {}\n\
             # HELP nitro_search_requests_total Total number of search requests\n\
             # TYPE nitro_search_requests_total counter\n\
             nitro_search_requests_total {}\n\
             # HELP nitro_index_requests_total Total number of index requests\n\
             # TYPE nitro_index_requests_total counter\n\
             nitro_index_requests_total {}\n\
             # HELP nitro_errors_total Total number of errors\n\
             # TYPE nitro_errors_total counter\n\
             nitro_errors_total {}\n\
             # HELP nitro_avg_search_latency_ms Average search latency in milliseconds\n\
             # TYPE nitro_avg_search_latency_ms gauge\n\
             nitro_avg_search_latency_ms {}\n",
            self.requests_total.load(Ordering::Relaxed),
            self.search_requests_total.load(Ordering::Relaxed),
            self.index_requests_total.load(Ordering::Relaxed),
            self.errors_total.load(Ordering::Relaxed),
            self.avg_search_latency_ms.load(Ordering::Relaxed)
        )
    }
}

pub static METRICS: Metrics = Metrics {
    requests_total: AtomicU64::new(0),
    search_requests_total: AtomicU64::new(0),
    index_requests_total: AtomicU64::new(0),
    errors_total: AtomicU64::new(0),
    avg_search_latency_ms: AtomicU64::new(0),
};
