use std::collections::HashMap;
use std::path::Path;

use crate::ann::Flat;
use crate::bm25::BM25Index;
use crate::chunk::chunk_source;
use crate::embed::Embedder;
use crate::tokens::tokenize;
use crate::types::{Chunk, IndexStats, SearchResult};
use crate::utils::enrich_for_bm25;
use crate::walk::walk_directory;

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
            tracing::warn!("Loaded cached vectors but embedder unavailable; hybrid queries will fall back to BM25");
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
        if let Some(index) = crate::persist::load_cached(path)? {
            return Ok(index);
        }
        crate::persist::build_and_save(path)
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
        if !path.exists() {
            return Err(format!("Path does not exist: {}", path.display()));
        }
        if !path.is_dir() {
            return Err(format!("Path is not a directory: {}", path.display()));
        }

        let walked = walk_directory(path);
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

        match self.mode {
            Mode::Bm25 => self.search_bm25(query, top_k),
            Mode::Semantic => self.search_semantic(query, top_k),
            Mode::Hybrid => {
                if self.embedder.is_some() && self.flat.is_some() {
                    self.search_hybrid(query, top_k)
                } else {
                    self.search_bm25(query, top_k)
                }
            }
        }
    }

    fn search_bm25(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let tokens = tokenize(query);
        let scored = self.bm25.search(&tokens, top_k * 5);

        let mut bm25_scores: HashMap<Chunk, f64> = HashMap::new();
        for (idx, score) in &scored {
            bm25_scores.insert(self.chunks[*idx].clone(), *score);
        }

        crate::rank::boost::boost_multi_chunk_files(&mut bm25_scores);
        let boosted = crate::rank::boost::apply_query_boost(&bm25_scores, query, &self.chunks);
        crate::rank::penalty::rerank_topk(&boosted, top_k, true)
    }

    fn search_semantic(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let (embedder, flat) = match (&self.embedder, &self.flat) {
            (Some(e), Some(f)) => (e, f),
            _ => return self.search_bm25(query, top_k),
        };

        let q_vec = embedder.encode(query);
        let hits = flat.query(&q_vec, top_k * 5);

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        for h in &hits {
            scores.insert(self.chunks[h.index].clone(), h.score);
        }

        crate::rank::boost::boost_multi_chunk_files(&mut scores);
        let boosted = crate::rank::boost::apply_query_boost(&scores, query, &self.chunks);
        crate::rank::penalty::rerank_topk(&boosted, top_k, false)
    }

    fn search_hybrid(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let (embedder, flat) = match (&self.embedder, &self.flat) {
            (Some(e), Some(f)) => (e, f),
            _ => return self.search_bm25(query, top_k),
        };

        let q_vec = embedder.encode(query);
        let sem_hits = flat.query(&q_vec, top_k * 5);

        let mut semantic_scores: HashMap<Chunk, f64> = HashMap::new();
        for hit in &sem_hits {
            semantic_scores.insert(self.chunks[hit.index].clone(), hit.score);
        }

        let tokens = tokenize(query);
        let bm25_raw = self.bm25.search(&tokens, top_k * 5);
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
            None,
            true,
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

        hits.into_iter()
            .filter(|h| &self.chunks[h.index] != chunk)
            .take(top_k)
            .map(|h| SearchResult {
                chunk: self.chunks[h.index].clone(),
                score: h.score,
            })
            .collect()
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
}
