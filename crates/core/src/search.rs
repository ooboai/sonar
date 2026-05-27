use std::collections::HashMap;

use crate::rank::boost::{apply_query_boost, boost_multi_chunk_files};
use crate::rank::penalty::rerank_topk;
use crate::types::{Chunk, SearchResult};

/// RRF constant k. Ported from semble's `search.py::_RRF_K`.
const RRF_K: f64 = 60.0;

/// Convert raw scores to Reciprocal Rank Fusion scores: 1/(k + rank).
/// Higher raw score -> rank 1.
///
/// Ported verbatim from semble's `search.py::_rrf_scores`.
pub fn rrf_scores(scores: &HashMap<Chunk, f64>) -> HashMap<Chunk, f64> {
    if scores.is_empty() {
        return HashMap::new();
    }

    let mut ranked: Vec<_> = scores.iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));

    ranked
        .into_iter()
        .enumerate()
        .map(|(rank, (chunk, _))| (chunk.clone(), 1.0 / (RRF_K + (rank + 1) as f64)))
        .collect()
}

/// Top-level hybrid search combining semantic + BM25 via RRF.
///
/// Accepts pre-computed semantic and BM25 result scores (since the index is not
/// yet built). Applies RRF normalization, alpha-weighted combination, and
/// optional reranking (multi-chunk boost, query boost, path penalties).
///
/// Ported from semble's `search.py::search`.
pub fn hybrid_search(
    query: &str,
    semantic_scores: &HashMap<Chunk, f64>,
    bm25_scores: &HashMap<Chunk, f64>,
    all_chunks: &[Chunk],
    top_k: usize,
    alpha: Option<f64>,
    rerank: bool,
) -> Vec<SearchResult> {
    let alpha_weight = crate::rank::resolve_alpha(query, alpha);

    let normalized_semantic = rrf_scores(semantic_scores);
    let normalized_bm25 = rrf_scores(bm25_scores);

    // Union of all candidate chunks, sorted by start_line for determinism.
    let mut all_candidates: Vec<&Chunk> = normalized_semantic
        .keys()
        .chain(normalized_bm25.keys())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    all_candidates.sort_by_key(|c| c.start_line);

    let mut combined_scores: HashMap<Chunk, f64> = all_candidates
        .into_iter()
        .map(|chunk| {
            let sem = normalized_semantic.get(chunk).copied().unwrap_or(0.0);
            let bm25 = normalized_bm25.get(chunk).copied().unwrap_or(0.0);
            (
                chunk.clone(),
                alpha_weight * sem + (1.0 - alpha_weight) * bm25,
            )
        })
        .collect();

    if rerank {
        boost_multi_chunk_files(&mut combined_scores);
        combined_scores = apply_query_boost(&combined_scores, query, all_chunks);
        rerank_topk(&combined_scores, top_k, alpha_weight < 1.0)
    } else {
        let mut sorted: Vec<_> = combined_scores.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted
            .into_iter()
            .take(top_k)
            .map(|(chunk, score)| SearchResult { chunk, score })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(content: &str, file_path: &str, start_line: usize) -> Chunk {
        Chunk {
            content: content.to_string(),
            file_path: file_path.to_string(),
            start_line,
            end_line: start_line + 10,
            language: Some("python".to_string()),
        }
    }

    #[test]
    fn test_rrf_scores_empty() {
        let scores: HashMap<Chunk, f64> = HashMap::new();
        assert!(rrf_scores(&scores).is_empty());
    }

    #[test]
    fn test_rrf_scores_ordering() {
        let c1 = make_chunk("a", "a.py", 1);
        let c2 = make_chunk("b", "b.py", 1);
        let c3 = make_chunk("c", "c.py", 1);

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(c1.clone(), 10.0);
        scores.insert(c2.clone(), 5.0);
        scores.insert(c3.clone(), 1.0);

        let rrf = rrf_scores(&scores);
        // Rank 1 -> 1/(60+1), Rank 2 -> 1/(60+2), Rank 3 -> 1/(60+3)
        assert!((rrf[&c1] - 1.0 / 61.0).abs() < 1e-10);
        assert!((rrf[&c2] - 1.0 / 62.0).abs() < 1e-10);
        assert!((rrf[&c3] - 1.0 / 63.0).abs() < 1e-10);
    }

    #[test]
    fn test_hybrid_search_no_rerank() {
        let c1 = make_chunk("fn main() {}", "src/main.rs", 1);
        let c2 = make_chunk("fn helper() {}", "src/lib.rs", 1);

        let mut semantic: HashMap<Chunk, f64> = HashMap::new();
        semantic.insert(c1.clone(), 0.9);
        semantic.insert(c2.clone(), 0.7);

        let mut bm25: HashMap<Chunk, f64> = HashMap::new();
        bm25.insert(c1.clone(), 0.8);
        bm25.insert(c2.clone(), 0.6);

        let all_chunks = vec![c1.clone(), c2.clone()];
        let results = hybrid_search(
            "main function",
            &semantic,
            &bm25,
            &all_chunks,
            2,
            None,
            false,
        );

        assert_eq!(results.len(), 2);
        // c1 has rank 1 in both, c2 has rank 2 in both
        // So c1 should have higher combined score
        assert_eq!(results[0].chunk, c1);
        assert_eq!(results[1].chunk, c2);
    }

    #[test]
    fn test_hybrid_search_with_rerank() {
        let c1 = make_chunk("class Router:\n    pass", "src/router.py", 1);
        let c2 = make_chunk("x = Router()", "src/main.py", 1);

        let mut semantic: HashMap<Chunk, f64> = HashMap::new();
        semantic.insert(c1.clone(), 0.8);
        semantic.insert(c2.clone(), 0.9);

        let mut bm25: HashMap<Chunk, f64> = HashMap::new();
        bm25.insert(c1.clone(), 0.7);
        bm25.insert(c2.clone(), 0.8);

        let all_chunks = vec![c1.clone(), c2.clone()];
        let results = hybrid_search("Router", &semantic, &bm25, &all_chunks, 2, None, true);

        assert_eq!(results.len(), 2);
        // With reranking, c1 defines Router and its stem matches -> should be boosted to top
        assert_eq!(results[0].chunk, c1);
    }

    #[test]
    fn test_hybrid_search_empty_inputs() {
        let semantic: HashMap<Chunk, f64> = HashMap::new();
        let bm25: HashMap<Chunk, f64> = HashMap::new();
        let results = hybrid_search("query", &semantic, &bm25, &[], 10, None, true);
        assert!(results.is_empty());
    }

    #[test]
    fn test_hybrid_search_top_k_limit() {
        let c1 = make_chunk("a", "a.py", 1);
        let c2 = make_chunk("b", "b.py", 10);
        let c3 = make_chunk("c", "c.py", 20);

        let mut semantic: HashMap<Chunk, f64> = HashMap::new();
        semantic.insert(c1.clone(), 0.9);
        semantic.insert(c2.clone(), 0.8);
        semantic.insert(c3.clone(), 0.7);

        let bm25: HashMap<Chunk, f64> = HashMap::new();
        let all_chunks = vec![c1, c2, c3];

        let results = hybrid_search("something", &semantic, &bm25, &all_chunks, 2, None, false);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_hybrid_search_alpha_override() {
        let c1 = make_chunk("fn a() {}", "src/a.rs", 1);

        let mut semantic: HashMap<Chunk, f64> = HashMap::new();
        semantic.insert(c1.clone(), 1.0);

        let bm25: HashMap<Chunk, f64> = HashMap::new();
        let all_chunks = vec![c1.clone()];

        // alpha=1.0 means only semantic scores matter
        let results = hybrid_search(
            "something",
            &semantic,
            &bm25,
            &all_chunks,
            1,
            Some(1.0),
            false,
        );
        assert_eq!(results.len(), 1);
        // With alpha=1.0, full semantic RRF score: 1/(60+1) * 1.0 = 0.01639...
        let expected = 1.0 / 61.0;
        assert!((results[0].score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_hybrid_search_bm25_only() {
        let c1 = make_chunk("fn a() {}", "src/a.rs", 1);

        let semantic: HashMap<Chunk, f64> = HashMap::new();

        let mut bm25: HashMap<Chunk, f64> = HashMap::new();
        bm25.insert(c1.clone(), 1.0);

        let all_chunks = vec![c1.clone()];

        // alpha=0.0 means only BM25 scores matter
        let results = hybrid_search(
            "something",
            &semantic,
            &bm25,
            &all_chunks,
            1,
            Some(0.0),
            false,
        );
        assert_eq!(results.len(), 1);
        let expected = 1.0 / 61.0;
        assert!((results[0].score - expected).abs() < 1e-10);
    }

    #[test]
    fn test_hybrid_search_disjoint_results() {
        let c1 = make_chunk("semantic only", "src/sem.rs", 1);
        let c2 = make_chunk("bm25 only", "src/bm25.rs", 10);

        let mut semantic: HashMap<Chunk, f64> = HashMap::new();
        semantic.insert(c1.clone(), 1.0);

        let mut bm25: HashMap<Chunk, f64> = HashMap::new();
        bm25.insert(c2.clone(), 1.0);

        let all_chunks = vec![c1.clone(), c2.clone()];
        let results = hybrid_search("query", &semantic, &bm25, &all_chunks, 2, Some(0.5), false);

        assert_eq!(results.len(), 2);
        // Both get RRF rank 1 in their respective lists: 1/(60+1) = 0.01639
        // c1: 0.5 * (1/61) + 0.5 * 0 = 0.5/61
        // c2: 0.5 * 0 + 0.5 * (1/61) = 0.5/61
        // They should have equal scores
        assert!((results[0].score - results[1].score).abs() < 1e-10);
    }
}
