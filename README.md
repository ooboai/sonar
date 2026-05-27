# sonar

Fast hybrid code search for AI agents. Pure Rust port of [semble](https://github.com/MinishLab/semble).

> **This project is a verbatim Rust translation of semble** — same algorithm, same constants, same ranking pipeline. Full credit to [MinishLab](https://github.com/MinishLab) for designing the search and ranking system. We maintain this port because we need a single-binary, zero-dependency solution that AI agents can install in any sandbox without Python.

## Why this exists

AI coding agents (Cursor, Claude Code, Codex, etc.) waste significant tokens re-reading codebases. They `grep` for a keyword, get 90+ file matches, then `cat` multiple files hunting for the right context. Sonar gives them the exact code chunks they need in one call — **5 ranked results instead of 90 files to read**.

This project is:
- **100% AI-built** with minor human steering
- **A living mirror of semble** — when semble ships improvements, we port them
- **Open for agents to maintain** — the intention is that LLM agents keep this in sync with upstream

## Performance

| Metric | Value |
|--------|-------|
| Index 1500-chunk Rust project | 1.3s |
| Index 5600-chunk Python project | 2.4s |
| Search latency (cached index) | ~130ms avg |
| Binary size | ~15MB |
| Dependencies at runtime | zero (statically linked) |

## Features

- **Hybrid search** — BM25 keyword + Model2Vec semantic, fused with Reciprocal Rank Fusion
- **Tree-sitter chunking** — Python, Rust, JavaScript, TypeScript, TSX, Go, Java + Markdown heading splits + line fallback
- **290+ file extensions** recognized
- **Pure Rust** — no Python, no ONNX runtime, no C dependencies. Single static binary.
- **MCP server** — stdio JSON-RPC, same tool schemas as semble
- **Index persistence** — OS cache dir with BLAKE3 staleness detection + per-file mtime tracking
- **File watching** — automatic re-indexing on changes via `notify`
- **`.gitignore` + `.sonarignore` support** — respects your ignore rules
- **Git clone support** — index remote repos directly via HTTPS URL
- **Graceful fallback** — if embedding model can't download, falls back to BM25-only

## Install

```bash
cargo install --path crates/cli
```

Or build from source:

```bash
git clone https://github.com/ooboai/sonar.git
cd sonar
cargo build --release
# Binaries at target/release/sonar and target/release/sonar-mcp
```

## Usage

### CLI

```bash
# Index a codebase
sonar index /path/to/project

# Search (hybrid: BM25 + semantic)
sonar search "auth middleware" -p /path/to/project

# Search modes
sonar search "parse config" -p ./project --mode hybrid    # default
sonar search "parse config" -p ./project --mode bm25      # keyword only (no model needed)
sonar search "parse config" -p ./project --mode semantic   # vector only

# Index a remote repo
sonar index https://github.com/some/repo.git

# Pre-download embedding model
sonar download-model

# Watch for changes and re-index automatically
sonar watch /path/to/project

# View token savings stats
sonar savings
```

### MCP Server

```bash
sonar-mcp
```

Exposes two tools over stdio JSON-RPC:

- **`search`** — search a codebase with natural language or code queries
- **`find_related`** — find semantically similar code to a given location

Compatible with any MCP client (Cursor, Claude Desktop, etc.). Add to your MCP config:

```json
{
  "mcpServers": {
    "sonar": {
      "command": "sonar-mcp",
      "args": []
    }
  }
}
```

### As a Library

```toml
[dependencies]
sonar-core = { git = "https://github.com/ooboai/sonar" }
```

```rust
use sonar_core::index::SonarIndex;

let index = SonarIndex::from_path_cached(Path::new("./my-project"), None)?;
let results = index.search("error handling", 10);
for r in &results {
    println!("{} L{}-{} (score: {:.3})",
        r.chunk.file_path, r.chunk.start_line, r.chunk.end_line, r.score);
}
```

## Architecture

```
sonar/
├── crates/
│   ├── core/       # Library: chunking, BM25, embeddings, ANN, ranking, persistence
│   ├── cli/        # CLI binary
│   └── mcp/        # MCP server binary
├── benchmarks/     # Parity tests and token efficiency benchmarks
└── Cargo.toml      # Workspace
```

### How it works

1. **Walk** — discover source files via `ignore` crate (respects `.gitignore` + `.sonarignore`), detect languages from 290+ extensions
2. **Chunk** — split files into semantic units using tree-sitter (functions, classes, structs, methods) with merge + split for consistent sizes
3. **Index** — build BM25 inverted index + Model2Vec embedding vectors (brute-force flat ANN)
4. **Search** — score with both BM25 and cosine similarity, fuse with Reciprocal Rank Fusion
5. **Rank** — apply symbol definition boosts, path penalties, file saturation decay, embedded-symbol boosts

### Embedding Model

Uses [Model2Vec](https://github.com/MinishLab/model2vec) `potion-code-16M` via the official [`model2vec-rs`](https://crates.io/crates/model2vec-rs) crate. Auto-downloads from HuggingFace Hub on first use (~30MB). Falls back to BM25-only if offline or download fails.

Override the model with `SONAR_MODEL_NAME` env var.

## Relation to semble

This is a **direct port** of [semble](https://github.com/MinishLab/semble) by [MinishLab](https://github.com/MinishLab) (MIT licensed). We intentionally match their:

- Chunking strategy and constants (`DESIRED_CHUNK_LENGTH_CHARS = 1500`)
- BM25 parameters (`k1 = 1.2`, `b = 0.75`)
- RRF fusion (`k = 60`, candidate counts)
- Alpha weighting logic (symbol detection, natural language detection)
- Ranking pipeline (definition boost, path penalty, file saturation)
- MCP tool schemas

When semble updates their algorithm, we update ours. This is not a fork — it's a rewrite in a different language with the explicit goal of staying in sync.

### Why not just use semble directly?

- **Semble requires Python.** AI agents in sandboxes can't always install Python + pip dependencies.
- **Single binary distribution.** `sonar` is one static binary, no runtime deps.
- **Embeddable.** Can be linked as a Rust library into other tools (like [oobo](https://github.com/ooboai/oobo-cli)).

## Contributing

This project is primarily maintained by AI agents with human oversight. Contributions welcome — especially:

- Porting new semble features as they ship
- Adding tree-sitter grammars for more languages
- Performance improvements
- Bug fixes with test cases

## Credits

- **[semble](https://github.com/MinishLab/semble)** by [MinishLab](https://github.com/MinishLab) — the original Python implementation that this project ports
- **[Model2Vec](https://github.com/MinishLab/model2vec)** by MinishLab — the embedding model and Rust inference library
- **[oobo](https://github.com/ooboai/oobo-cli)** — the AI code attribution tool that uses sonar for local code search

## License

MIT — see [LICENSE](LICENSE).

This project is a derivative work of [semble](https://github.com/MinishLab/semble) (also MIT licensed, Copyright (c) 2026 Thomas van Dongen). The algorithm, constants, and ranking logic are ported from semble with full attribution.
