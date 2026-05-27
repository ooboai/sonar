use std::collections::HashMap;

const K1: f64 = 1.5;
const B: f64 = 0.75;

/// Minimal BM25 index for keyword search.
///
/// Follows the Okapi BM25 scoring formula used by semble (via bm25s).
#[derive(Debug)]
pub struct BM25Index {
    tf: Vec<HashMap<String, f64>>,
    df: HashMap<String, usize>,
    doc_lengths: Vec<f64>,
    avg_dl: f64,
    n_docs: usize,
}

impl BM25Index {
    /// Build a BM25 index from pre-tokenized documents.
    pub fn build(documents: &[Vec<String>]) -> Self {
        let n_docs = documents.len();
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut tf = Vec::with_capacity(n_docs);
        let mut doc_lengths = Vec::with_capacity(n_docs);

        for doc in documents {
            let mut term_freq: HashMap<String, f64> = HashMap::new();
            for term in doc {
                *term_freq.entry(term.clone()).or_default() += 1.0;
            }
            for key in term_freq.keys() {
                *df.entry(key.clone()).or_default() += 1;
            }
            doc_lengths.push(doc.len() as f64);
            tf.push(term_freq);
        }

        let avg_dl = if n_docs > 0 {
            doc_lengths.iter().sum::<f64>() / n_docs as f64
        } else {
            0.0
        };

        BM25Index {
            tf,
            df,
            doc_lengths,
            avg_dl,
            n_docs,
        }
    }

    /// Score a single document against a query.
    fn score(&self, query: &[String], doc_idx: usize) -> f64 {
        let dl = self.doc_lengths[doc_idx];
        let tf_map = &self.tf[doc_idx];
        let mut total = 0.0;

        for term in query {
            let df = *self.df.get(term).unwrap_or(&0) as f64;
            let tf = *tf_map.get(term).unwrap_or(&0.0);
            if tf == 0.0 {
                continue;
            }

            let idf = ((self.n_docs as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();
            let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * dl / self.avg_dl));
            total += idf * tf_norm;
        }

        total
    }

    /// Return the `top_k` highest-scoring (doc_index, score) pairs.
    pub fn search(&self, query: &[String], top_k: usize) -> Vec<(usize, f64)> {
        let mut scores: Vec<(usize, f64)> = (0..self.n_docs)
            .map(|i| (i, self.score(query, i)))
            .filter(|(_, s)| *s > 0.0)
            .collect();

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores.truncate(top_k);
        scores
    }

    pub fn n_docs(&self) -> usize {
        self.n_docs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn test_empty_index() {
        let idx = BM25Index::build(&[]);
        assert_eq!(idx.n_docs(), 0);
        assert!(idx.search(&tok("hello"), 5).is_empty());
    }

    #[test]
    fn test_basic_search() {
        let docs = vec![
            tok("the cat sat on the mat"),
            tok("the dog played in the park"),
            tok("a cat and a dog are friends"),
        ];
        let idx = BM25Index::build(&docs);
        assert_eq!(idx.n_docs(), 3);

        let results = idx.search(&tok("cat"), 2);
        assert!(!results.is_empty());
        assert!(
            results[0].0 == 0 || results[0].0 == 2,
            "top result should be a doc mentioning 'cat'"
        );
    }

    #[test]
    fn test_no_match() {
        let docs = vec![tok("alpha beta gamma")];
        let idx = BM25Index::build(&docs);
        let results = idx.search(&tok("zzz"), 5);
        assert!(results.is_empty());
    }
}
