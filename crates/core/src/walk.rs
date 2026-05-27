use std::path::Path;

use ignore::WalkBuilder;

use crate::types::ContentType;

const MAX_FILE_BYTES: u64 = 1_000_000;
const MIN_FILE_BYTES: u64 = 128;

const ALWAYS_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    "node_modules",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".cache",
    ".semble",
    ".sonar",
    ".next",
    "dist",
    "build",
    ".eggs",
];

pub struct WalkedFile {
    pub relative_path: String,
    pub content: String,
    pub language: Option<String>,
    pub content_type: ContentType,
}

/// Detect programming language from file extension.
///
/// Covers ~290 extensions mapping to ~230 languages, matching semble's coverage.
pub fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    let lang = match ext {
        // Python
        "py" => "python",

        // Rust
        "rs" => "rust",

        // JavaScript family
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",

        // Go
        "go" => "go",

        // Java
        "java" => "java",

        // Ruby
        "rb" => "ruby",

        // PHP
        "php" => "php",

        // C
        "c" | "h" => "c",

        // C++
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",

        // C#
        "cs" => "csharp",

        // Swift
        "swift" => "swift",

        // Kotlin
        "kt" | "kts" => "kotlin",

        // Scala
        "scala" | "sc" => "scala",

        // Clojure
        "clj" | "cljs" | "cljc" | "edn" => "clojure",

        // Elixir
        "ex" | "exs" => "elixir",

        // Erlang
        "erl" | "hrl" => "erlang",

        // Haskell
        "hs" | "lhs" => "haskell",

        // Lua
        "lua" => "lua",

        // Perl
        "pl" | "pm" => "perl",

        // R
        "r" | "R" | "rmd" => "r",

        // Julia
        "jl" => "julia",

        // Dart
        "dart" => "dart",

        // Zig
        "zig" => "zig",

        // Nim
        "nim" | "nimble" => "nim",

        // V
        "v" | "vsh" => "vlang",

        // Odin
        "odin" => "odin",

        // Mojo
        "mojo" => "mojo",

        // Cairo
        "cairo" => "cairo",

        // Solidity
        "sol" => "solidity",

        // Vyper
        "vy" => "vyper",

        // Move
        "move" => "move",

        // Elm
        "elm" => "elm",

        // OCaml
        "ml" | "mli" => "ocaml",

        // F#
        "fs" | "fsi" | "fsx" | "fsscript" => "fsharp",

        // Pascal / Delphi
        "pas" | "pp" | "lpr" | "dpr" => "pascal",

        // D
        "d" => "dlang",

        // Ada
        "ada" | "adb" | "ads" => "ada",

        // Fortran
        "f" | "f90" | "f95" | "f03" | "f08" | "for" | "fpp" => "fortran",

        // VHDL
        "vhd" | "vhdl" => "vhdl",

        // SystemVerilog
        "sv" | "svh" => "systemverilog",

        // Tcl
        "tcl" => "tcl",

        // AWK
        "awk" => "awk",

        // sed
        "sed" => "sed",

        // Objective-C
        "m" => "objectivec",
        "mm" => "objectivecpp",

        // Lisp family
        "lisp" | "lsp" | "cl" => "commonlisp",
        "scm" | "ss" => "scheme",
        "rkt" => "racket",

        // Groovy / Gradle
        "groovy" | "gvy" | "gy" | "gsh" | "gradle" => "groovy",

        // Crystal
        "cr" => "crystal",

        // Hack
        "hack" | "hh" | "hhi" => "hack",

        // Pony
        "pony" => "pony",

        // Chapel
        "chapel" | "chpl" => "chapel",

        // Forth
        "forth" | "fth" | "4th" => "forth",

        // Factor
        "factor" => "factor",

        // Io
        "io" => "io",

        // Pike
        "pike" | "pmod" => "pike",

        // Red
        "red" | "reds" => "red",

        // Ring
        "ring" => "ring",

        // Wren
        "wren" => "wren",

        // Ballerina
        "ballerina" | "bal" => "ballerina",

        // HCL / Terraform
        "hcl" | "tf" | "tfvars" => "hcl",

        // Nix
        "nix" => "nix",

        // Dhall
        "dhall" => "dhall",

        // CUE
        "cue" => "cue",

        // Jsonnet
        "jsonnet" | "libsonnet" => "jsonnet",

        // Pkl
        "pkl" => "pkl",

        // Starlark / Bazel
        "starlark" | "bzl" => "starlark",

        // CMake
        "cmake" => "cmake",

        // Make
        "make" | "makefile" => "make",

        // Dockerfile
        "dockerfile" | "containerfile" => "dockerfile",

        // Shell family
        "sh" | "bash" | "zsh" => "bash",
        "fish" => "fish",
        "nu" => "nushell",
        "elvish" => "elvish",
        "ion" => "ion",
        "xonsh" => "xonsh",

        // Batch / PowerShell
        "bat" | "cmd" => "batch",
        "ps1" | "psm1" | "psd1" => "powershell",

        // SQL variants
        "sql" | "pgsql" | "mysql" | "plsql" => "sql",
        "sparql" => "sparql",
        "cypher" => "cypher",

        // GraphQL
        "graphql" | "gql" => "graphql",

        // Protocol Buffers / IDL
        "proto" | "protobuf" => "protobuf",
        "thrift" => "thrift",
        "avdl" => "avro",
        "capnp" => "capnproto",
        "flatbuffers" | "fbs" => "flatbuffers",
        "smithy" => "smithy",
        "wit" => "wit",
        "aidl" => "aidl",

        // WebAssembly
        "wasm" | "wat" => "wasm",

        // LLVM
        "ll" | "bc" => "llvm",

        // Assembly
        "s" | "asm" | "nasm" | "masm" => "assembly",

        // TeX / LaTeX / Typst
        "tex" | "sty" | "cls" | "bib" => "latex",
        "typ" => "typst",

        // Documentation formats (also returned by detect_content_type as Docs)
        "rst" => "restructuredtext",
        "adoc" | "asciidoc" => "asciidoc",
        "org" | "norg" => "org",
        "pod" => "pod",
        "man" | "roff" => "roff",
        "md" => "markdown",
        "mdx" => "mdx",
        "djot" => "djot",

        // Config formats (also returned by detect_content_type as Config)
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "ini" | "cfg" | "conf" => "ini",
        "xml" | "xsl" | "xslt" => "xml",
        "kdl" => "kdl",
        "ron" => "ron",
        "properties" => "properties",
        "desktop" => "desktop",
        "hocon" => "hocon",
        "hjson" => "hjson",
        "diff" | "patch" => "diff",
        "gitignore" | "gitattributes" => "gitconfig",
        "editorconfig" => "editorconfig",
        "dockerignore" => "dockerignore",
        "env" => "dotenv",
        "npmrc" | "browserslist" => "npmrc",

        // HTML
        "html" | "htm" => "html",

        // Mermaid
        "mermaid" => "mermaid",
        "vimdoc" => "vimdoc",

        // Rich text
        "rtf" => "rtf",
        "txt" => "plaintext",
        "latex" => "latex",

        _ => return None,
    };
    Some(lang.to_string())
}

/// Detect content type from file extension.
pub fn detect_content_type(path: &Path) -> ContentType {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Also handle special filenames without extensions
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Data files are never indexed — callers filter on ContentType
    if matches!(ext, "csv" | "json" | "json5" | "psv" | "tsv") {
        return ContentType::Data;
    }

    // Handle known filenames that are code (build systems, etc.)
    if matches!(
        filename.as_str(),
        "gnumakefile"
            | "makefile"
            | "dockerfile"
            | "containerfile"
            | "vagrantfile"
            | "rakefile"
            | "gemfile"
            | "justfile"
            | "earthfile"
    ) {
        return ContentType::Code;
    }

    // Handle BUILD files (Bazel)
    if filename == "build" && ext.is_empty() {
        return ContentType::Code;
    }

    match ext {
        // Docs
        "md" | "mdx" | "rst" | "adoc" | "asciidoc" | "txt" | "org" | "norg" | "tex" | "latex"
        | "html" | "htm" | "djot" | "pod" | "man" | "roff" | "rtf" | "mermaid" | "vimdoc" => {
            ContentType::Docs
        }

        // Config
        "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "xml" | "xsl" | "xslt" | "hcl"
        | "tf" | "tfvars" | "proto" | "smithy" | "kdl" | "ron" | "properties" | "desktop"
        | "hocon" | "hjson" | "diff" | "patch" | "gitignore" | "gitattributes" | "editorconfig"
        | "dockerignore" | "env" | "npmrc" | "browserslist" => ContentType::Config,

        _ => ContentType::Code,
    }
}

/// Build an `ignore::WalkBuilder` rooted at `root` with all standard ignore rules.
///
/// Used by both `walk_directory` and `walk_paths` (for content hashing) to ensure
/// consistent file traversal.
fn build_walker(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false) // we handle hidden dirs via ALWAYS_IGNORED_DIRS
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .add_custom_ignore_filename(".sonarignore");

    builder
}

/// Check if `.sonarignore` at `root` contains a `!*.ext` negation pattern for the given extension.
fn has_sonarignore_inclusion(root: &Path, ext: &str) -> bool {
    let ignore_path = root.join(".sonarignore");
    let content = match std::fs::read_to_string(&ignore_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let pattern = format!("!*.{ext}");
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == pattern || trimmed == format!("!*.{}", ext.to_uppercase())
    })
}

/// Walk a directory tree and return indexable files.
///
/// Respects `.gitignore`, `.git/info/exclude`, global gitignore, and `.sonarignore`.
/// Skips hardcoded directories, files > 1 MB, files < 128 bytes that are whitespace-only,
/// symlinks, and data files.
///
/// `content_filter` selects which content types to include. Data files are always excluded.
pub fn walk_directory(root: &Path, content_filter: &[ContentType]) -> Vec<WalkedFile> {
    let mut files = Vec::new();

    let walker = build_walker(root).build();

    for entry in walker.flatten() {
        // Skip directories and symlinks
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }

        let path = entry.path();

        // Check hardcoded ignored directories
        if path
            .components()
            .any(|c| ALWAYS_IGNORED_DIRS.contains(&c.as_os_str().to_str().unwrap_or("")))
        {
            continue;
        }

        // Size check
        if let Ok(meta) = path.metadata()
            && meta.len() > MAX_FILE_BYTES
        {
            continue;
        }

        let content_type = detect_content_type(path);

        // Data files are always excluded
        if content_type == ContentType::Data {
            continue;
        }

        // Content type filter — but force-included extensions bypass this
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let force_included = !ext.is_empty() && has_sonarignore_inclusion(root, ext);

        if !force_included && !content_filter.contains(&content_type) {
            continue;
        }

        let language = detect_language(path);

        // For Code files without a recognized language: skip unless force-included
        if content_type == ContentType::Code && language.is_none() && !force_included {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Skip small files that are whitespace-only
        if let Ok(meta) = path.metadata()
            && meta.len() < MIN_FILE_BYTES
            && content.trim().is_empty()
        {
            continue;
        }

        // Skip completely empty files
        if content.trim().is_empty() {
            continue;
        }

        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_str = relative.to_string_lossy().to_string();

        files.push(WalkedFile {
            relative_path: relative_str,
            content,
            language,
            content_type,
        });
    }

    files
}

/// Walk a directory tree and return just paths and mtimes (for content hashing).
///
/// Uses the same `ignore`-based walker as `walk_directory` for consistency.
pub fn walk_paths(root: &Path) -> Vec<(String, u64)> {
    let mut entries: Vec<(String, u64)> = Vec::new();

    let walker = build_walker(root).build();

    for entry in walker.flatten() {
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }

        let path = entry.path();

        if path
            .components()
            .any(|c| ALWAYS_IGNORED_DIRS.contains(&c.as_os_str().to_str().unwrap_or("")))
        {
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push((rel, mtime));
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sonar_walk_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // ─── Language Detection ───────────────────────────────────────────

    #[test]
    fn test_detect_language_core() {
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
    fn test_detect_language_broad_coverage() {
        let cases = vec![
            ("main.go", "go"),
            ("App.java", "java"),
            ("script.rb", "ruby"),
            ("index.php", "php"),
            ("main.c", "c"),
            ("lib.h", "c"),
            ("main.cpp", "cpp"),
            ("main.cc", "cpp"),
            ("main.cxx", "cpp"),
            ("lib.hpp", "cpp"),
            ("lib.hxx", "cpp"),
            ("Program.cs", "csharp"),
            ("App.swift", "swift"),
            ("Main.kt", "kotlin"),
            ("Main.kts", "kotlin"),
            ("Main.scala", "scala"),
            ("main.sc", "scala"),
            ("core.clj", "clojure"),
            ("core.cljs", "clojure"),
            ("core.cljc", "clojure"),
            ("data.edn", "clojure"),
            ("lib.ex", "elixir"),
            ("lib.exs", "elixir"),
            ("mod.erl", "erlang"),
            ("mod.hrl", "erlang"),
            ("Main.hs", "haskell"),
            ("Main.lhs", "haskell"),
            ("script.lua", "lua"),
            ("script.pl", "perl"),
            ("Mod.pm", "perl"),
            ("analysis.r", "r"),
            ("analysis.R", "r"),
            ("report.rmd", "r"),
            ("sim.jl", "julia"),
            ("main.dart", "dart"),
            ("main.zig", "zig"),
            ("main.nim", "nim"),
            ("config.nimble", "nim"),
            ("main.v", "vlang"),
            ("build.vsh", "vlang"),
            ("main.odin", "odin"),
            ("main.mojo", "mojo"),
            ("contract.cairo", "cairo"),
            ("Token.sol", "solidity"),
            ("Token.vy", "vyper"),
            ("module.move", "move"),
            ("Main.elm", "elm"),
            ("main.ml", "ocaml"),
            ("main.mli", "ocaml"),
            ("Main.fs", "fsharp"),
            ("Main.fsi", "fsharp"),
            ("Main.fsx", "fsharp"),
            ("Main.fsscript", "fsharp"),
            ("main.pas", "pascal"),
            ("unit.pp", "pascal"),
            ("project.lpr", "pascal"),
            ("project.dpr", "pascal"),
            ("main.d", "dlang"),
            ("main.ada", "ada"),
            ("main.adb", "ada"),
            ("main.ads", "ada"),
            ("main.f", "fortran"),
            ("main.f90", "fortran"),
            ("main.f95", "fortran"),
            ("main.f03", "fortran"),
            ("main.f08", "fortran"),
            ("main.for", "fortran"),
            ("main.fpp", "fortran"),
            ("design.vhd", "vhdl"),
            ("design.vhdl", "vhdl"),
            ("module.sv", "systemverilog"),
            ("module.svh", "systemverilog"),
            ("script.tcl", "tcl"),
            ("script.awk", "awk"),
            ("script.sed", "sed"),
            ("ViewController.m", "objectivec"),
            ("ViewController.mm", "objectivecpp"),
            ("code.lisp", "commonlisp"),
            ("code.lsp", "commonlisp"),
            ("code.cl", "commonlisp"),
            ("code.scm", "scheme"),
            ("code.ss", "scheme"),
            ("code.rkt", "racket"),
            ("build.groovy", "groovy"),
            ("build.gvy", "groovy"),
            ("build.gy", "groovy"),
            ("build.gsh", "groovy"),
            ("build.gradle", "groovy"),
            ("shard.cr", "crystal"),
            ("entrypoint.hack", "hack"),
            ("entrypoint.hh", "hack"),
            ("types.hhi", "hack"),
            ("actor.pony", "pony"),
            ("main.chapel", "chapel"),
            ("main.chpl", "chapel"),
            ("main.forth", "forth"),
            ("main.fth", "forth"),
            ("main.4th", "forth"),
            ("vocab.factor", "factor"),
            ("hello.io", "io"),
            ("server.pike", "pike"),
            ("mod.pmod", "pike"),
            ("app.red", "red"),
            ("sys.reds", "red"),
            ("app.ring", "ring"),
            ("game.wren", "wren"),
            ("service.ballerina", "ballerina"),
            ("service.bal", "ballerina"),
            ("main.hcl", "hcl"),
            ("main.tf", "hcl"),
            ("vars.tfvars", "hcl"),
            ("config.nix", "nix"),
            ("config.dhall", "dhall"),
            ("schema.cue", "cue"),
            ("config.jsonnet", "jsonnet"),
            ("lib.libsonnet", "jsonnet"),
            ("config.pkl", "pkl"),
            ("rules.starlark", "starlark"),
            ("deps.bzl", "starlark"),
            ("CMakeLists.cmake", "cmake"),
            ("run.sh", "bash"),
            ("run.bash", "bash"),
            ("run.zsh", "bash"),
            ("config.fish", "fish"),
            ("script.nu", "nushell"),
            ("script.elvish", "elvish"),
            ("script.ion", "ion"),
            ("script.xonsh", "xonsh"),
            ("run.bat", "batch"),
            ("run.cmd", "batch"),
            ("script.ps1", "powershell"),
            ("module.psm1", "powershell"),
            ("data.psd1", "powershell"),
            ("query.sql", "sql"),
            ("query.pgsql", "sql"),
            ("query.mysql", "sql"),
            ("query.plsql", "sql"),
            ("query.sparql", "sparql"),
            ("query.cypher", "cypher"),
            ("schema.graphql", "graphql"),
            ("schema.gql", "graphql"),
            ("msg.proto", "protobuf"),
            ("msg.protobuf", "protobuf"),
            ("service.thrift", "thrift"),
            ("schema.avdl", "avro"),
            ("schema.capnp", "capnproto"),
            ("schema.flatbuffers", "flatbuffers"),
            ("schema.fbs", "flatbuffers"),
            ("model.smithy", "smithy"),
            ("world.wit", "wit"),
            ("IFoo.aidl", "aidl"),
            ("module.wasm", "wasm"),
            ("module.wat", "wasm"),
            ("ir.ll", "llvm"),
            ("ir.bc", "llvm"),
            ("boot.s", "assembly"),
            ("boot.asm", "assembly"),
            ("boot.nasm", "assembly"),
            ("boot.masm", "assembly"),
            ("paper.tex", "latex"),
            ("style.sty", "latex"),
            ("article.cls", "latex"),
            ("refs.bib", "latex"),
            ("doc.typ", "typst"),
            ("doc.rst", "restructuredtext"),
            ("doc.adoc", "asciidoc"),
            ("doc.asciidoc", "asciidoc"),
            ("notes.org", "org"),
            ("notes.norg", "org"),
            ("doc.pod", "pod"),
            ("page.man", "roff"),
            ("page.roff", "roff"),
            ("readme.md", "markdown"),
            ("guide.mdx", "mdx"),
            ("doc.djot", "djot"),
            // config-type extensions that still get a language
            ("config.yaml", "yaml"),
            ("config.yml", "yaml"),
            ("config.toml", "toml"),
            ("app.ini", "ini"),
            ("app.cfg", "ini"),
            ("app.conf", "ini"),
            ("layout.xml", "xml"),
            ("transform.xsl", "xml"),
            ("transform.xslt", "xml"),
            ("config.kdl", "kdl"),
            ("config.ron", "ron"),
            ("app.properties", "properties"),
            ("app.desktop", "desktop"),
            ("app.hocon", "hocon"),
            ("app.hjson", "hjson"),
            ("changes.diff", "diff"),
            ("fix.patch", "diff"),
            (".gitignore_file.gitignore", "gitconfig"),
            (".gitattributes_file.gitattributes", "gitconfig"),
            (".editorconfig_file.editorconfig", "editorconfig"),
            (".dockerignore_file.dockerignore", "dockerignore"),
            ("local.env", "dotenv"),
            ("proj.npmrc", "npmrc"),
            ("proj.browserslist", "npmrc"),
            ("page.html", "html"),
            ("page.htm", "html"),
            ("diagram.mermaid", "mermaid"),
            ("help.vimdoc", "vimdoc"),
            ("doc.rtf", "rtf"),
            ("notes.txt", "plaintext"),
            ("doc.latex", "latex"),
        ];

        for (file, expected_lang) in cases {
            let result = detect_language(Path::new(file));
            assert_eq!(
                result,
                Some(expected_lang.to_string()),
                "detect_language({file}) should be {expected_lang}"
            );
        }
    }

    // ─── Content Type Detection ───────────────────────────────────────

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
    fn test_detect_content_type_data_excluded() {
        assert_eq!(
            detect_content_type(Path::new("data.csv")),
            ContentType::Data
        );
        assert_eq!(
            detect_content_type(Path::new("data.json")),
            ContentType::Data
        );
        assert_eq!(
            detect_content_type(Path::new("data.json5")),
            ContentType::Data
        );
        assert_eq!(
            detect_content_type(Path::new("data.tsv")),
            ContentType::Data
        );
        assert_eq!(
            detect_content_type(Path::new("data.psv")),
            ContentType::Data
        );
    }

    #[test]
    fn test_detect_content_type_special_filenames() {
        assert_eq!(
            detect_content_type(Path::new("Makefile")),
            ContentType::Code
        );
        assert_eq!(
            detect_content_type(Path::new("Dockerfile")),
            ContentType::Code
        );
        assert_eq!(
            detect_content_type(Path::new("Justfile")),
            ContentType::Code
        );
    }

    // ─── gitignore Respected ──────────────────────────────────────────

    #[test]
    fn test_gitignore_respected() {
        let dir = tmp_dir("gitignore");

        // init a git repo so .gitignore is picked up
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .unwrap();

        write_file(&dir, ".gitignore", "ignored.rs\n");
        write_file(
            &dir,
            "ignored.rs",
            "fn ignored() {\n    // should not appear\n}\n",
        );
        write_file(&dir, "kept.rs", "fn kept() {\n    // should appear\n}\n");

        let files = walk_directory(&dir, &[ContentType::Code]);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert!(
            !paths.contains(&"ignored.rs"),
            "ignored.rs should be excluded by .gitignore"
        );
        assert!(paths.contains(&"kept.rs"), "kept.rs should be included");

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── .sonarignore Respected ───────────────────────────────────────

    #[test]
    fn test_sonarignore_respected() {
        let dir = tmp_dir("sonarignore");

        write_file(&dir, ".sonarignore", "secret.rs\n");
        write_file(
            &dir,
            "secret.rs",
            "fn secret() {\n    // should not appear\n}\n",
        );
        write_file(
            &dir,
            "visible.rs",
            "fn visible() {\n    // should appear\n}\n",
        );

        let files = walk_directory(&dir, &[ContentType::Code]);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert!(
            !paths.contains(&"secret.rs"),
            "secret.rs should be excluded by .sonarignore"
        );
        assert!(
            paths.contains(&"visible.rs"),
            "visible.rs should be included"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Content-Type Filtering ───────────────────────────────────────

    #[test]
    fn test_content_type_filtering() {
        let dir = tmp_dir("content_filter");

        write_file(
            &dir,
            "main.rs",
            "fn main() {\n    println!(\"hello\");\n}\n",
        );
        write_file(&dir, "README.md", "# My Project\n\nThis is a readme.\n");
        write_file(&dir, "config.toml", "[package]\nname = \"test\"\n");

        // Code only (default)
        let code_files = walk_directory(&dir, &[ContentType::Code]);
        assert!(code_files.iter().any(|f| f.relative_path == "main.rs"));
        assert!(!code_files.iter().any(|f| f.relative_path == "README.md"));
        assert!(!code_files.iter().any(|f| f.relative_path == "config.toml"));

        // Docs only
        let doc_files = walk_directory(&dir, &[ContentType::Docs]);
        assert!(!doc_files.iter().any(|f| f.relative_path == "main.rs"));
        assert!(doc_files.iter().any(|f| f.relative_path == "README.md"));

        // Multiple types
        let mixed = walk_directory(
            &dir,
            &[ContentType::Code, ContentType::Docs, ContentType::Config],
        );
        assert!(mixed.iter().any(|f| f.relative_path == "main.rs"));
        assert!(mixed.iter().any(|f| f.relative_path == "README.md"));
        assert!(mixed.iter().any(|f| f.relative_path == "config.toml"));

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Walk on Self ─────────────────────────────────────────────────

    #[test]
    fn test_walk_directory_on_self() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let files = walk_directory(&manifest_dir, &[ContentType::Code]);
        assert!(
            !files.is_empty(),
            "should find at least the crate's own .rs files"
        );
        assert!(
            files.iter().any(|f| f.relative_path.ends_with(".rs")),
            "should contain .rs files"
        );
    }

    // ─── Hardcoded Ignores ────────────────────────────────────────────

    #[test]
    fn test_hardcoded_ignored_dirs() {
        let dir = tmp_dir("hardcoded_ignore");

        write_file(
            &dir,
            "src/main.rs",
            "fn main() {\n    println!(\"hello\");\n}\n",
        );
        write_file(&dir, "node_modules/dep/index.js", "module.exports = {};\n");
        write_file(&dir, "__pycache__/mod.py", "# cached\n");
        write_file(&dir, ".venv/lib/site.py", "# venv\nimport os\n");

        let files = walk_directory(&dir, &[ContentType::Code]);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

        assert!(paths.iter().any(|p| p.contains("main.rs")));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
        assert!(!paths.iter().any(|p| p.contains("__pycache__")));
        assert!(!paths.iter().any(|p| p.contains(".venv")));

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Data Files Always Excluded ───────────────────────────────────

    #[test]
    fn test_data_files_always_excluded() {
        let dir = tmp_dir("data_excluded");

        write_file(&dir, "data.csv", "a,b,c\n1,2,3\n");
        write_file(&dir, "config.json", "{\"key\": \"value\"}\n");
        write_file(
            &dir,
            "main.rs",
            "fn main() {\n    println!(\"hello\");\n}\n",
        );

        // Even if we ask for all content types, data files are excluded
        let files = walk_directory(
            &dir,
            &[ContentType::Code, ContentType::Docs, ContentType::Config],
        );
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

        assert!(!paths.contains(&"data.csv"), "CSV should be excluded");
        assert!(!paths.contains(&"config.json"), "JSON should be excluded");
        assert!(paths.contains(&"main.rs"), "Rust files should be included");

        let _ = fs::remove_dir_all(&dir);
    }

    // ─── Force-include via !*.ext in .sonarignore ───────────────────────

    #[test]
    fn test_sonarignore_force_include() {
        let dir = tmp_dir("force_include");

        write_file(&dir, ".sonarignore", "!*.proto\n!*.cob\n");
        write_file(
            &dir,
            "schema.proto",
            "syntax = \"proto3\";\n\nmessage User {\n  string name = 1;\n  int32 age = 2;\n}\n",
        );
        write_file(
            &dir,
            "legacy.cob",
            "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. HELLO.\n       PROCEDURE DIVISION.\n           DISPLAY 'HELLO WORLD'.\n           STOP RUN.\n",
        );
        write_file(
            &dir,
            "main.rs",
            "fn main() {\n    println!(\"hello\");\n}\n",
        );
        // .unknown file without a force-include should still be skipped
        write_file(&dir, "data.xyz", "some unknown format content here\n");

        let files = walk_directory(&dir, &[ContentType::Code]);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

        assert!(paths.contains(&"main.rs"), "Rust files included normally");
        assert!(
            paths.contains(&"schema.proto"),
            "Proto files force-included via !*.proto"
        );
        assert!(
            paths.contains(&"legacy.cob"),
            "COBOL files force-included via !*.cob"
        );
        assert!(
            !paths.contains(&"data.xyz"),
            "Unknown extensions without !*.ext still skipped"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
