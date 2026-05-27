# Benchmarking & Parity Testing

Sonar is a Rust translation of [semble](https://github.com/MinishLab/semble). These tools verify that sonar produces the same results and measure its token efficiency.

## Parity Tests

### What it tests

The parity test runs identical queries through both `sonar` and `semble` on the same repository and compares:

- **File overlap** - Jaccard similarity of file paths in the top-k results
- **Rank correlation** - how closely the ranking order matches between the two engines
- **Divergent queries** - any queries where overlap drops below 50%

### Running

```bash
# Prerequisites
cargo install --path crates/cli    # install sonar
pip install semble                  # install semble

# Run parity test
python benchmarks/parity_test.py /path/to/repo

# With options
python benchmarks/parity_test.py /path/to/repo --top-k 10
python benchmarks/parity_test.py /path/to/repo --queries custom_queries.txt
python benchmarks/parity_test.py /path/to/repo --json  # machine-readable output
```

### Target

**≥90% average file overlap** across the default query set. The script exits with code 1 if this target is not met.

### Expected results

Since sonar is a 1:1 port with the same constants, ranking pipeline, and chunking logic, differences should come from:

- **BM25 tokenization** - minor differences in stemming/tokenization between Rust and Python implementations
- **Embedding model** - sonar uses `model2vec-rs` while semble uses the Python `model2vec`; float precision may cause small ranking differences
- **Tree-sitter versions** - different grammar versions may produce slightly different chunk boundaries

Queries that target exact symbol names (e.g. `BM25Index`, `getUserById`) should have near-perfect overlap. Natural language queries (e.g. `how does search work`) may show more variation due to embedding precision.

## Token Efficiency

### What it measures

For each query, the token efficiency test compares:

- **Snippet tokens** - tokens in sonar's returned code chunks
- **File tokens** - tokens in the full source files those chunks came from
- **Savings ratio** - `1 - (snippet_tokens / file_tokens)`

This quantifies how much context window an agent saves by using sonar search instead of reading entire files with `cat`/`grep`.

### Running

```bash
# Prerequisites
cargo install --path crates/cli
pip install tiktoken  # optional, for accurate counts (falls back to char estimate)

# Run
python benchmarks/token_efficiency.py /path/to/repo
python benchmarks/token_efficiency.py /path/to/repo --top-k 10 --json
```

### Expected numbers

On typical codebases, sonar achieves roughly:

| Metric | Expected Range |
|---|---|
| Savings ratio | 70-95% |
| Compression ratio | 3-20x |

The actual numbers depend on file sizes and how focused the query is. Symbol queries on large files yield the highest savings.

## Semble Sync GitHub Action

### How it works

The `.github/workflows/semble-sync.yml` workflow runs every Monday at 9 AM UTC (and on manual dispatch). It:

1. Clones the latest semble from GitHub
2. Compares against a known-good commit (stored in `.semble-sync-state`)
3. Checks for changes in algorithm-relevant files:
   - `search.py`, `ranking/*`, `chunking/*`, `index/*`
   - `types.py`, `tokens.py`, `utils.py`
4. Extracts current values of key constants (RRF_K, boost multipliers, penalties)
5. Creates a GitHub issue labeled `semble-sync` if changes are detected

### Customizing

To track the last synced state, create a `.semble-sync-state` file containing the semble commit hash you've verified parity against:

```bash
echo "abc123def456" > .semble-sync-state
git add .semble-sync-state && git commit -m "track semble sync state"
```

After porting upstream changes, update this file to the new commit hash.

### Manual trigger

Go to **Actions → Semble Sync Check → Run workflow** in the GitHub UI, or:

```bash
gh workflow run semble-sync.yml
```
