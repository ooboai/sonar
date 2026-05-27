# sonar - Implementation Plan

Rust translation of [semble](https://github.com/MinishLab/semble).
Same algorithm, same constants, same MCP tool schemas.

## Module Map: semble → sonar

| semble (Python) | sonar (Rust) | Status |
|---|---|---|
| `types.py` - Chunk, SearchResult, IndexStats | `core/src/types.rs` | **Done** |
| `tokens.py` - tokenize, split_identifier | `core/src/tokens.rs` | **Done** |
| `ranking/weighting.py` - resolve_alpha | `core/src/rank/weight.rs` | **Done** |
| `ranking/boosting.py` - symbol detection, boosts | `core/src/rank/boost.rs` | **Done** |
| `ranking/penalties.py` - file path penalties, rerank | `core/src/rank/penalty.rs` | **Done** |
| `search.py` - hybrid search, RRF | `core/src/search.rs` | **Done** |
| `chunking/chunking.py` - chunk_source | `core/src/chunk.rs` | **Done** |
| `chunking/core.py` - tree-sitter chunking | `core/src/chunk.rs` | **Done** |
| `index/index.py` - SembleIndex | `core/src/index.rs` | **Done** (BM25 + Hybrid) |
| `index/create.py` - create_index_from_path | `core/src/index.rs` | **Done** |
| `index/dense.py` - vector index (vicinity) | `core/src/ann.rs` | **Done** |
| `index/sparse.py` - BM25 wrapper | `core/src/bm25.rs` | **Done** |
| `index/files.py` - extension lists | `core/src/walk.rs` | **Done** |
| `index/file_walker.py` - file discovery | `core/src/walk.rs` | **Done** |
| `stats.py` - token savings tracking | Not porting (semble-specific analytics) | Skip |
| `utils.py` - format_results, resolve_chunk | `core/src/utils.rs` | **Done** |
| `mcp.py` - MCP server | `mcp/src/main.rs` | **Done** |
| `cli.py` - CLI | `cli/src/main.rs` | **Done** |
| - (model loading) | `core/src/embed.rs` (model2vec-rs) | **Done** |
| - (index persistence) | `core/src/persist.rs` | **Done** |

## Current State

**Phases 1-3 are complete.** 108 tests passing, zero warnings.

### What's built
- BM25 keyword search, semantic vector search, and hybrid RRF fusion
- Model2Vec embeddings via `model2vec-rs` (pure Rust, no Python/C deps)
- Brute-force cosine ANN (`ann.rs`)
- Tree-sitter chunking for Python, Rust, JS, TS, TSX, Go, Java
- Markdown heading-based chunking + line-based fallback
- Full ranking pipeline: RRF, alpha weighting, symbol/NL boosts, path penalties, file saturation decay
- Index persistence (binary format with BLAKE3 staleness detection)
- MCP server (stdio JSON-RPC, `search` + `find_related` tools)
- CLI: `sonar index`, `sonar search --mode hybrid|semantic|bm25`, `sonar download-model`
- Graceful fallback: if model unavailable, falls back to BM25-only

## Implementation Sequence

### Phase 1: Core search engine (BM25-only) ✅

1. **File walker** (`walk.rs`) ✅
2. **Tree-sitter chunking** (`chunk.rs`) ✅
3. **BM25 index** (`bm25.rs`, `index.rs`) ✅
4. **Wire up search** (`search.rs`) ✅
5. **CLI** (`cli/`) ✅

### Phase 2: Semantic search (add embeddings) ✅

6. **Embeddings** (`embed.rs`) - thin wrapper around `model2vec-rs` crate ✅
7. **ANN** (`ann.rs`) - brute-force cosine similarity with min-heap top-k ✅
8. **Hybrid search** - BM25 + semantic via RRF, with mode selection ✅
9. **CLI mode flag** - `--mode hybrid|semantic|bm25` ✅

### Phase 3: MCP server + persistence ✅

10. **MCP server** (`mcp/`) ✅
    - stdio JSON-RPC transport
    - `search` tool + `find_related` tool
    - Index caching (HashMap per session)
    - 24 tests
11. **Index persistence** (`persist.rs`) ✅
    - Binary format: magic bytes, version, BLAKE3 content hash, chunks, BM25 docs
    - Staleness detection via content hash comparison
    - `from_path_cached` for instant warm starts

### Phase 3b: File watching (in progress)

12. **File watcher** (`watch.rs`) - `notify` crate, debounced, incremental re-index

### Phase 4: Integration with oobo (in progress)

13. **Add sonar-core as path dependency to oobo-cli**
14. **Create `src/sonar.rs` integration module**
15. **Wire into oobo's MCP command** alongside memory tools

## Key Constants (ported from semble)

| Constant | Value | Source |
|---|---|---|
| RRF_K | 60 | search.py |
| ALPHA_SYMBOL | 0.3 | ranking/weighting.py |
| ALPHA_NL | 0.5 | ranking/weighting.py |
| DEFINITION_BOOST_MULTIPLIER | 3.0 | ranking/boosting.py |
| STEM_BOOST_MULTIPLIER | 1.0 | ranking/boosting.py |
| FILE_COHERENCE_BOOST_FRAC | 0.2 | ranking/boosting.py |
| STRONG_PENALTY | 0.3 | ranking/penalties.py |
| MODERATE_PENALTY | 0.5 | ranking/penalties.py |
| MILD_PENALTY | 0.7 | ranking/penalties.py |
| FILE_SATURATION_THRESHOLD | 1 | ranking/penalties.py |
| FILE_SATURATION_DECAY | 0.5 | ranking/penalties.py |
| DESIRED_CHUNK_LENGTH_CHARS | 1500 | chunking/chunking.py |
| CANDIDATE_COUNT | top_k * 5 | search.py |

## Rust Dependencies

| Purpose | Python (semble) | Rust (sonar) |
|---|---|---|
| Tree-sitter | tree_sitter, tree_sitter_languages | tree-sitter crate + language grammars |
| Embeddings | model2vec (StaticModel) | model2vec-rs (official MinishLab crate) |
| BM25 | bm25s | Custom implementation (`bm25.rs`) |
| Vectors | numpy, vicinity | Brute-force cosine (`ann.rs`) |
| MCP | mcp (FastMCP) | Custom stdio JSON-RPC |
| CLI | click | clap |
| File walking | pathlib, os.walk | walkdir + ignore crates |
| Serialization | orjson | serde_json |
| File watching | watchfiles | notify crate |
| Hashing | - | blake3 |
