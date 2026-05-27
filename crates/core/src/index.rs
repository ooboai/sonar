use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::ann::Flat;
use crate::bm25::BM25Index;
use crate::chunk::chunk_source;
use crate::embed::Embedder;
use crate::stats::{UsageRecord, record_usage};
use crate::tokens::tokenize;
use crate::types::{Chunk, ContentType, IndexStats, SearchResult};
use crate::utils::enrich_for_bm25;
use crate::walk::walk_directory;

/// Options for filtered search.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub top_k: usize,
    pub alpha: Option<f64>,
    pub filter_languages: Option<Vec<String>>,
    pub filter_paths: Option<Vec<String>>,
    pub rerank: Option<bool>,
}

/// Controls which retrieval backends are used for search.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// BM25 keyword search only.
    Bm25,
    /// Vector similarity only (requires embeddings).
    Semantic,
    /// RRF combination of semantic + BM25 (requires embeddings).
    #[default]
    Hybrid,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Bm25 => write!(f, "bm25"),
            Mode::Semantic => write!(f, "semantic"),
            Mode::Hybrid => write!(f, "hybrid"),
        }
    }
}

/// Main index entry point — supports BM25-only, semantic-only, and hybrid search.
///
/// Ported from semble's `index/index.py::SembleIndex`.
pub struct SonarIndex {
    chunks: Vec<Chunk>,
    bm25: BM25Index,
    file_mapping: HashMap<String, Vec<usize>>,
    language_mapping: HashMap<String, Vec<usize>>,
    embedder: Option<Embedder>,
    flat: Option<Flat>,
    mode: Mode,
}

impl std::fmt::Debug for SonarIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SonarIndex")
            .field("chunks", &self.chunks.len())
            .field("mode", &self.mode)
            .field("has_embeddings", &self.embedder.is_some())
            .finish()
    }
}

impl SonarIndex {
    /// Direct constructor for BM25-only (used by persist and tests).
    pub fn new(
        chunks: Vec<Chunk>,
        bm25: BM25Index,
        file_mapping: HashMap<String, Vec<usize>>,
        language_mapping: HashMap<String, Vec<usize>>,
    ) -> Self {
        Self {
            chunks,
            bm25,
            file_mapping,
            language_mapping,
            embedder: None,
            flat: None,
            mode: Mode::Bm25,
        }
    }

    /// Full hybrid constructor with both embedder and flat index.
    pub fn new_hybrid(
        chunks: Vec<Chunk>,
        bm25: BM25Index,
        file_mapping: HashMap<String, Vec<usize>>,
        language_mapping: HashMap<String, Vec<usize>>,
        embedder: Embedder,
        flat: Flat,
    ) -> Self {
        Self {
            chunks,
            bm25,
            file_mapping,
            language_mapping,
            embedder: Some(embedder),
            flat: Some(flat),
            mode: Mode::Hybrid,
        }
    }

    /// Constructor with pre-built vector index but no embedder (loaded from cache).
    /// Eagerly loads the embedder so hybrid/semantic queries work immediately.
    pub fn new_with_vectors(
        chunks: Vec<Chunk>,
        bm25: BM25Index,
        file_mapping: HashMap<String, Vec<usize>>,
        language_mapping: HashMap<String, Vec<usize>>,
        flat: Flat,
    ) -> Self {
        let embedder = Embedder::load_default().ok();
        let mode = if embedder.is_some() {
            Mode::Hybrid
        } else {
            tracing::warn!(
                "Loaded cached vectors but embedder unavailable; hybrid queries will fall back to BM25"
            );
            Mode::Bm25
        };
        Self {
            chunks,
            bm25,
            file_mapping,
            language_mapping,
            embedder,
            flat: Some(flat),
            mode,
        }
    }

    /// Try loading a cached index; rebuild (and save) if stale or missing.
    /// Uses v2 persist format which stores embedding vectors for hybrid roundtrip.
    pub fn from_path_cached(path: &Path) -> Result<Self, String> {
        Self::from_path_cached_with_content(path, &[ContentType::Code])
    }

    /// Try loading a cached index with content type filter.
    pub fn from_path_cached_with_content(
        path: &Path,
        content_types: &[ContentType],
    ) -> Result<Self, String> {
        if let Some(index) = crate::persist::load_cached_content(path, content_types)? {
            return Ok(index);
        }
        crate::persist::build_and_save_content(path, content_types)
    }

    /// Clone a remote git repository and index it.
    ///
    /// Ported from semble's `index/index.py::SembleIndex.from_git`.
    ///
    /// - Always shallow clones (`--depth 1`)
    /// - Optional `git_ref` for a specific branch or tag
    /// - URL must start with `https://` or `http://` for safety
    /// - Timeout controlled by `SONAR_CLONE_TIMEOUT` env var (default 60s)
    /// - Clones into a temp directory which is cleaned up automatically
    pub fn from_git(
        url: &str,
        git_ref: Option<&str>,
        _content: &[ContentType],
    ) -> Result<Self, String> {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(format!(
                "Only https:// and http:// URLs are supported, got: {url}"
            ));
        }

        let tmp_dir =
            tempfile::tempdir().map_err(|e| format!("Failed to create temp directory: {e}"))?;

        let mut args = vec!["clone", "--depth", "1"];
        let ref_string;
        if let Some(r) = git_ref {
            ref_string = r.to_string();
            args.push("--branch");
            args.push(&ref_string);
        }
        args.push("--");
        args.push(url);
        let tmp_path_str = tmp_dir.path().to_string_lossy().to_string();
        args.push(&tmp_path_str);

        let mut child = Command::new("git")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git not found on PATH: {e}"))?;

        let timeout = Duration::from_secs(
            std::env::var("SONAR_CLONE_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60),
        );

        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if start.elapsed() > timeout {
                        child.kill().ok();
                        return Err(format!("git clone timed out after {}s", timeout.as_secs()));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(format!("Error waiting for git clone: {e}")),
            }
        };

        if !status.success() {
            let stderr = child
                .stderr
                .take()
                .and_then(|mut s| {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                    Some(buf)
                })
                .unwrap_or_default();
            return Err(format!(
                "git clone failed (exit {}): {}",
                status,
                stderr.trim()
            ));
        }

        Self::from_path(tmp_dir.path())
    }

    /// Walk `path`, chunk every supported file, and build an index.
    /// Falls back to BM25-only if the embedding model is unavailable.
    pub fn from_path(path: &Path) -> Result<Self, String> {
        Self::from_path_with_mode(path, Mode::default())
    }

    /// Walk `path` and build an index with the requested mode.
    /// If `Hybrid` or `Semantic` is requested but the model can't load,
    /// falls back to `Bm25` and logs a warning.
    pub fn from_path_with_mode(path: &Path, requested_mode: Mode) -> Result<Self, String> {
        Self::from_path_with_mode_and_content(path, requested_mode, &[ContentType::Code])
    }

    /// Walk `path` and build an index with the requested mode and content types.
    pub fn from_path_with_mode_and_content(
        path: &Path,
        requested_mode: Mode,
        content_types: &[ContentType],
    ) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!("Path does not exist: {}", path.display()));
        }
        if !path.is_dir() {
            return Err(format!("Path is not a directory: {}", path.display()));
        }

        let walked = walk_directory(path, content_types);
        if walked.is_empty() {
            return Err(format!("No supported files found under {}", path.display()));
        }

        let mut chunks = Vec::new();
        for file in &walked {
            chunks.extend(chunk_source(
                &file.content,
                &file.relative_path,
                file.language.as_deref(),
            ));
        }

        if chunks.is_empty() {
            return Err("No chunks produced from files".to_string());
        }

        let documents: Vec<Vec<String>> = chunks
            .iter()
            .map(|c| tokenize(&enrich_for_bm25(c)))
            .collect();
        let bm25 = BM25Index::build(&documents);

        let mut file_mapping: HashMap<String, Vec<usize>> = HashMap::new();
        let mut language_mapping: HashMap<String, Vec<usize>> = HashMap::new();

        for (i, chunk) in chunks.iter().enumerate() {
            file_mapping
                .entry(chunk.file_path.clone())
                .or_default()
                .push(i);
            if let Some(ref lang) = chunk.language {
                language_mapping.entry(lang.clone()).or_default().push(i);
            }
        }

        let (embedder, flat, actual_mode) = if requested_mode == Mode::Hybrid
            || requested_mode == Mode::Semantic
        {
            match Embedder::load_default() {
                Ok(emb) => {
                    let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
                    let vecs = emb.encode_batch(&texts);
                    let flat_idx = Flat::new(vecs);
                    (Some(emb), Some(flat_idx), requested_mode)
                }
                Err(e) => {
                    tracing::warn!("Embedding model unavailable, falling back to BM25-only: {e}");
                    (None, None, Mode::Bm25)
                }
            }
        } else {
            (None, None, Mode::Bm25)
        };

        Ok(SonarIndex {
            chunks,
            bm25,
            file_mapping,
            language_mapping,
            embedder,
            flat,
            mode: actual_mode,
        })
    }

    /// Search using the active mode.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        if self.chunks.is_empty() || query.trim().is_empty() {
            return Vec::new();
        }

        let results = match self.mode {
            Mode::Bm25 => self.search_bm25(query, top_k),
            Mode::Semantic => self.search_semantic(query, top_k),
            Mode::Hybrid => {
                if self.embedder.is_some() && self.flat.is_some() {
                    self.search_hybrid(query, top_k)
                } else {
                    self.search_bm25(query, top_k)
                }
            }
        };

        self.record_search_usage("search", &results);
        results
    }

    /// Search with filter options (language/path filtering, alpha, rerank control).
    pub fn search_with_options(&self, query: &str, opts: &SearchOptions) -> Vec<SearchResult> {
        if self.chunks.is_empty() || query.trim().is_empty() {
            return Vec::new();
        }

        let mask = self.build_filter_mask(
            opts.filter_languages.as_deref(),
            opts.filter_paths.as_deref(),
        );

        let rerank = opts.rerank.unwrap_or(true);

        match self.mode {
            Mode::Bm25 => self.search_bm25_masked(query, opts.top_k, mask.as_deref()),
            Mode::Semantic => self.search_semantic_masked(query, opts.top_k, mask.as_deref()),
            Mode::Hybrid => {
                if self.embedder.is_some() && self.flat.is_some() {
                    self.search_hybrid_masked(
                        query,
                        opts.top_k,
                        opts.alpha,
                        rerank,
                        mask.as_deref(),
                    )
                } else {
                    self.search_bm25_masked(query, opts.top_k, mask.as_deref())
                }
            }
        }
    }

    /// Build a boolean mask from language and path filters.
    /// Returns None if no filters are active (i.e. all chunks pass).
    fn build_filter_mask(
        &self,
        filter_languages: Option<&[String]>,
        filter_paths: Option<&[String]>,
    ) -> Option<Vec<bool>> {
        let has_lang = filter_languages.is_some_and(|v| !v.is_empty());
        let has_path = filter_paths.is_some_and(|v| !v.is_empty());

        if !has_lang && !has_path {
            return None;
        }

        let n = self.chunks.len();

        let lang_indices: Option<Vec<usize>> = if has_lang {
            let langs = filter_languages.unwrap();
            let mut indices = Vec::new();
            for lang in langs {
                if let Some(chunk_ids) = self.language_mapping.get(lang) {
                    indices.extend(chunk_ids);
                }
            }
            Some(indices)
        } else {
            None
        };

        let path_indices: Option<Vec<usize>> = if has_path {
            let paths = filter_paths.unwrap();
            let mut indices = Vec::new();
            for (file_path, chunk_ids) in &self.file_mapping {
                if paths
                    .iter()
                    .any(|p| file_path == p || file_path.starts_with(p))
                {
                    indices.extend(chunk_ids);
                }
            }
            Some(indices)
        } else {
            None
        };

        // Intersect the two masks if both are present, otherwise use whichever is active.
        let lang_mask = lang_indices
            .as_deref()
            .and_then(|sel| crate::utils::selector_to_mask(Some(sel), n));
        let path_mask = path_indices
            .as_deref()
            .and_then(|sel| crate::utils::selector_to_mask(Some(sel), n));

        match (lang_mask, path_mask) {
            (Some(lm), Some(pm)) => Some(lm.iter().zip(pm.iter()).map(|(a, b)| *a && *b).collect()),
            (Some(m), None) | (None, Some(m)) => Some(m),
            (None, None) => None,
        }
    }

    fn search_bm25(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        self.search_bm25_masked(query, top_k, None)
    }

    fn search_bm25_masked(
        &self,
        query: &str,
        top_k: usize,
        mask: Option<&[bool]>,
    ) -> Vec<SearchResult> {
        let tokens = tokenize(query);
        let scored = self.bm25.search_masked(&tokens, top_k * 5, mask);

        let mut bm25_scores: HashMap<Chunk, f64> = HashMap::new();
        for (idx, score) in &scored {
            bm25_scores.insert(self.chunks[*idx].clone(), *score);
        }

        crate::rank::boost::boost_multi_chunk_files(&mut bm25_scores);
        let boosted = crate::rank::boost::apply_query_boost(&bm25_scores, query, &self.chunks);
        crate::rank::penalty::rerank_topk(&boosted, top_k, true)
    }

    fn search_semantic(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        self.search_semantic_masked(query, top_k, None)
    }

    fn search_semantic_masked(
        &self,
        query: &str,
        top_k: usize,
        mask: Option<&[bool]>,
    ) -> Vec<SearchResult> {
        let (embedder, flat) = match (&self.embedder, &self.flat) {
            (Some(e), Some(f)) => (e, f),
            _ => return self.search_bm25_masked(query, top_k, mask),
        };

        let q_vec = embedder.encode(query);
        let hits = flat.query_masked(&q_vec, top_k * 5, mask);

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        for h in &hits {
            scores.insert(self.chunks[h.index].clone(), h.score);
        }

        crate::rank::boost::boost_multi_chunk_files(&mut scores);
        let boosted = crate::rank::boost::apply_query_boost(&scores, query, &self.chunks);
        crate::rank::penalty::rerank_topk(&boosted, top_k, false)
    }

    fn search_hybrid(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        self.search_hybrid_masked(query, top_k, None, true, None)
    }

    fn search_hybrid_masked(
        &self,
        query: &str,
        top_k: usize,
        alpha: Option<f64>,
        rerank: bool,
        mask: Option<&[bool]>,
    ) -> Vec<SearchResult> {
        let (embedder, flat) = match (&self.embedder, &self.flat) {
            (Some(e), Some(f)) => (e, f),
            _ => return self.search_bm25_masked(query, top_k, mask),
        };

        let q_vec = embedder.encode(query);
        let sem_hits = flat.query_masked(&q_vec, top_k * 5, mask);

        let mut semantic_scores: HashMap<Chunk, f64> = HashMap::new();
        for hit in &sem_hits {
            semantic_scores.insert(self.chunks[hit.index].clone(), hit.score);
        }

        let tokens = tokenize(query);
        let bm25_raw = self.bm25.search_masked(&tokens, top_k * 5, mask);
        let mut bm25_scores: HashMap<Chunk, f64> = HashMap::new();
        for (idx, score) in &bm25_raw {
            bm25_scores.insert(self.chunks[*idx].clone(), *score);
        }

        crate::search::hybrid_search(
            query,
            &semantic_scores,
            &bm25_scores,
            &self.chunks,
            top_k,
            alpha,
            rerank,
        )
    }

    /// Find chunks semantically related to the given chunk.
    /// Requires embeddings; returns empty if unavailable.
    pub fn find_related(&self, chunk: &Chunk, top_k: usize) -> Vec<SearchResult> {
        let (embedder, flat) = match (&self.embedder, &self.flat) {
            (Some(e), Some(f)) => (e, f),
            _ => return Vec::new(),
        };

        let q_vec = embedder.encode(&chunk.content);
        let hits = flat.query(&q_vec, top_k + 1);

        let results: Vec<SearchResult> = hits
            .into_iter()
            .filter(|h| &self.chunks[h.index] != chunk)
            .take(top_k)
            .map(|h| SearchResult {
                chunk: self.chunks[h.index].clone(),
                score: h.score,
            })
            .collect();

        self.record_search_usage("find_related", &results);
        results
    }

    /// Record usage stats for savings tracking (best-effort, failures are silent).
    fn record_search_usage(&self, call: &str, results: &[SearchResult]) {
        if results.is_empty() {
            return;
        }

        let snippet_chars: usize = results.iter().map(|r| r.chunk.content.len()).sum();

        let unique_files: HashSet<&str> =
            results.iter().map(|r| r.chunk.file_path.as_str()).collect();
        let file_chars: usize = unique_files
            .iter()
            .map(|fp| {
                self.file_mapping
                    .get(*fp)
                    .map(|indices| {
                        indices
                            .iter()
                            .map(|&i| self.chunks[i].content.len())
                            .sum::<usize>()
                    })
                    .unwrap_or(0)
            })
            .sum();

        let record = UsageRecord::now(call, results.len(), snippet_chars, file_chars);
        let _ = record_usage(&record);
    }

    pub fn stats(&self) -> IndexStats {
        let mut languages: HashMap<String, usize> = HashMap::new();
        for chunk in &self.chunks {
            if let Some(ref lang) = chunk.language {
                *languages.entry(lang.clone()).or_default() += 1;
            }
        }
        IndexStats {
            indexed_files: self.file_mapping.len(),
            total_chunks: self.chunks.len(),
            languages,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Override the search mode (e.g. to force BM25 on a hybrid index).
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn has_embeddings(&self) -> bool {
        self.flat.is_some()
    }

    pub fn flat(&self) -> Option<&Flat> {
        self.flat.as_ref()
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn file_mapping(&self) -> &HashMap<String, Vec<usize>> {
        &self.file_mapping
    }

    pub fn language_mapping(&self) -> &HashMap<String, Vec<usize>> {
        &self.language_mapping
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn test_from_git_rejects_non_http_urls() {
        let result = SonarIndex::from_git("git@github.com:org/repo.git", None, &[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Only https:// and http://"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_from_git_rejects_ssh_url() {
        let result = SonarIndex::from_git("ssh://git@github.com/org/repo", None, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Only https://"));
    }

    #[test]
    fn test_from_git_rejects_file_url() {
        let result = SonarIndex::from_git("file:///tmp/repo", None, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Only https://"));
    }

    #[test]
    fn test_from_git_invalid_repo_url() {
        let result = SonarIndex::from_git(
            "https://github.com/this-org-does-not-exist-zzz/no-repo-here",
            None,
            &[],
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("git clone failed") || err.contains("git not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_index_from_path_on_self() {
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index the sonar repo itself");
        let stats = index.stats();
        assert!(stats.indexed_files > 0, "should index some files");
        assert!(stats.total_chunks > 0, "should produce some chunks");
        assert!(
            stats.languages.contains_key("rust"),
            "should detect rust files"
        );
    }

    #[test]
    fn test_search_returns_results() {
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index");
        let results = index.search("chunk_source", 5);
        assert!(!results.is_empty(), "should find chunk_source");
        assert!(
            results[0].chunk.content.contains("chunk_source"),
            "top result should contain the query term"
        );
    }

    #[test]
    fn test_search_empty_query() {
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index");
        let results = index.search("", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_find_related_empty_without_embeddings() {
        let chunk = Chunk {
            content: "test".to_string(),
            file_path: "test.rs".to_string(),
            start_line: 1,
            end_line: 1,
            language: Some("rust".to_string()),
        };
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index");
        // Without the model downloaded, find_related returns empty.
        if !index.has_embeddings() {
            let results = index.find_related(&chunk, 5);
            assert!(results.is_empty());
        }
    }

    #[test]
    fn test_bm25_only_mode() {
        let root = repo_root();
        let index =
            SonarIndex::from_path_with_mode(&root, Mode::Bm25).expect("should index in BM25 mode");
        assert_eq!(index.mode(), Mode::Bm25);
        assert!(!index.has_embeddings());
        let results = index.search("chunk_source", 5);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_from_path_falls_back_gracefully() {
        let root = repo_root();
        // Request hybrid — if model isn't available, should fall back to BM25 without error.
        let index = SonarIndex::from_path_with_mode(&root, Mode::Hybrid).expect("should index");
        let results = index.search("chunk_source", 5);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_with_language_filter() {
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index");
        let opts = SearchOptions {
            top_k: 5,
            alpha: None,
            filter_languages: Some(vec!["rust".to_string()]),
            filter_paths: None,
            rerank: None,
        };
        let results = index.search_with_options("chunk_source", &opts);
        assert!(!results.is_empty());
        for r in &results {
            assert_eq!(r.chunk.language.as_deref(), Some("rust"));
        }
    }

    #[test]
    fn test_search_with_nonexistent_language_filter() {
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index");
        let opts = SearchOptions {
            top_k: 5,
            alpha: None,
            filter_languages: Some(vec!["cobol".to_string()]),
            filter_paths: None,
            rerank: None,
        };
        let results = index.search_with_options("chunk_source", &opts);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_with_path_filter() {
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index");
        let opts = SearchOptions {
            top_k: 5,
            alpha: None,
            filter_languages: None,
            filter_paths: Some(vec!["crates/core/src/".to_string()]),
            rerank: None,
        };
        let results = index.search_with_options("chunk_source", &opts);
        assert!(!results.is_empty());
        for r in &results {
            assert!(
                r.chunk.file_path.starts_with("crates/core/src/"),
                "result file_path {} should start with filter prefix",
                r.chunk.file_path
            );
        }
    }

    #[test]
    fn test_search_with_no_filters_matches_search() {
        let root = repo_root();
        let index = SonarIndex::from_path(&root).expect("should index");
        let opts = SearchOptions {
            top_k: 5,
            alpha: None,
            filter_languages: None,
            filter_paths: None,
            rerank: None,
        };
        let filtered = index.search_with_options("chunk_source", &opts);
        let unfiltered = index.search("chunk_source", 5);
        assert_eq!(filtered.len(), unfiltered.len());
        for (f, u) in filtered.iter().zip(unfiltered.iter()) {
            assert_eq!(f.chunk.file_path, u.chunk.file_path);
        }
    }
}
