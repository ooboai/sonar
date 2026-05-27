use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// A single scored result from a vector search.
#[derive(Debug, Clone)]
pub struct Hit {
    pub index: usize,
    pub score: f64,
}

/// Brute-force cosine similarity index over L2-normalized vectors.
///
/// Since vectors are L2-normalized, cosine similarity = dot product.
/// Ported from ken's `internal/ann/flat.go`.
#[derive(Debug)]
pub struct Flat {
    vecs: Vec<Vec<f32>>,
    dim: usize,
}

impl Flat {
    /// Build a flat index from L2-normalized vectors (used by reference).
    pub fn new(vecs: Vec<Vec<f32>>) -> Self {
        let dim = vecs.first().map_or(0, |v| v.len());
        Flat { vecs, dim }
    }

    pub fn len(&self) -> usize {
        self.vecs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vecs.is_empty()
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Borrow the raw vector storage (for serialization).
    pub fn vecs(&self) -> &[Vec<f32>] {
        &self.vecs
    }

    /// Return the k highest cosine-similarity vectors to q, descending.
    /// Ties broken by ascending index for determinism.
    pub fn query(&self, q: &[f32], k: usize) -> Vec<Hit> {
        self.query_masked(q, k, None)
    }

    /// Return the k highest cosine-similarity vectors to q, descending,
    /// restricted to indices where `mask[i] == true` (if a mask is provided).
    pub fn query_masked(&self, q: &[f32], k: usize, mask: Option<&[bool]>) -> Vec<Hit> {
        if self.vecs.is_empty() || q.len() != self.dim {
            return Vec::new();
        }

        let candidates = self
            .vecs
            .iter()
            .enumerate()
            .filter(|(i, _)| mask.is_none_or(|m| m.get(*i).copied().unwrap_or(false)));

        if k == 0 {
            let mut hits: Vec<Hit> = candidates
                .map(|(i, v)| Hit {
                    index: i,
                    score: dot(v, q),
                })
                .collect();
            hits.sort_by(cmp_hits);
            return hits;
        }

        // Min-heap of size k for efficient top-k selection.
        let mut heap: BinaryHeap<MinHeapEntry> = BinaryHeap::new();

        for (i, v) in self.vecs.iter().enumerate() {
            if !mask.is_none_or(|m| m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            let score = dot(v, q);
            if heap.len() < k {
                heap.push(MinHeapEntry { index: i, score });
            } else if let Some(min) = heap.peek()
                && (score > min.score || (score == min.score && i < min.index))
            {
                heap.pop();
                heap.push(MinHeapEntry { index: i, score });
            }
        }

        let mut hits: Vec<Hit> = heap
            .into_iter()
            .map(|e| Hit {
                index: e.index,
                score: e.score,
            })
            .collect();
        hits.sort_by(cmp_hits);
        hits
    }
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| f64::from(x) * f64::from(y))
        .sum()
}

fn cmp_hits(a: &Hit, b: &Hit) -> Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.index.cmp(&b.index))
}

/// Min-heap entry: BinaryHeap is a max-heap, so we reverse the ordering.
struct MinHeapEntry {
    index: usize,
    score: f64,
}

impl PartialEq for MinHeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score && self.index == other.index
    }
}
impl Eq for MinHeapEntry {}

impl PartialOrd for MinHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MinHeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap behavior in BinaryHeap (which is max-heap).
        other
            .score
            .partial_cmp(&self.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.index.cmp(&self.index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_index() {
        let flat = Flat::new(vec![]);
        assert!(flat.is_empty());
        assert_eq!(flat.len(), 0);
        assert!(flat.query(&[1.0, 0.0], 5).is_empty());
    }

    #[test]
    fn test_single_vector() {
        let flat = Flat::new(vec![vec![1.0, 0.0, 0.0]]);
        let hits = flat.query(&[1.0, 0.0, 0.0], 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].index, 0);
        assert!((hits[0].score - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_ordering() {
        let flat = Flat::new(vec![
            vec![1.0, 0.0, 0.0], // idx 0: dot with query = 0.6
            vec![0.0, 1.0, 0.0], // idx 1: dot with query = 0.8
            vec![0.0, 0.0, 1.0], // idx 2: dot with query = 0.0
        ]);
        let q = vec![0.6, 0.8, 0.0];
        let hits = flat.query(&q, 3);

        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].index, 1); // highest dot product
        assert_eq!(hits[1].index, 0);
        assert_eq!(hits[2].index, 2);
    }

    #[test]
    fn test_top_k_limits() {
        let flat = Flat::new(vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.7, 0.7]]);
        let hits = flat.query(&[1.0, 0.0], 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].index, 0); // exact match
    }

    #[test]
    fn test_dimension_mismatch() {
        let flat = Flat::new(vec![vec![1.0, 0.0]]);
        let hits = flat.query(&[1.0, 0.0, 0.0], 1);
        assert!(hits.is_empty());
    }

    #[test]
    fn test_tie_breaking() {
        let flat = Flat::new(vec![
            vec![1.0, 0.0],
            vec![1.0, 0.0], // same vector, should come after idx 0
        ]);
        let hits = flat.query(&[1.0, 0.0], 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].index, 0); // lower index wins tie
        assert_eq!(hits[1].index, 1);
    }

    #[test]
    fn test_k_zero_returns_all() {
        let flat = Flat::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        let hits = flat.query(&[1.0, 0.0], 0);
        assert_eq!(hits.len(), 2);
    }
}
