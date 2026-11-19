/// Entry in the log of executed queries.
pub struct LogEntry {
    pub time: String,
    pub query: String,
    pub duration_ms: u64,
    pub rows: Option<usize>, // None if there was an error
    pub error: Option<String>,
}

impl LogEntry {
    pub fn ok(query: impl Into<String>, duration_ms: u64, rows: usize) -> Self {
        Self {
            time: now_hms(),
            query: query.into(),
            duration_ms,
            rows: Some(rows),
            error: None,
        }
    }

    pub fn err(query: impl Into<String>, duration_ms: u64, error: impl Into<String>) -> Self {
        Self {
            time: now_hms(),
            query: query.into(),
            duration_ms,
            rows: None,
            error: Some(error.into()),
        }
    }
}

/// Local time: arithmetic on UNIX_EPOCH gave UTC time.
fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}
