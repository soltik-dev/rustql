/// Entrada en el log de consultas ejecutadas.
pub struct LogEntry {
    pub time: String,
    pub query: String,
    pub duration_ms: u64,
    pub rows: Option<usize>,  // None si hubo error
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

fn now_hms() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:02}:{:02}:{:02}", (secs % 86400) / 3600, (secs % 3600) / 60, secs % 60)
}
