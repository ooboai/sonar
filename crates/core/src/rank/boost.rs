use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::tokens::split_identifier;
use crate::types::Chunk;

/// Symbol-lookup queries: namespace-qualified, leading-underscore, or containing
/// uppercase/underscore.
///
/// Ported verbatim from semble's `ranking/boosting.py::_SYMBOL_QUERY_RE`.
static SYMBOL_QUERY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:",
        r"[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\\|->|\.)[A-Za-z_][A-Za-z0-9_]*)+", // namespace-qualified
        r"|_[A-Za-z0-9_]*",                                                  // leading underscore
        r"|[A-Za-z][A-Za-z0-9]*[A-Z_][A-Za-z0-9_]*", // contains uppercase or underscore
        r"|[A-Z][A-Za-z0-9]*",                       // starts with uppercase
        r")$",
    ))
    .unwrap()
});

/// CamelCase/camelCase identifiers embedded in a NL query.
///
/// Uses `\b` word boundaries to avoid partial matches inside longer tokens.
static EMBEDDED_SYMBOL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b(?:",
        r"[A-Z][a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*", // PascalCase
        r"|[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]+",     // camelCase
        r")\b",
    ))
    .unwrap()
});

/// Token extraction for NL queries.
static WORD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").unwrap());

static STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "a", "an", "and", "are", "as", "at", "be", "by", "do", "does", "for", "from", "has",
        "have", "how", "if", "in", "is", "it", "not", "of", "on", "or", "the", "to", "was", "what",
        "when", "where", "which", "who", "why", "with",
    ]
    .into_iter()
    .collect()
});

const DEFINITION_KEYWORDS: &[&str] = &[
    "class",
    "module",
    "defmodule",
    "def",
    "interface",
    "struct",
    "enum",
    "trait",
    "type",
    "func",
    "function",
    "object",
    "abstract class",
    "data class",
    "fn",
    "fun",
    "package",
    "namespace",
    "protocol",
    "record",
    "typedef",
];

const SQL_DEFINITION_KEYWORDS: &[&str] = &[
    "CREATE TABLE",
    "CREATE VIEW",
    "CREATE PROCEDURE",
    "CREATE FUNCTION",
];

/// Pre-compiled definition-matching regex for general keywords.
/// Uses a capture group for the symbol name so the regex can be compiled once at startup.
static GENERAL_DEFINITION_RE: LazyLock<Regex> = LazyLock::new(|| {
    let keywords: String = DEFINITION_KEYWORDS
        .iter()
        .map(|k| regex::escape(k))
        .collect::<Vec<_>>()
        .join("|");
    let ns_prefix = r"(?:[A-Za-z_][A-Za-z0-9_]*(?:\.|::))*";
    let capture = r"([A-Za-z_][A-Za-z0-9_]*)";
    let terminator = r"(?:\s|[<({:\[;]|$)";
    Regex::new(&format!(
        r"(?m)(?:^|\s)(?:{})\s+{}{}{}",
        keywords, ns_prefix, capture, terminator
    ))
    .unwrap()
});

/// Pre-compiled definition-matching regex for SQL keywords (case-insensitive).
static SQL_DEFINITION_RE: LazyLock<Regex> = LazyLock::new(|| {
    let keywords: String = SQL_DEFINITION_KEYWORDS
        .iter()
        .map(|k| regex::escape(k))
        .collect::<Vec<_>>()
        .join("|");
    let ns_prefix = r"(?:[A-Za-z_][A-Za-z0-9_]*(?:\.|::))*";
    let capture = r"([A-Za-z_][A-Za-z0-9_]*)";
    let terminator = r"(?:\s|[<({:\[;]|$)";
    Regex::new(&format!(
        r"(?mi)(?:^|\s)(?:{})\s+{}{}{}",
        keywords, ns_prefix, capture, terminator
    ))
    .unwrap()
});

/// Minimum stem length for prefix-based non-candidate scan.
const EMBEDDED_STEM_MIN_LEN: usize = 4;

/// Half-strength: the symbol may be incidental to the NL query.
const EMBEDDED_SYMBOL_BOOST_SCALE: f64 = 0.5;

/// Additive boost multiplier for chunks that define a queried symbol.
pub const DEFINITION_BOOST_MULTIPLIER: f64 = 3.0;

/// Additive boost multiplier for NL queries when file stems match query words.
pub const STEM_BOOST_MULTIPLIER: f64 = 1.0;

/// Fraction of max_score added to each file's top chunk.
pub const FILE_COHERENCE_BOOST_FRAC: f64 = 0.2;

/// Return True if the query looks like a bare symbol or namespace-qualified identifier.
///
/// Ported verbatim from semble's `ranking/boosting.py::is_symbol_query`.
pub fn is_symbol_query(query: &str) -> bool {
    SYMBOL_QUERY_RE.is_match(query.trim())
}

/// Apply query-type boosts to candidate scores.
///
/// Ported verbatim from semble's `ranking/boosting.py::apply_query_boost`.
pub fn apply_query_boost(
    combined_scores: &HashMap<Chunk, f64>,
    query: &str,
    all_chunks: &[Chunk],
) -> HashMap<Chunk, f64> {
    if combined_scores.is_empty() {
        return combined_scores.clone();
    }

    let max_score = combined_scores
        .values()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let mut boosted = combined_scores.clone();

    if is_symbol_query(query) {
        boost_symbol_definitions(&mut boosted, query, max_score, all_chunks);
    } else {
        boost_stem_matches(&mut boosted, query, max_score);
        boost_embedded_symbols(&mut boosted, query, max_score, all_chunks);
    }

    boosted
}

/// Promote files with multiple high-scoring chunks by boosting their top chunk (in-place).
///
/// Ported verbatim from semble's `ranking/boosting.py::boost_multi_chunk_files`.
pub fn boost_multi_chunk_files(scores: &mut HashMap<Chunk, f64>) {
    if scores.is_empty() {
        return;
    }

    let max_score = scores.values().copied().fold(f64::NEG_INFINITY, f64::max);
    if max_score == 0.0 {
        return;
    }

    let mut file_sum: HashMap<&str, f64> = HashMap::new();
    let mut best_chunk: HashMap<&str, &Chunk> = HashMap::new();

    for (chunk, &score) in scores.iter() {
        let fp = chunk.file_path.as_str();
        *file_sum.entry(fp).or_insert(0.0) += score;
        let is_better = best_chunk.get(fp).is_none_or(|prev| score > scores[*prev]);
        if is_better {
            best_chunk.insert(fp, chunk);
        }
    }

    let max_file_sum = file_sum.values().copied().fold(f64::NEG_INFINITY, f64::max);
    let boost_unit = max_score * FILE_COHERENCE_BOOST_FRAC;

    let boosts: Vec<(Chunk, f64)> = best_chunk
        .iter()
        .map(|(&fp, &chunk)| {
            let boost = boost_unit * file_sum[fp] / max_file_sum;
            (chunk.clone(), boost)
        })
        .collect();

    for (chunk, boost) in boosts {
        if let Some(score) = scores.get_mut(&chunk) {
            *score += boost;
        }
    }
}

/// Extract the final identifier from a possibly namespace-qualified query.
///
/// Examples: "Sinatra::Base" -> "Base", "Client" -> "Client".
fn extract_symbol_name(query: &str) -> &str {
    for separator in &["::", "\\", "->", "."] {
        if let Some(pos) = query.rfind(separator) {
            return &query[pos + separator.len()..];
        }
    }
    query.trim()
}

/// Return true if the chunk contains a definition of `symbol_name`.
///
/// Uses pre-compiled `GENERAL_DEFINITION_RE` / `SQL_DEFINITION_RE` with a
/// capture group for the symbol name, avoiding per-call regex compilation.
fn chunk_defines_symbol(chunk: &Chunk, symbol_name: &str) -> bool {
    for caps in GENERAL_DEFINITION_RE.captures_iter(&chunk.content) {
        if caps.get(1).map(|m| m.as_str()) == Some(symbol_name) {
            return true;
        }
    }
    // SQL regex is case-insensitive — mirror that for the captured name.
    for caps in SQL_DEFINITION_RE.captures_iter(&chunk.content) {
        if let Some(m) = caps.get(1) {
            if m.as_str().eq_ignore_ascii_case(symbol_name) {
                return true;
            }
        }
    }
    false
}

/// Return true if `stem` matches `name` (exact, snake_case-normalised, or plural).
fn stem_matches(stem: &str, name: &str) -> bool {
    let stem_norm = stem.replace('_', "");
    stem == name
        || stem_norm == name
        || stem.trim_end_matches('s') == name
        || stem_norm.trim_end_matches('s') == name
}

/// Return the boost amount for a chunk that defines one of `names` (0.0 if none match).
fn definition_tier(chunk: &Chunk, names: &HashSet<&str>, boost_unit: f64) -> f64 {
    if !names.iter().any(|name| chunk_defines_symbol(chunk, name)) {
        return 0.0;
    }
    let stem = Path::new(&chunk.file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if names
        .iter()
        .any(|name| stem_matches(&stem, &name.to_lowercase()))
    {
        boost_unit * 1.5
    } else {
        boost_unit
    }
}

/// Boost non-candidate chunks whose lowercased file stem satisfies `stem_ok` (in-place).
fn scan_non_candidates(
    boosted: &mut HashMap<Chunk, f64>,
    names: &HashSet<&str>,
    boost_unit: f64,
    all_chunks: &[Chunk],
    stem_ok: impl Fn(&str) -> bool,
) {
    let mut to_insert: Vec<(Chunk, f64)> = Vec::new();
    for chunk in all_chunks {
        if boosted.contains_key(chunk) {
            continue;
        }
        let stem = Path::new(&chunk.file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !stem_ok(&stem) {
            continue;
        }
        let tier = definition_tier(chunk, names, boost_unit);
        if tier > 0.0 {
            to_insert.push((chunk.clone(), tier));
        }
    }
    for (chunk, tier) in to_insert {
        boosted.insert(chunk, tier);
    }
}

/// Boost chunks that define the queried symbol (in-place).
fn boost_symbol_definitions(
    boosted: &mut HashMap<Chunk, f64>,
    query: &str,
    max_score: f64,
    all_chunks: &[Chunk],
) {
    let symbol_name = extract_symbol_name(query);
    let mut names: HashSet<&str> = HashSet::new();
    names.insert(symbol_name);
    let trimmed = query.trim();
    if symbol_name != trimmed {
        names.insert(trimmed);
    }

    let boost_unit = max_score * DEFINITION_BOOST_MULTIPLIER;

    let candidates: Vec<Chunk> = boosted.keys().cloned().collect();
    for chunk in &candidates {
        let tier = definition_tier(chunk, &names, boost_unit);
        if tier > 0.0 {
            *boosted.get_mut(chunk).unwrap() += tier;
        }
    }

    let symbol_lower = symbol_name.to_lowercase();
    scan_non_candidates(boosted, &names, boost_unit, all_chunks, |stem| {
        stem_matches(stem, &symbol_lower)
    });
}

/// Boost chunks whose file paths match NL query keywords (in-place).
fn boost_stem_matches(boosted: &mut HashMap<Chunk, f64>, query: &str, max_score: f64) {
    let keywords: HashSet<String> = WORD_RE
        .find_iter(query)
        .map(|m| m.as_str().to_lowercase())
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w.as_str()))
        .collect();

    if keywords.is_empty() {
        return;
    }

    let boost = max_score * STEM_BOOST_MULTIPLIER;
    let mut path_cache: HashMap<String, HashSet<String>> = HashMap::new();

    let candidates: Vec<Chunk> = boosted.keys().cloned().collect();
    for chunk in &candidates {
        let parts = path_cache
            .entry(chunk.file_path.clone())
            .or_insert_with(|| {
                let path = Path::new(&chunk.file_path);
                let mut parts: HashSet<String> = HashSet::new();
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    for p in split_identifier(stem) {
                        parts.insert(p);
                    }
                }
                if let Some(parent) = path.parent().and_then(|p| p.file_name())
                    && let Some(parent_name) = parent.to_str()
                    && parent_name != "."
                    && parent_name != "/"
                    && parent_name != ".."
                {
                    for p in split_identifier(parent_name) {
                        parts.insert(p);
                    }
                }
                parts
            })
            .clone();

        let n_matches = count_keyword_matches(&keywords, &parts);
        if n_matches > 0 {
            let match_ratio = n_matches as f64 / keywords.len() as f64;
            if match_ratio >= 0.10 {
                *boosted.get_mut(chunk).unwrap() += boost * match_ratio;
            }
        }
    }
}

/// Boost chunks defining CamelCase/camelCase symbols embedded in NL queries (in-place).
fn boost_embedded_symbols(
    boosted: &mut HashMap<Chunk, f64>,
    query: &str,
    max_score: f64,
    all_chunks: &[Chunk],
) {
    let names: HashSet<&str> = EMBEDDED_SYMBOL_RE
        .find_iter(query)
        .map(|m| m.as_str())
        .collect();

    if names.is_empty() {
        return;
    }

    let boost_unit = max_score * DEFINITION_BOOST_MULTIPLIER * EMBEDDED_SYMBOL_BOOST_SCALE;

    let candidates: Vec<Chunk> = boosted.keys().cloned().collect();
    for chunk in &candidates {
        let tier = definition_tier(chunk, &names, boost_unit);
        if tier > 0.0 {
            *boosted.get_mut(chunk).unwrap() += tier;
        }
    }

    let symbols_lower: HashSet<String> = names.iter().map(|s| s.to_lowercase()).collect();

    let mut to_insert: Vec<(Chunk, f64)> = Vec::new();
    for chunk in all_chunks {
        if boosted.contains_key(chunk) {
            continue;
        }
        let stem = Path::new(&chunk.file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let stem_norm = stem.replace('_', "");

        let matches = symbols_lower.iter().any(|symbol_lower| {
            stem == *symbol_lower
                || stem_norm == *symbol_lower
                || (stem.len() >= EMBEDDED_STEM_MIN_LEN && symbol_lower.starts_with(&stem))
                || (stem_norm.len() >= EMBEDDED_STEM_MIN_LEN
                    && symbol_lower.starts_with(&stem_norm))
        });

        if !matches {
            continue;
        }

        let tier = definition_tier(chunk, &names, boost_unit);
        if tier > 0.0 {
            to_insert.push((chunk.clone(), tier));
        }
    }

    for (chunk, tier) in to_insert {
        boosted.insert(chunk, tier);
    }
}

/// Count query keywords that match path parts, allowing prefix overlap (min 3 chars).
fn count_keyword_matches(keywords: &HashSet<String>, parts: &HashSet<String>) -> usize {
    let exact: HashSet<&String> = keywords.intersection(parts).collect();
    if exact.len() == keywords.len() {
        return exact.len();
    }

    let mut n_matches = exact.len();
    for keyword in keywords {
        if exact.contains(keyword) {
            continue;
        }
        for part in parts {
            let (shorter, longer) = if keyword.len() <= part.len() {
                (keyword.as_str(), part.as_str())
            } else {
                (part.as_str(), keyword.as_str())
            };
            if shorter.len() >= 3 && longer.starts_with(shorter) {
                n_matches += 1;
                break;
            }
        }
    }
    n_matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(content: &str, file_path: &str) -> Chunk {
        Chunk {
            content: content.to_string(),
            file_path: file_path.to_string(),
            start_line: 1,
            end_line: 10,
            language: Some("rust".to_string()),
        }
    }

    #[test]
    fn test_symbol_queries() {
        assert!(is_symbol_query("HandlerStack"));
        assert!(is_symbol_query("Sinatra::Base"));
        assert!(is_symbol_query("_private_var"));
        assert!(is_symbol_query("my_func"));
        assert!(!is_symbol_query("how does auth work"));
        assert!(!is_symbol_query("session"));
    }

    #[test]
    fn test_extract_symbol_name() {
        assert_eq!(extract_symbol_name("Sinatra::Base"), "Base");
        assert_eq!(extract_symbol_name("Client"), "Client");
        assert_eq!(extract_symbol_name("foo.bar.Baz"), "Baz");
        assert_eq!(extract_symbol_name("Mod->Thing"), "Thing");
        assert_eq!(extract_symbol_name("Ns\\Class"), "Class");
    }

    #[test]
    fn test_chunk_defines_symbol() {
        let chunk = make_chunk("class HandlerStack:\n    pass", "src/handler.py");
        assert!(chunk_defines_symbol(&chunk, "HandlerStack"));

        let chunk2 = make_chunk("fn something() {}", "src/lib.rs");
        assert!(!chunk_defines_symbol(&chunk2, "HandlerStack"));
    }

    #[test]
    fn test_chunk_defines_symbol_sql() {
        let chunk = make_chunk("CREATE TABLE users (\n  id INT\n)", "schema.sql");
        assert!(chunk_defines_symbol(&chunk, "users"));
    }

    #[test]
    fn test_chunk_defines_namespaced_symbol() {
        let chunk = make_chunk("defmodule Phoenix.Router do\nend", "lib/router.ex");
        assert!(chunk_defines_symbol(&chunk, "Router"));
    }

    #[test]
    fn test_stem_matches_exact() {
        assert!(stem_matches("handler", "handler"));
    }

    #[test]
    fn test_stem_matches_normalized() {
        assert!(stem_matches("handler_stack", "handlerstack"));
    }

    #[test]
    fn test_stem_matches_plural() {
        assert!(stem_matches("handlers", "handler"));
    }

    #[test]
    fn test_stem_matches_no_match() {
        assert!(!stem_matches("utils", "handler"));
    }

    #[test]
    fn test_definition_tier_with_match() {
        let chunk = make_chunk("class Router:\n    pass", "src/router.py");
        let names: HashSet<&str> = ["Router"].into_iter().collect();
        let tier = definition_tier(&chunk, &names, 1.0);
        assert_eq!(tier, 1.5); // stem "router" matches "Router" lowercased
    }

    #[test]
    fn test_definition_tier_no_stem_match() {
        let chunk = make_chunk("class Router:\n    pass", "src/utils.py");
        let names: HashSet<&str> = ["Router"].into_iter().collect();
        let tier = definition_tier(&chunk, &names, 1.0);
        assert_eq!(tier, 1.0); // defines it but stem doesn't match
    }

    #[test]
    fn test_definition_tier_no_definition() {
        let chunk = make_chunk("x = Router()", "src/main.py");
        let names: HashSet<&str> = ["Router"].into_iter().collect();
        let tier = definition_tier(&chunk, &names, 1.0);
        assert_eq!(tier, 0.0);
    }

    #[test]
    fn test_boost_multi_chunk_files() {
        let c1 = make_chunk("fn a() {}", "src/lib.rs");
        let c2 = make_chunk("fn b() {}", "src/lib.rs");
        let c3 = make_chunk("fn c() {}", "src/other.rs");

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(c1.clone(), 0.8);
        scores.insert(c2.clone(), 0.6);
        scores.insert(c3.clone(), 0.5);

        boost_multi_chunk_files(&mut scores);

        // c1 is the best chunk in src/lib.rs which has the highest file_sum (1.4)
        // c3 is the best chunk in src/other.rs
        // Both should get boosted, but c1 more since its file has higher aggregate
        assert!(scores[&c1] > 0.8);
        assert!(scores[&c3] > 0.5);
    }

    #[test]
    fn test_boost_multi_chunk_files_empty() {
        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        boost_multi_chunk_files(&mut scores);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_apply_query_boost_symbol() {
        let chunk = make_chunk("class HandlerStack:\n    pass", "src/handler_stack.py");
        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(chunk.clone(), 1.0);

        let boosted = apply_query_boost(&scores, "HandlerStack", &[chunk]);
        assert!(boosted.values().next().unwrap() > &1.0);
    }

    #[test]
    fn test_apply_query_boost_nl() {
        let chunk = make_chunk("def authenticate():\n    pass", "src/auth.py");
        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(chunk.clone(), 1.0);

        let boosted = apply_query_boost(&scores, "how does authentication work", &[chunk]);
        // "auth" is the stem, "authentication" is a keyword - prefix match should boost
        assert!(boosted.values().next().unwrap() > &1.0);
    }

    #[test]
    fn test_apply_query_boost_empty() {
        let scores: HashMap<Chunk, f64> = HashMap::new();
        let boosted = apply_query_boost(&scores, "anything", &[]);
        assert!(boosted.is_empty());
    }

    #[test]
    fn test_count_keyword_matches_exact() {
        let keywords: HashSet<String> = ["auth", "handler"].iter().map(|s| s.to_string()).collect();
        let parts: HashSet<String> = ["auth", "handler"].iter().map(|s| s.to_string()).collect();
        assert_eq!(count_keyword_matches(&keywords, &parts), 2);
    }

    #[test]
    fn test_count_keyword_matches_prefix() {
        let keywords: HashSet<String> = ["authentication"].iter().map(|s| s.to_string()).collect();
        let parts: HashSet<String> = ["auth"].iter().map(|s| s.to_string()).collect();
        // shorter="auth" (4 >= 3), longer="authentication".starts_with("auth") -> match
        assert_eq!(count_keyword_matches(&keywords, &parts), 1);
    }

    #[test]
    fn test_count_keyword_matches_no_match() {
        let keywords: HashSet<String> = ["router"].iter().map(|s| s.to_string()).collect();
        let parts: HashSet<String> = ["auth", "handler"].iter().map(|s| s.to_string()).collect();
        assert_eq!(count_keyword_matches(&keywords, &parts), 0);
    }

    #[test]
    fn test_count_keyword_matches_short_prefix_rejected() {
        let keywords: HashSet<String> = ["ab"].iter().map(|s| s.to_string()).collect();
        let parts: HashSet<String> = ["abcdef"].iter().map(|s| s.to_string()).collect();
        // "ab" is too short (< 3) for prefix match
        assert_eq!(count_keyword_matches(&keywords, &parts), 0);
    }

    #[test]
    fn test_boost_symbol_definitions_non_candidate() {
        let candidate = make_chunk("x = Router()", "src/main.py");
        let non_candidate = make_chunk("class Router:\n    pass", "src/router.py");

        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(candidate.clone(), 1.0);

        let all_chunks = vec![candidate, non_candidate.clone()];
        let boosted = apply_query_boost(&scores, "Router", &all_chunks);

        // non_candidate should be pulled in because its stem matches and it defines Router
        assert!(boosted.contains_key(&non_candidate));
        assert!(boosted[&non_candidate] > 0.0);
    }

    #[test]
    fn test_boost_embedded_symbols() {
        let chunk = make_chunk("class StateManager:\n    pass", "src/state.py");
        let mut scores: HashMap<Chunk, f64> = HashMap::new();
        scores.insert(chunk.clone(), 1.0);

        let boosted =
            apply_query_boost(&scores, "how does the StateManager handle events", &[chunk]);
        assert!(boosted.values().next().unwrap() > &1.0);
    }

    #[test]
    fn test_scan_non_candidates_pulls_in_definitions() {
        let non_candidate = make_chunk("class Foo:\n    pass", "src/foo.py");
        let mut boosted: HashMap<Chunk, f64> = HashMap::new();

        let names: HashSet<&str> = ["Foo"].into_iter().collect();
        let all_chunks = vec![non_candidate.clone()];

        scan_non_candidates(&mut boosted, &names, 3.0, &all_chunks, |stem| stem == "foo");

        assert!(boosted.contains_key(&non_candidate));
        assert!(boosted[&non_candidate] > 0.0);
    }
}
