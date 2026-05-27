use std::collections::HashMap;

use regex::Regex;
use std::sync::LazyLock;

use crate::types::{Chunk, SearchResult};

/// Ported from semble's `ranking/penalties.py`.
static TEST_FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?:^|/)",
        r"(?:",
        r"test_[^/]*\.py|[^/]*_test\.py",
        r"|[^/]*_test\.go",
        r"|[^/]*Tests?\.java",
        r"|[^/]*Test\.php",
        r"|[^/]*_spec\.rb|[^/]*_test\.rb",
        r"|[^/]*\.test\.[jt]sx?|[^/]*\.spec\.[jt]sx?",
        r"|[^/]*Tests?\.kt|[^/]*Spec\.kt",
        r"|[^/]*Tests?\.swift|[^/]*Spec\.swift",
        r"|[^/]*Tests?\.cs",
        r"|test_[^/]*\.cpp|[^/]*_test\.cpp",
        r"|test_[^/]*\.c|[^/]*_test\.c",
        r"|[^/]*Spec\.scala|[^/]*Suite\.scala|[^/]*Test\.scala",
        r"|[^/]*_test\.dart|test_[^/]*\.dart",
        r"|[^/]*_spec\.lua|[^/]*_test\.lua|test_[^/]*\.lua",
        r"|test_helpers?[^/]*\.\w+",
        r")$",
    ))
    .unwrap()
});

static TEST_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)(?:tests?|__tests__|spec|testing)(?:/|$)").unwrap());

static COMPAT_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)(?:compat|_compat|legacy)(?:/|$)").unwrap());

static EXAMPLES_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)(?:_?examples?|docs?_src)(?:/|$)").unwrap());

static TYPE_DEFS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.d\.ts$").unwrap());

const STRONG_PENALTY: f64 = 0.3;
const MODERATE_PENALTY: f64 = 0.5;
const MILD_PENALTY: f64 = 0.7;

const REEXPORT_FILENAMES: &[&str] = &["__init__.py", "package-info.java"];

const FILE_SATURATION_THRESHOLD: usize = 1;
const FILE_SATURATION_DECAY: f64 = 0.5;

/// Compute file-path penalty for a given path.
///
/// Ported verbatim from semble's `ranking/penalties.py::_file_path_penalty`.
pub fn file_path_penalty(file_path: &str) -> f64 {
    let normalised = file_path.replace('\\', "/");
    let mut penalty = 1.0;

    if TEST_FILE_RE.is_match(&normalised) || TEST_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }

    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if REEXPORT_FILENAMES.contains(&filename) {
        penalty *= MODERATE_PENALTY;
    }

    if COMPAT_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    if EXAMPLES_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    if TYPE_DEFS_RE.is_match(&normalised) {
        penalty *= MILD_PENALTY;
    }

    penalty
}

/// Select top-k results with file-path penalties and file-saturation decay.
///
/// Ported verbatim from semble's `ranking/penalties.py::rerank_topk`.
pub fn rerank_topk(
    scores: &HashMap<Chunk, f64>,
    top_k: usize,
    penalise_paths: bool,
) -> Vec<SearchResult> {
    if scores.is_empty() {
        return Vec::new();
    }

    let mut penalty_cache: HashMap<&str, f64> = HashMap::new();
    let mut penalised: HashMap<&Chunk, f64> = HashMap::new();

    for (chunk, &score) in scores {
        if penalise_paths {
            let penalty = *penalty_cache
                .entry(chunk.file_path.as_str())
                .or_insert_with(|| file_path_penalty(&chunk.file_path));
            penalised.insert(chunk, score * penalty);
        } else {
            penalised.insert(chunk, score);
        }
    }

    let mut ranked: Vec<&Chunk> = penalised.keys().copied().collect();
    ranked.sort_by(|a, b| {
        penalised[b]
            .partial_cmp(&penalised[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut file_selected: HashMap<&str, usize> = HashMap::new();
    let mut selected: Vec<(f64, &Chunk)> = Vec::new();
    let mut min_selected = f64::INFINITY;

    for chunk in ranked {
        let pen_score = penalised[chunk];

        if selected.len() >= top_k && pen_score <= min_selected {
            break;
        }

        let already_selected = *file_selected.get(chunk.file_path.as_str()).unwrap_or(&0);
        let eff_score = if already_selected >= FILE_SATURATION_THRESHOLD {
            let excess = (already_selected - FILE_SATURATION_THRESHOLD + 1) as f64;
            pen_score * FILE_SATURATION_DECAY.powf(excess)
        } else {
            pen_score
        };

        selected.push((eff_score, chunk));
        *file_selected.entry(chunk.file_path.as_str()).or_insert(0) += 1;

        if selected.len() >= top_k {
            min_selected = selected
                .iter()
                .map(|(s, _)| *s)
                .fold(f64::INFINITY, f64::min);
        }
    }

    selected.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    selected
        .into_iter()
        .take(top_k)
        .map(|(score, chunk)| SearchResult {
            chunk: chunk.clone(),
            score,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(file_path: &str, start_line: usize) -> Chunk {
        Chunk {
            content: String::new(),
            file_path: file_path.to_string(),
            start_line,
            end_line: start_line + 10,
            language: None,
        }
    }

    #[test]
    fn test_no_penalty_normal_file() {
        assert_eq!(file_path_penalty("src/main.rs"), 1.0);
    }

    #[test]
    fn test_test_file_penalty() {
        assert!((file_path_penalty("tests/test_auth.py") - STRONG_PENALTY).abs() < 0.01);
    }

    #[test]
    fn test_init_penalty() {
        assert!((file_path_penalty("src/utils/__init__.py") - MODERATE_PENALTY).abs() < 0.01);
    }

    #[test]
    fn test_rerank_topk_basic() {
        let c1 = make_chunk("src/main.rs", 1);
        let c2 = make_chunk("src/lib.rs", 1);
        let c3 = make_chunk("tests/test_foo.py", 1);

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(c1.clone(), 1.0);
        scores.insert(c2.clone(), 0.9);
        scores.insert(c3.clone(), 0.95);

        let results = rerank_topk(&scores, 3, true);
        assert_eq!(results.len(), 3);
        // c1 (1.0 * 1.0) should be first, c2 (0.9 * 1.0) second
        // c3 gets penalised: 0.95 * 0.3 = 0.285, so last
        assert_eq!(results[0].chunk.file_path, "src/main.rs");
        assert_eq!(results[1].chunk.file_path, "src/lib.rs");
        assert_eq!(results[2].chunk.file_path, "tests/test_foo.py");
    }

    #[test]
    fn test_rerank_topk_saturation() {
        let c1 = make_chunk("src/main.rs", 1);
        let c2 = make_chunk("src/main.rs", 20);
        let c3 = make_chunk("src/main.rs", 40);
        let c4 = make_chunk("src/other.rs", 1);

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(c1.clone(), 1.0);
        scores.insert(c2.clone(), 0.9);
        scores.insert(c3.clone(), 0.8);
        scores.insert(c4.clone(), 0.7);

        let results = rerank_topk(&scores, 4, false);
        assert_eq!(results.len(), 4);

        // First chunk from main.rs gets full score (1.0)
        // Second chunk from main.rs: already_selected=1 >= threshold=1, excess=1, 0.9 * 0.5^1 = 0.45
        // Third chunk from main.rs: already_selected=2 >= threshold=1, excess=2, 0.8 * 0.5^2 = 0.2
        // other.rs: 0.7 (no penalty)
        // Sort: 1.0, 0.7, 0.45, 0.2
        assert_eq!(results[0].chunk, c1);
        assert_eq!(results[1].chunk, c4);
    }

    #[test]
    fn test_rerank_topk_empty() {
        let scores: HashMap<Chunk, f64> = HashMap::new();
        let results = rerank_topk(&scores, 10, true);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rerank_topk_no_penalties() {
        let c1 = make_chunk("tests/test_foo.py", 1);
        let c2 = make_chunk("src/main.rs", 1);

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(c1.clone(), 1.0);
        scores.insert(c2.clone(), 0.5);

        let results = rerank_topk(&scores, 2, false);
        // Without penalties, test file keeps its higher score
        assert_eq!(results[0].chunk, c1);
        assert_eq!(results[1].chunk, c2);
    }

    #[test]
    fn test_rerank_topk_limits_results() {
        let c1 = make_chunk("src/a.rs", 1);
        let c2 = make_chunk("src/b.rs", 1);
        let c3 = make_chunk("src/c.rs", 1);

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(c1, 1.0);
        scores.insert(c2, 0.9);
        scores.insert(c3, 0.8);

        let results = rerank_topk(&scores, 2, false);
        assert_eq!(results.len(), 2);
    }
}
