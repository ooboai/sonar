use regex::Regex;
use std::sync::LazyLock;

static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").unwrap());

/// Split camelCase/PascalCase into parts without regex look-ahead.
/// Equivalent to Python's `[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|[0-9]+`.
fn split_camel(token: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let chars: Vec<char> = token.chars().collect();
    let mut start = 0;

    while start < chars.len() {
        if chars[start].is_ascii_digit() {
            let end = chars[start..]
                .iter()
                .position(|c| !c.is_ascii_digit())
                .map_or(chars.len(), |p| start + p);
            parts.push(chars[start..end].iter().collect::<String>());
            start = end;
        } else if chars[start].is_ascii_uppercase() {
            let upper_end = chars[start..]
                .iter()
                .position(|c| !c.is_ascii_uppercase())
                .map_or(chars.len(), |p| start + p);
            if upper_end - start > 1
                && upper_end < chars.len()
                && chars[upper_end].is_ascii_lowercase()
            {
                parts.push(chars[start..upper_end - 1].iter().collect::<String>());
                start = upper_end - 1;
            } else if upper_end < chars.len() && chars[upper_end].is_ascii_lowercase() {
                let lower_end = chars[upper_end..]
                    .iter()
                    .position(|c| !c.is_ascii_lowercase())
                    .map_or(chars.len(), |p| upper_end + p);
                parts.push(chars[start..lower_end].iter().collect::<String>());
                start = lower_end;
            } else {
                parts.push(chars[start..upper_end].iter().collect::<String>());
                start = upper_end;
            }
        } else if chars[start].is_ascii_lowercase() {
            let end = chars[start..]
                .iter()
                .position(|c| !c.is_ascii_lowercase())
                .map_or(chars.len(), |p| start + p);
            parts.push(chars[start..end].iter().collect::<String>());
            start = end;
        } else {
            start += 1;
        }
    }

    parts
}

/// Split a single identifier into sub-tokens via camelCase/snake_case.
///
/// Ported from semble's `tokens.py::split_identifier`.
///
/// Examples:
///   "HandlerStack" -> ["handlerstack", "handler", "stack"]
///   "my_func"      -> ["my_func", "my", "func"]
///   "simple"       -> ["simple"]
pub fn split_identifier(token: &str) -> Vec<String> {
    let lower = token.to_lowercase();

    let parts: Vec<String> = if token.contains('_') {
        lower
            .split('_')
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect()
    } else {
        split_camel(token)
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect()
    };

    if parts.len() >= 2 {
        let mut result = vec![lower];
        result.extend(parts);
        result
    } else {
        vec![lower]
    }
}

/// Split text into lowercase identifier-like tokens for BM25 indexing.
///
/// Ported verbatim from semble's `tokens.py::tokenize`.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    for m in TOKEN_RE.find_iter(text) {
        result.extend(split_identifier(m.as_str()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_camel_case() {
        assert_eq!(
            split_identifier("HandlerStack"),
            vec!["handlerstack", "handler", "stack"]
        );
    }

    #[test]
    fn test_split_snake_case() {
        assert_eq!(split_identifier("my_func"), vec!["my_func", "my", "func"]);
    }

    #[test]
    fn test_split_simple() {
        assert_eq!(split_identifier("simple"), vec!["simple"]);
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("getHTTPResponse from server");
        assert!(tokens.contains(&"gethttpresponse".to_string()));
        assert!(tokens.contains(&"get".to_string()));
        assert!(tokens.contains(&"from".to_string()));
        assert!(tokens.contains(&"server".to_string()));
    }
}
