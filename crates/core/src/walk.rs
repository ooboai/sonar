use std::path::Path;

use walkdir::WalkDir;

use crate::types::ContentType;

const MAX_FILE_BYTES: u64 = 1_000_000;

const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "__pycache__",
    ".tox",
    "target",
    "build",
    "dist",
    ".cache",
    ".eggs",
    "venv",
    ".venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
];

pub struct WalkedFile {
    pub relative_path: String,
    pub content: String,
    pub language: Option<String>,
    pub content_type: ContentType,
}

/// Detect programming language from file extension.
pub fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    let lang = match ext {
        "py" => "python",
        "rs" => "rust",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "php" => "php",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "scala" => "scala",
        "dart" => "dart",
        "lua" => "lua",
        "sh" | "bash" | "zsh" => "bash",
        "r" | "R" => "r",
        "sql" => "sql",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "hs" => "haskell",
        "ml" | "mli" => "ocaml",
        _ => return None,
    };
    Some(lang.to_string())
}

/// Detect content type from file extension.
pub fn detect_content_type(path: &Path) -> ContentType {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "md" | "mdx" | "rst" | "txt" | "adoc" => ContentType::Docs,
        "toml" | "yaml" | "yml" | "json" | "xml" | "ini" | "cfg" | "conf" | "env"
        | "properties" => ContentType::Config,
        _ => ContentType::Code,
    }
}

/// Walk a directory tree and return indexable files.
///
/// Skips hidden directories, common build/cache directories, and files
/// larger than 1 MB. Returns files with relative paths, content, detected
/// language, and content type.
pub fn walk_directory(root: &Path) -> Vec<WalkedFile> {
    let mut files = Vec::new();

    let walker = WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_str().unwrap_or("");
        if e.file_type().is_dir() {
            let is_hidden = name.starts_with('.') && name != ".";
            !is_hidden && !IGNORED_DIRS.contains(&name)
        } else {
            true
        }
    });

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        if let Ok(meta) = path.metadata()
            && meta.len() > MAX_FILE_BYTES
        {
            continue;
        }

        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_str = relative.to_string_lossy().to_string();

        let content_type = detect_content_type(path);
        let language = match content_type {
            ContentType::Docs => Some("markdown".to_string()),
            ContentType::Code => detect_language(path),
            ContentType::Config => None,
        };

        match content_type {
            ContentType::Code if language.is_none() => continue,
            ContentType::Config => continue,
            _ => {}
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if content.trim().is_empty() {
            continue;
        }

        files.push(WalkedFile {
            relative_path: relative_str,
            content,
            language,
            content_type,
        });
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_detect_language() {
        assert_eq!(
            detect_language(Path::new("foo.py")),
            Some("python".to_string())
        );
        assert_eq!(
            detect_language(Path::new("bar.rs")),
            Some("rust".to_string())
        );
        assert_eq!(
            detect_language(Path::new("baz.ts")),
            Some("typescript".to_string())
        );
        assert_eq!(detect_language(Path::new("no_ext")), None);
    }

    #[test]
    fn test_detect_content_type() {
        assert_eq!(
            detect_content_type(Path::new("README.md")),
            ContentType::Docs
        );
        assert_eq!(
            detect_content_type(Path::new("config.toml")),
            ContentType::Config
        );
        assert_eq!(detect_content_type(Path::new("main.rs")), ContentType::Code);
    }

    #[test]
    fn test_walk_directory_on_self() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let files = walk_directory(&manifest_dir);
        assert!(
            !files.is_empty(),
            "should find at least the crate's own .rs files"
        );
        assert!(
            files.iter().any(|f| f.relative_path.ends_with(".rs")),
            "should contain .rs files"
        );
    }
}
