use regex::Regex;
use std::sync::LazyLock;

use crate::types::{Chunk, SearchResult};

static GIT_URL_SCHEMES: &[&str] = &[
    "https://",
    "http://",
    "ssh://",
    "git://",
    "git+ssh://",
    "file://",
];

/// Matches SCP-style git URLs like `git@github.com:org/repo`.
/// Rust regex doesn't support look-ahead, so we match the colon
/// and check the next char isn't '/' manually.
fn is_scp_git_url(path: &str) -> bool {
    static SCP_PREFIX_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[\w.\-]+@[\w.\-]+:").unwrap());
    if let Some(m) = SCP_PREFIX_RE.find(path) {
        let rest = &path[m.end()..];
        !rest.starts_with('/')
    } else {
        false
    }
}

/// Return true if path looks like a remote git URL rather than a local path.
///
/// Ported from semble's `utils.py::is_git_url`.
pub fn is_git_url(path: &str) -> bool {
    GIT_URL_SCHEMES.iter().any(|s| path.starts_with(s)) || is_scp_git_url(path)
}

/// Return the chunk containing `line` in `file_path`, or None.
///
/// Ported from semble's `utils.py::resolve_chunk`.
pub fn resolve_chunk<'a>(chunks: &'a [Chunk], file_path: &str, line: usize) -> Option<&'a Chunk> {
    let mut fallback = None;
    for chunk in chunks {
        if chunk.file_path == file_path && chunk.start_line <= line && line <= chunk.end_line {
            if line < chunk.end_line {
                return Some(chunk);
            }
            if fallback.is_none() {
                fallback = Some(chunk);
            }
        }
    }
    fallback
}

/// Render SearchResult objects as a JSON-serializable structure.
///
/// Ported from semble's `utils.py::format_results`.
pub fn format_results(query: &str, results: &[SearchResult]) -> serde_json::Value {
    serde_json::json!({
        "query": query,
        "results": results.iter().map(|r| serde_json::json!({
            "chunk": {
                "content": r.chunk.content,
                "file_path": r.chunk.file_path,
                "start_line": r.chunk.start_line,
                "end_line": r.chunk.end_line,
                "language": r.chunk.language,
                "location": r.chunk.location(),
            },
            "score": r.score,
        })).collect::<Vec<_>>(),
    })
}

/// Append file path components to BM25 content to boost path-based queries.
///
/// Ported from semble's `index/sparse.py::enrich_for_bm25`.
pub fn enrich_for_bm25(chunk: &Chunk) -> String {
    let path = std::path::Path::new(&chunk.file_path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    let dir_parts: Vec<&str> = path
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| {
                    let s = c.as_os_str().to_str()?;
                    if s == "." || s == "/" { None } else { Some(s) }
                })
                .collect()
        })
        .unwrap_or_default();

    let dir_text: String = dir_parts
        .iter()
        .rev()
        .take(3)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");

    format!("{} {} {} {}", chunk.content, stem, stem, dir_text)
}

/// Convert a selector array of indices into a boolean mask.
///
/// Ported from semble's `index/sparse.py::selector_to_mask`.
pub fn selector_to_mask(selector: Option<&[usize]>, size: usize) -> Option<Vec<bool>> {
    let sel = selector?;
    let mut mask = vec![false; size];
    for &idx in sel {
        if idx < size {
            mask[idx] = true;
        }
    }
    Some(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_git_url() {
        assert!(is_git_url("https://github.com/org/repo"));
        assert!(is_git_url("git@github.com:org/repo"));
        assert!(!is_git_url("/Users/teddy/dev/project"));
        assert!(!is_git_url("./relative/path"));
    }

    #[test]
    fn test_resolve_chunk() {
        let chunks = vec![
            Chunk {
                content: "fn main()".to_string(),
                file_path: "src/main.rs".to_string(),
                start_line: 1,
                end_line: 10,
                language: Some("rust".to_string()),
            },
            Chunk {
                content: "fn helper()".to_string(),
                file_path: "src/main.rs".to_string(),
                start_line: 12,
                end_line: 20,
                language: Some("rust".to_string()),
            },
        ];

        let result = resolve_chunk(&chunks, "src/main.rs", 5);
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "fn main()");

        let result = resolve_chunk(&chunks, "src/main.rs", 15);
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "fn helper()");

        let result = resolve_chunk(&chunks, "src/other.rs", 5);
        assert!(result.is_none());
    }

    #[test]
    fn test_enrich_for_bm25() {
        let chunk = Chunk {
            content: "fn authenticate()".to_string(),
            file_path: "src/auth/handler.rs".to_string(),
            start_line: 1,
            end_line: 10,
            language: Some("rust".to_string()),
        };
        let enriched = enrich_for_bm25(&chunk);
        assert!(enriched.contains("fn authenticate()"));
        assert!(enriched.contains("handler"));
        assert!(enriched.contains("auth"));
    }

    #[test]
    fn test_selector_to_mask() {
        let mask = selector_to_mask(Some(&[0, 2, 4]), 5).unwrap();
        assert_eq!(mask, vec![true, false, true, false, true]);

        assert!(selector_to_mask(None, 5).is_none());
    }
}
