use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn sonar_bin() -> std::path::PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // deps/
    path.pop(); // debug/
    path.push("sonar");
    if !path.exists() {
        path.pop();
        path.push("deps");
        path.push("sonar");
    }
    path
}

fn create_sample_project(dir: &Path) {
    // Rust file with box-drawing multibyte chars (the exact case that caused the panic)
    fs::write(
        dir.join("support.rs"),
        r#"use std::fs;
use std::path::{Path, PathBuf};

/// Platform-specific application support directory.
/// ┌───────────────────────────────────────────────┐
/// │ This function handles OS─specific differences │
/// └───────────────────────────────────────────────┘
pub fn support_dir(app_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join(format!("Library/Application Support/{}", app_name)))
    }
    #[cfg(target_os = "linux")]
    {
        dirs::data_dir().map(|d| d.join(app_name))
    }
}

/// Helper with tree─drawing characters: ├── └── │
fn print_tree(entries: &[&str]) {
    for (i, entry) in entries.iter().enumerate() {
        if i == entries.len() - 1 {
            println!("└── {entry}");
        } else {
            println!("├── {entry}");
        }
    }
}
"#,
    )
    .unwrap();

    // Python file with unicode in strings
    fs::write(
        dir.join("unicode_heavy.py"),
        r#"#!/usr/bin/env python3
"""Module with heavy unicode usage — dashes, CJK, emoji."""

DIVIDER = "─" * 80

def greet(name: str) -> str:
    """Greet with Japanese: こんにちは世界"""
    return f"Hello {name} 🌍"

# Table header
# ┌──────┬──────┐
# │ col1 │ col2 │
# └──────┴──────┘
def format_table(rows):
    header = "│ " + " │ ".join(rows[0]) + " │"
    separator = "├" + "─" * 6 + "┼" + "─" * 6 + "┤"
    return f"{header}\n{separator}"

class DataProcessor:
    """Processes data with special chars: α β γ δ ε ζ η θ"""

    def __init__(self):
        self.symbols = "αβγδεζηθ"
        self.arrows = "← → ↑ ↓ ↔ ↕"

    def transform(self, data):
        """Apply transformation → result."""
        return [x * 2 for x in data]
"#,
    )
    .unwrap();

    // JavaScript with emoji and special chars
    fs::write(
        dir.join("app.js"),
        r#"/**
 * Application entry point 🚀
 * Handles routing─middleware─database connections.
 */
const SEPARATOR = '═══════════════════════════════';

// Config with unicode keys (edge case)
const config = {
  'name': 'Ōōbō App',
  'version': '1.0.0─beta',
  'encoding': 'UTF─8'
};

function renderDashboard(user) {
  console.log(`┌${'─'.repeat(30)}┐`);
  console.log(`│ Welcome, ${user.name}${' '.repeat(18 - user.name.length)}│`);
  console.log(`└${'─'.repeat(30)}┘`);
}

// Arrows in comments: → ← ↑ ↓
export default { config, renderDashboard };
"#,
    )
    .unwrap();

    // Markdown with lots of box-drawing and unicode
    fs::write(
        dir.join("README.md"),
        r#"# Project Nāme with Ünïcödé

## Architecture

```
┌─────────┐     ┌─────────┐     ┌─────────┐
│  Client │────→│   API   │────→│   DB    │
└─────────┘     └─────────┘     └─────────┘
```

## Features

- Fast search → O(log n) lookup
- Unicode support: 日本語, 한국어, العربية
- Box drawing: ├── └── ─── ┬── ┼──

## Emoji Table

| Feature | Status |
|---------|--------|
| Search  | ✅     |
| Index   | ✅     |
| Watch   | 🔄     |

---

© 2024 Ōōbō — all rights reserved.
"#,
    )
    .unwrap();

    // Large Rust file to trigger actual tree-sitter boundary issues
    let mut large_rust = String::new();
    large_rust.push_str("//! Module with many functions to test chunk splitting with unicode.\n\n");
    for i in 0..50 {
        large_rust.push_str(&format!(
            "/// Function {i} — handles edge─case processing.\n\
             pub fn func_{i}(input: &str) -> String {{\n\
             \x20   let separator = \"─────────────\";\n\
             \x20   format!(\"{{separator}} {{input}} → result_{i}\")\n\
             }}\n\n"
        ));
    }
    fs::write(dir.join("large_module.rs"), &large_rust).unwrap();

    // Go file
    fs::write(
        dir.join("main.go"),
        r#"package main

import "fmt"

// buildTree renders a tree─style diagram
func buildTree(items []string) string {
	result := ""
	for i, item := range items {
		if i == len(items)-1 {
			result += "└── " + item + "\n"
		} else {
			result += "├── " + item + "\n"
		}
	}
	return result
}

func main() {
	fmt.Println("Hello 世界")
	fmt.Println(buildTree([]string{"α", "β", "γ"}))
}
"#,
    )
    .unwrap();
}

#[test]
fn test_index_project_with_multibyte_utf8() {
    let dir = TempDir::new().unwrap();
    create_sample_project(dir.path());

    let output = Command::new(sonar_bin())
        .args(["index", dir.path().to_str().unwrap()])
        .output()
        .expect("failed to run sonar");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "sonar index panicked/failed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("indexed_files"),
        "Expected JSON output with indexed_files.\nstdout: {stdout}"
    );
}

#[test]
fn test_search_project_with_multibyte_utf8() {
    let dir = TempDir::new().unwrap();
    create_sample_project(dir.path());

    // Index first
    let index_result = Command::new(sonar_bin())
        .args(["index", dir.path().to_str().unwrap()])
        .output()
        .expect("failed to run sonar index");
    assert!(
        index_result.status.success(),
        "index failed: {}",
        String::from_utf8_lossy(&index_result.stderr)
    );

    // Search for something that exists in the unicode-heavy files
    let search_result = Command::new(sonar_bin())
        .args(["search", "support_dir", "-p", dir.path().to_str().unwrap()])
        .output()
        .expect("failed to run sonar search");

    let stdout = String::from_utf8_lossy(&search_result.stdout);
    let stderr = String::from_utf8_lossy(&search_result.stderr);

    assert!(
        search_result.status.success(),
        "sonar search panicked/failed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("results"),
        "Expected JSON output with results.\nstdout: {stdout}"
    );
}

#[test]
fn test_index_oobo_cli_no_panic() {
    let oobo_cli_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("oobo-cli");

    if !oobo_cli_path.exists() {
        eprintln!("Skipping: oobo-cli not found at {:?}", oobo_cli_path);
        return;
    }

    let output = Command::new(sonar_bin())
        .args(["index", oobo_cli_path.to_str().unwrap()])
        .output()
        .expect("failed to run sonar");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "sonar index on oobo-cli panicked.\nstdout: {stdout}\nstderr: {stderr}"
    );
}
