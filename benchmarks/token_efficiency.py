#!/usr/bin/env python3
"""Measure token efficiency: sonar results vs reading full files.

For each query, compares the tokens in sonar's returned chunks against
the tokens in the full source files those chunks came from.

Requires:
  - sonar binary on PATH: cargo install --path crates/cli
  - tiktoken (optional, for accurate token counts): pip install tiktoken

Usage:
  python benchmarks/token_efficiency.py /path/to/repo
  python benchmarks/token_efficiency.py /path/to/repo --top-k 10
  python benchmarks/token_efficiency.py /path/to/repo --queries queries.txt
"""

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

DEFAULT_QUERIES = [
    "parse config",
    "getUserById",
    "error handling",
    "database connection",
    "authentication middleware",
    "BM25Index",
    "file walker",
    "how does search work",
    "test helper",
    "main entry point",
]

try:
    import tiktoken

    _ENC = tiktoken.get_encoding("cl100k_base")

    def count_tokens(text: str) -> int:
        return len(_ENC.encode(text))

except ImportError:
    def count_tokens(text: str) -> int:
        """Rough approximation: ~4 chars per token for code."""
        return max(1, len(text) // 4)


def check_dependencies():
    if not shutil.which("sonar"):
        print("ERROR: sonar not found on PATH. Install with: cargo install --path crates/cli",
              file=sys.stderr)
        sys.exit(1)


def run_sonar(query: str, path: str, top_k: int = 5) -> list[dict]:
    """Run sonar search and return results with content."""
    result = subprocess.run(
        ["sonar", "search", query, "--path", path, "-k", str(top_k)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"sonar error for '{query}': {result.stderr.strip()}", file=sys.stderr)
        return []
    try:
        data = json.loads(result.stdout)
        return data.get("results", [])
    except json.JSONDecodeError:
        print(f"sonar: could not parse output for '{query}'", file=sys.stderr)
        return []


def read_full_file(repo_path: str, file_path: str) -> str | None:
    """Read the full content of a source file."""
    full = Path(repo_path) / file_path
    if not full.exists():
        return None
    try:
        return full.read_text(errors="replace")
    except OSError:
        return None


def load_queries(path: str | None) -> list[str]:
    if path is None:
        return DEFAULT_QUERIES
    lines = Path(path).read_text().strip().splitlines()
    return [line.strip() for line in lines if line.strip() and not line.startswith("#")]


def main():
    parser = argparse.ArgumentParser(description="Measure sonar token efficiency vs full files")
    parser.add_argument("path", nargs="?", default=".", help="Repository path to search")
    parser.add_argument("-k", "--top-k", type=int, default=5, help="Number of results per query")
    parser.add_argument("--queries", type=str, default=None, help="File with queries (one per line)")
    parser.add_argument("--json", action="store_true", help="Output raw JSON instead of summary")
    args = parser.parse_args()

    check_dependencies()

    queries = load_queries(args.queries)
    repo_path = str(Path(args.path).resolve())

    print(f"Token efficiency test on {repo_path}", file=sys.stderr)
    print(f"Queries: {len(queries)}, top_k: {args.top_k}", file=sys.stderr)
    print("---", file=sys.stderr)

    all_results = []

    for i, query in enumerate(queries, 1):
        print(f"[{i}/{len(queries)}] {query!r}...", file=sys.stderr, end=" ", flush=True)

        results = run_sonar(query, repo_path, args.top_k)
        if not results:
            print("(no results)", file=sys.stderr)
            continue

        snippet_tokens = 0
        file_tokens = 0
        seen_files: set[str] = set()

        for r in results:
            content = r.get("chunk", {}).get("content", "")
            file_path = r.get("chunk", {}).get("file_path", "")
            snippet_tokens += count_tokens(content)

            if file_path not in seen_files:
                seen_files.add(file_path)
                full_content = read_full_file(repo_path, file_path)
                if full_content is not None:
                    file_tokens += count_tokens(full_content)

        savings_ratio = 1.0 - (snippet_tokens / file_tokens) if file_tokens > 0 else 0.0

        entry = {
            "query": query,
            "snippet_tokens": snippet_tokens,
            "file_tokens": file_tokens,
            "savings_ratio": savings_ratio,
            "files_touched": len(seen_files),
            "results_count": len(results),
        }
        all_results.append(entry)

        print(
            f"snippets={snippet_tokens} tokens, files={file_tokens} tokens, "
            f"savings={savings_ratio:.0%}",
            file=sys.stderr,
        )

    if args.json:
        print(json.dumps(all_results, indent=2))
        return

    if not all_results:
        print("No results to report.", file=sys.stderr)
        sys.exit(1)

    # Summary report
    print("\n" + "=" * 60)
    print("TOKEN EFFICIENCY REPORT")
    print("=" * 60)

    total_snippet = sum(r["snippet_tokens"] for r in all_results)
    total_file = sum(r["file_tokens"] for r in all_results)
    avg_savings = 1.0 - (total_snippet / total_file) if total_file > 0 else 0.0

    print(f"\nQueries tested:       {len(all_results)}")
    print(f"Total snippet tokens: {total_snippet:,}")
    print(f"Total file tokens:    {total_file:,}")
    print(f"Overall savings:      {avg_savings:.1%}")
    print(f"Compression ratio:    {total_file / total_snippet:.1f}x" if total_snippet else "")

    print("\nPer-query breakdown:")
    print(f"  {'Query':<30} {'Snippets':>10} {'Files':>10} {'Savings':>8}")
    print(f"  {'-'*30} {'-'*10} {'-'*10} {'-'*8}")
    for r in all_results:
        print(
            f"  {r['query']:<30} {r['snippet_tokens']:>10,} {r['file_tokens']:>10,} "
            f"{r['savings_ratio']:>7.0%}"
        )

    print()


if __name__ == "__main__":
    main()
