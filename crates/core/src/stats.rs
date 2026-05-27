use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UsageRecord {
    pub ts: f64,
    pub call: String,
    pub results: usize,
    pub snippet_chars: usize,
    pub file_chars: usize,
}

impl UsageRecord {
    pub fn now(call: &str, results: usize, snippet_chars: usize, file_chars: usize) -> Self {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        Self {
            ts,
            call: call.to_string(),
            results,
            snippet_chars,
            file_chars,
        }
    }
}

/// OS-specific cache directory for savings data.
pub fn savings_path() -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("sonar").join("savings.jsonl")
}

/// Append a usage record to the savings file.
pub fn record_usage(record: &UsageRecord) -> Result<(), String> {
    let path = savings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create cache dir: {e}"))?;
    }
    let line = serde_json::to_string(record).map_err(|e| format!("Serialize error: {e}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Failed to open savings file: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("Write error: {e}"))?;
    Ok(())
}

/// Read all usage records from the savings file.
pub fn read_usage() -> Result<Vec<UsageRecord>, String> {
    let path = savings_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path).map_err(|e| format!("Failed to open savings file: {e}"))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("Read error: {e}"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<UsageRecord>(trimmed) {
            records.push(record);
        }
    }
    Ok(records)
}

#[derive(Debug, Clone, Default)]
pub struct SavingsSummary {
    pub calls: usize,
    pub snippet_chars: usize,
    pub file_chars: usize,
    pub saved_chars: usize,
    pub saved_tokens: usize,
}

/// Calculate savings for a time period.
/// If `since` is provided, only records with ts >= since are included.
pub fn calculate_savings(records: &[UsageRecord], since: Option<f64>) -> SavingsSummary {
    let filtered: Vec<&UsageRecord> = records
        .iter()
        .filter(|r| since.is_none_or(|s| r.ts >= s))
        .collect();

    let calls = filtered.len();
    let snippet_chars: usize = filtered.iter().map(|r| r.snippet_chars).sum();
    let file_chars: usize = filtered.iter().map(|r| r.file_chars).sum();
    let saved_chars = file_chars.saturating_sub(snippet_chars);
    let saved_tokens = saved_chars / 4;

    SavingsSummary {
        calls,
        snippet_chars,
        file_chars,
        saved_chars,
        saved_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_savings_path_not_empty() {
        let path = savings_path();
        assert!(path.to_str().unwrap().contains("sonar"));
        assert!(path.to_str().unwrap().contains("savings.jsonl"));
    }

    #[test]
    fn test_calculate_savings_empty() {
        let summary = calculate_savings(&[], None);
        assert_eq!(summary.calls, 0);
        assert_eq!(summary.saved_tokens, 0);
    }

    #[test]
    fn test_calculate_savings_basic() {
        let records = vec![
            UsageRecord {
                ts: 1000.0,
                call: "search".to_string(),
                results: 5,
                snippet_chars: 200,
                file_chars: 1000,
            },
            UsageRecord {
                ts: 2000.0,
                call: "find_related".to_string(),
                results: 3,
                snippet_chars: 100,
                file_chars: 500,
            },
        ];

        let summary = calculate_savings(&records, None);
        assert_eq!(summary.calls, 2);
        assert_eq!(summary.snippet_chars, 300);
        assert_eq!(summary.file_chars, 1500);
        assert_eq!(summary.saved_chars, 1200);
        assert_eq!(summary.saved_tokens, 300);
    }

    #[test]
    fn test_calculate_savings_with_since() {
        let records = vec![
            UsageRecord {
                ts: 1000.0,
                call: "search".to_string(),
                results: 5,
                snippet_chars: 200,
                file_chars: 1000,
            },
            UsageRecord {
                ts: 2000.0,
                call: "search".to_string(),
                results: 3,
                snippet_chars: 100,
                file_chars: 500,
            },
        ];

        let summary = calculate_savings(&records, Some(1500.0));
        assert_eq!(summary.calls, 1);
        assert_eq!(summary.snippet_chars, 100);
        assert_eq!(summary.file_chars, 500);
    }

    #[test]
    fn test_record_and_read_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("savings.jsonl");

        let record = UsageRecord::now("search", 5, 200, 1000);
        let line = serde_json::to_string(&record).unwrap();
        fs::write(&path, format!("{line}\n")).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let parsed: UsageRecord = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.call, "search");
        assert_eq!(parsed.results, 5);
    }
}
