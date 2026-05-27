use crate::rank::boost::is_symbol_query;

/// Lean BM25 for exact keyword matching.
/// Ported from semble's `ranking/weighting.py::_ALPHA_SYMBOL`.
const ALPHA_SYMBOL: f64 = 0.3;

/// Balanced semantic + BM25.
/// Ported from semble's `ranking/weighting.py::_ALPHA_NL`.
const ALPHA_NL: f64 = 0.5;

/// Return the blending weight for semantic scores, auto-detecting from query type.
///
/// Ported verbatim from semble's `ranking/weighting.py::resolve_alpha`.
pub fn resolve_alpha(query: &str, alpha: Option<f64>) -> f64 {
    match alpha {
        Some(a) => a,
        None => {
            if is_symbol_query(query) {
                ALPHA_SYMBOL
            } else {
                ALPHA_NL
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explicit_alpha_wins() {
        assert!((resolve_alpha("anything", Some(0.8)) - 0.8).abs() < 1e-10);
        assert!((resolve_alpha("anything", Some(0.0)) - 0.0).abs() < 1e-10);
        assert!((resolve_alpha("anything", Some(1.0)) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_symbol_query_gets_low_alpha() {
        assert!((resolve_alpha("parseConfig", None) - ALPHA_SYMBOL).abs() < 1e-10);
        assert!((resolve_alpha("get_user_by_id", None) - ALPHA_SYMBOL).abs() < 1e-10);
        assert!((resolve_alpha("BM25Index", None) - ALPHA_SYMBOL).abs() < 1e-10);
    }

    #[test]
    fn test_nl_query_gets_balanced_alpha() {
        assert!((resolve_alpha("how does auth work", None) - ALPHA_NL).abs() < 1e-10);
        assert!((resolve_alpha("find error handling", None) - ALPHA_NL).abs() < 1e-10);
    }

    #[test]
    fn test_alpha_constants() {
        assert!(ALPHA_SYMBOL < ALPHA_NL, "symbol queries should lean toward BM25");
        assert!(ALPHA_NL <= 1.0);
        assert!(ALPHA_SYMBOL >= 0.0);
    }
}
