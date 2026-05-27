# Sonar Benchmarks

Tools for verifying sonar's parity with [semble](https://github.com/MinishLab/semble) and measuring token efficiency.

## Prerequisites

```bash
# Build and install sonar
cargo install --path crates/cli

# Install semble (required for parity tests)
pip install semble

# Optional: accurate token counting (falls back to char-based estimate)
pip install tiktoken
```

## Parity Test

Runs the same queries through both sonar and semble, then compares the top-k results.

```bash
# Run against a local repo
python benchmarks/parity_test.py /path/to/repo

# Customize top-k
python benchmarks/parity_test.py /path/to/repo --top-k 10

# Use custom queries (one per line)
python benchmarks/parity_test.py /path/to/repo --queries my_queries.txt

# JSON output for CI
python benchmarks/parity_test.py /path/to/repo --json
```

**Metrics reported:**
- File overlap (Jaccard similarity of file paths in results)
- Rank correlation (positional agreement for shared results)
- Divergent queries (overlap < 50%)

**Target:** ≥90% average overlap. Exit code 1 if target not met.

## Token Efficiency

Measures how many tokens sonar's search snippets save compared to reading the full source files.

```bash
python benchmarks/token_efficiency.py /path/to/repo
python benchmarks/token_efficiency.py /path/to/repo --top-k 10 --json
```

**Metrics reported:**
- Snippet tokens vs full-file tokens per query
- Overall savings ratio and compression ratio

## Custom Query Files

Both scripts accept `--queries path/to/file.txt`. Format: one query per line, blank lines and `#` comments are ignored.

```text
# Symbol lookups
getUserById
BM25Index
SonarIndex

# Natural language
how does search work
parse config file
error handling strategy
```
