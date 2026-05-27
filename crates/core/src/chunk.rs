use tree_sitter::{Language, Parser};

use crate::types::Chunk;

/// Desired chunk length in characters.
/// Ported from semble's `chunking.py::_DESIRED_CHUNK_LENGTH_CHARS`.
const DESIRED_CHUNK_LENGTH_CHARS: usize = 1500;

/// Max recursion depth when splitting large tree-sitter nodes.
const RECURSION_DEPTH: usize = 500;

/// Nodes smaller than this (bytes) are never recursed into.
const MIN_CHUNK_SIZE: usize = 50;

/// Chunk source code using tree-sitter when available, falling back to line-based chunking.
///
/// Ported from semble's `chunking.py::chunk_source`.
pub fn chunk_source(source: &str, file_path: &str, language: Option<&str>) -> Vec<Chunk> {
    if source.trim().is_empty() {
        return Vec::new();
    }

    let boundaries = match language {
        Some("markdown") => Some(chunk_markdown(source, DESIRED_CHUNK_LENGTH_CHARS)),
        Some(lang) => chunk_tree_sitter(source, lang, DESIRED_CHUNK_LENGTH_CHARS),
        None => None,
    };

    let boundaries = boundaries.unwrap_or_else(|| chunk_lines(source, DESIRED_CHUNK_LENGTH_CHARS));

    boundaries
        .into_iter()
        .filter_map(|b| {
            let start = floor_char_boundary(source, b.start);
            let end = ceil_char_boundary(source, b.end.min(source.len()));
            if start >= end {
                return None;
            }
            let text = &source[start..end];
            let start_line = source[..start].matches('\n').count() + 1;
            let end_line = source[..end].matches('\n').count() + 1;
            Some(Chunk {
                content: text.to_string(),
                file_path: file_path.to_string(),
                start_line,
                end_line,
                language: language.map(String::from),
            })
        })
        .collect()
}

/// Round down to the nearest char boundary at or before `index`.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round up to the nearest char boundary at or after `index`.
fn ceil_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

struct Boundary {
    start: usize,
    end: usize,
}

// ---------------------------------------------------------------------------
// Tree-sitter chunking — ported from semble's chunking/core.py
// ---------------------------------------------------------------------------

fn get_ts_language(lang: &str) -> Option<Language> {
    match lang {
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "javascript" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        _ => None,
    }
}

/// Merge adjacent boundary chunks up to `desired_len`.
///
/// Ported from semble's `core.py::_merge_adjacent_chunks`.
fn merge_adjacent(chunks: &[Boundary], desired_len: usize) -> Vec<Boundary> {
    if chunks.is_empty() {
        return Vec::new();
    }

    let mut merged = Vec::new();
    let mut cur_start = chunks[0].start;
    let mut cur_end = chunks[0].end;
    let mut cur_len = cur_end - cur_start;

    for b in &chunks[1..] {
        let len = b.end - b.start;
        if cur_len + len > desired_len {
            merged.push(Boundary {
                start: cur_start,
                end: cur_end,
            });
            cur_start = b.start;
            cur_end = b.end;
            cur_len = len;
        } else {
            cur_end = b.end;
            cur_len += len;
        }
    }
    merged.push(Boundary {
        start: cur_start,
        end: cur_end,
    });

    merged
}

/// Recursively split and merge tree-sitter nodes into boundary chunks.
///
/// Ported from semble's `core.py::_merge_node_inner`.
fn merge_node_inner(node: &tree_sitter::Node, desired_len: usize, depth: usize) -> Vec<Boundary> {
    let child_count = node.child_count();
    if child_count == 0 {
        return vec![Boundary {
            start: node.start_byte(),
            end: node.end_byte(),
        }];
    }

    let length = node.end_byte() - node.start_byte();
    if depth > RECURSION_DEPTH || length < MIN_CHUNK_SIZE {
        return vec![Boundary {
            start: node.start_byte(),
            end: node.end_byte(),
        }];
    }

    let mut groups = Vec::new();
    let mut index = 0;

    while index < child_count {
        let child = match node.child(index) {
            Some(c) => c,
            None => {
                index += 1;
                continue;
            }
        };
        let start = child.start_byte();
        let mut end = child.end_byte();
        let mut len = end - start;
        index += 1;

        if len > desired_len {
            groups.extend(merge_node_inner(&child, desired_len, depth + 1));
            continue;
        }

        while index < child_count {
            let next = match node.child(index) {
                Some(c) => c,
                None => break,
            };
            let next_len = next.end_byte() - next.start_byte();
            if len + next_len > desired_len {
                break;
            }
            end = next.end_byte();
            len += next_len;
            index += 1;
        }

        groups.push(Boundary { start, end });
    }

    groups
}

/// Turn a tree-sitter root into merged boundary chunks.
///
/// Ported from semble's `core.py::_merge_node`.
fn merge_node(node: &tree_sitter::Node, desired_len: usize) -> Vec<Boundary> {
    let raw = merge_node_inner(node, desired_len, 0);
    merge_adjacent(&raw, desired_len)
}

/// Parse `source` with tree-sitter for `language` and return chunk boundaries.
///
/// Returns `None` when the language is not supported or parsing fails,
/// signalling the caller to fall back to line-based chunking.
fn chunk_tree_sitter(source: &str, language: &str, desired_len: usize) -> Option<Vec<Boundary>> {
    let ts_lang = get_ts_language(language)?;
    let mut parser = Parser::new();
    parser.set_language(&ts_lang).ok()?;
    let tree = parser.parse(source.as_bytes(), None)?;
    Some(merge_node(&tree.root_node(), desired_len))
}

// ---------------------------------------------------------------------------
// Markdown chunking — split on heading boundaries
// ---------------------------------------------------------------------------

/// Split markdown on heading boundaries (`# …`), then merge small sections.
fn chunk_markdown(source: &str, desired_len: usize) -> Vec<Boundary> {
    let mut raw = Vec::new();
    let mut section_start: usize = 0;
    let mut pos: usize = 0;

    for line in source.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();

        if line.starts_with('#') && line_start > section_start {
            raw.push(Boundary {
                start: section_start,
                end: line_start,
            });
            section_start = line_start;
        }
    }

    if section_start < source.len() {
        raw.push(Boundary {
            start: section_start,
            end: source.len(),
        });
    }

    if raw.is_empty() {
        return chunk_lines(source, desired_len);
    }

    merge_adjacent(&raw, desired_len)
}

// ---------------------------------------------------------------------------
// Line-based fallback chunker (unchanged from original)
// ---------------------------------------------------------------------------

/// Splits source into chunks of approximately `desired_len` characters
/// at line boundaries.
fn chunk_lines(source: &str, desired_len: usize) -> Vec<Boundary> {
    let mut boundaries = Vec::new();
    let mut start = 0;
    let mut current_len = 0;

    for (i, c) in source.char_indices() {
        current_len += c.len_utf8();
        if c == '\n' && current_len >= desired_len {
            boundaries.push(Boundary { start, end: i + 1 });
            start = i + 1;
            current_len = 0;
        }
    }

    if start < source.len() {
        boundaries.push(Boundary {
            start,
            end: source.len(),
        });
    }

    boundaries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_python_function() {
        let source = r#"
def hello(name):
    """Say hello."""
    print(f"Hello, {name}!")

def goodbye(name):
    """Say goodbye."""
    print(f"Goodbye, {name}!")
"#;
        let chunks = chunk_source(source, "example.py", Some("python"));
        assert!(!chunks.is_empty());
        assert!(chunks[0].content.contains("def hello"));
        assert_eq!(chunks[0].language.as_deref(), Some("python"));
    }

    #[test]
    fn test_chunk_rust_struct() {
        let source = r#"
pub struct Config {
    pub name: String,
    pub value: i32,
}

impl Config {
    pub fn new(name: &str, value: i32) -> Self {
        Config {
            name: name.to_string(),
            value,
        }
    }
}
"#;
        let chunks = chunk_source(source, "config.rs", Some("rust"));
        assert!(!chunks.is_empty());
        assert!(chunks[0].content.contains("struct Config"));
        assert_eq!(chunks[0].language.as_deref(), Some("rust"));
    }

    #[test]
    fn test_chunk_unsupported_language_falls_back_to_lines() {
        let source = "line1\nline2\nline3\n";
        let chunks = chunk_source(source, "file.xyz", Some("brainfuck"));
        assert!(!chunks.is_empty());
        assert!(chunks[0].content.contains("line1"));
    }

    #[test]
    fn test_chunk_no_language_falls_back_to_lines() {
        let source = "line1\nline2\nline3\n";
        let chunks = chunk_source(source, "file.txt", None);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_chunk_empty_source() {
        let chunks = chunk_source("   ", "empty.py", Some("python"));
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_markdown() {
        let source = "# Heading 1\nSome text.\n## Heading 2\nMore text.\n";
        let chunks = chunk_source(source, "README.md", Some("markdown"));
        assert!(!chunks.is_empty());
        assert!(chunks[0].content.contains("Heading 1"));
    }

    #[test]
    fn test_merge_adjacent_respects_limit() {
        let boundaries = vec![
            Boundary { start: 0, end: 100 },
            Boundary {
                start: 100,
                end: 200,
            },
            Boundary {
                start: 200,
                end: 300,
            },
        ];
        let merged = merge_adjacent(&boundaries, 250);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start, 0);
        assert_eq!(merged[0].end, 200);
        assert_eq!(merged[1].start, 200);
        assert_eq!(merged[1].end, 300);
    }

    #[test]
    fn test_tree_sitter_returns_none_for_unknown_lang() {
        assert!(get_ts_language("cobol").is_none());
    }

    #[test]
    fn test_all_supported_languages_parse() {
        for lang in &[
            "python",
            "rust",
            "javascript",
            "typescript",
            "tsx",
            "go",
            "java",
        ] {
            let ts = get_ts_language(lang);
            assert!(ts.is_some(), "get_ts_language failed for {lang}");
        }
    }

    #[test]
    fn test_chunk_multibyte_utf8_no_panic() {
        let source = "use std::fs;\n\n/// Platform─specific box─drawing ├── and └── chars.\npub fn hello() {\n    println!(\"héllo wörld 日本語\");\n}\n\n// More code after unicode: αβγδ εζηθ\nfn other() {}\n";
        let chunks = chunk_source(source, "utf8.rs", Some("rust"));
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(chunk.content.len() > 0);
            assert!(chunk.content.is_char_boundary(0));
        }
    }

    #[test]
    fn test_chunk_box_drawing_chars() {
        let source = "/// ┌───────────┐\n/// │  diagram  │\n/// └───────────┘\npub fn render() {\n    let s = \"├── child\";\n    println!(\"{}\", s);\n}\n";
        let chunks = chunk_source(source, "draw.rs", Some("rust"));
        assert!(!chunks.is_empty());
        assert!(chunks[0].content.contains("diagram"));
    }
}
