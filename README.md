# sonar

Fast hybrid code search for AI agents. Pure Rust, drop-in compatible with [semble](https://github.com/MinishLab/semble).

## Features

- **Hybrid search** — BM25 keyword + Model2Vec semantic, fused with Reciprocal Rank Fusion
- **Tree-sitter chunking** — Python, Rust, JavaScript, TypeScript, Go, Java + Markdown heading splits + line fallback
- **Pure Rust** — no Python, no ONNX runtime, no C dependencies. Single static binary.
- **MCP server** — stdio JSON-RPC, same tool schemas as semble
- **Index persistence** — binary format with BLAKE3 staleness detection
- **File watching** — automatic re-indexing on changes

## Install

```bash
cargo install --path crates/cli
```

## Usage

### CLI

```bash
# Index a codebase
sonar index /path/to/project

# Search (BM25 + semantic hybrid)
sonar search "auth middleware" --path /path/to/project -k 10

# Search modes
sonar search "parse config" --mode hybrid    # default: BM25 + semantic
sonar search "parse config" --mode bm25      # keyword only
sonar search "parse config" --mode semantic  # vector only

# Pre-download embedding model (also auto-downloads on first hybrid search)
sonar download-model

# Watch for changes and re-index
sonar watch /path/to/project
```

### MCP Server

```bash
sonar-mcp
```

Exposes two tools over stdio JSON-RPC:

- **`search`** — search a codebase with natural language or code queries
- **`find_related`** — find semantically similar code to a given location

### As a Library

```toml
[dependencies]
sonar-core = { git = "https://github.com/ooboai/sonar" }
```

```rust
use sonar_core::index::SonarIndex;

let index = SonarIndex::from_path(Path::new("./my-project"))?;
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
│   ├── core/       # Library: chunking, BM25, embeddings, ranking, search
│   ├── cli/        # CLI binary
│   └── mcp/        # MCP server binary
├── Cargo.toml      # Workspace
└── PLAN.md         # Implementation plan
```

### How it works

1. **Walk** — discover source files, detect languages
2. **Chunk** — split files into semantic units using tree-sitter (functions, classes, structs)
3. **Index** — build BM25 inverted index + Model2Vec embedding vectors
4. **Search** — score with both BM25 and cosine similarity, fuse with RRF
5. **Rank** — apply symbol boosts, path penalties, file saturation decay

### Embedding Model

Uses [Model2Vec](https://github.com/MinishLab/model2vec) `potion-code-16M` via the official [`model2vec-rs`](https://crates.io/crates/model2vec-rs) crate. Downloads from HuggingFace Hub on first use (~30MB). Falls back to BM25-only if unavailable.

## Credits

Verbatim port of [semble](https://github.com/MinishLab/semble) by [MinishLab](https://github.com/MinishLab). Same algorithm, same constants, same ranking pipeline.

## License

MIT
