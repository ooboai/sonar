#!/usr/bin/env python3
"""Compare sonar vs semble search results for parity testing.

Requires:
  - semble installed: pip install semble
  - sonar binary on PATH: cargo install --path crates/cli

Usage:
  python benchmarks/parity_test.py /path/to/repo
  python benchmarks/parity_test.py /path/to/repo --top-k 10
  python benchmarks/parity_test.py /path/to/repo --queries queries.txt
"""

import argparse
import json
import re
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


def check_dependencies():
    """Verify semble and sonar are available."""
    errors = []
    if not shutil.which("sonar"):
        errors.append("sonar not found on PATH. Install with: cargo install --path crates/cli")
    try:
        subprocess.run(
            ["python3", "-c", "import semble"],
            capture_output=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        errors.append("semble not installed. Install with: pip install semble")
    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)


def run_semble(query: str, path: str, top_k: int = 5) -> list[dict]:
    """Run semble search and parse results into a list of dicts."""
    result = subprocess.run(
        [
            "python3",
            "-c",
            f"""
import json, semble
idx = semble.SembleIndex.from_path("{path}")
results = idx.search("{query}", top_k={top_k})
out = []
for r in results:
    out.append({{
        "file_path": r.chunk.file_path,
        "start_line": r.chunk.start_line,
        "end_line": r.chunk.end_line,
        "score": r.score,
    }})
print(json.dumps(out))
""",
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"semble error for '{query}': {result.stderr.strip()}", file=sys.stderr)
        return []
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        print(f"semble: could not parse output for '{query}'", file=sys.stderr)
        return []


def run_sonar(query: str, path: str, top_k: int = 5) -> list[dict]:
    """Run sonar search and parse JSON results."""
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
        return [
            {
                "file_path": r["chunk"]["file_path"],
                "start_line": r["chunk"]["start_line"],
                "end_line": r["chunk"]["end_line"],
                "score": r["score"],
            }
            for r in data.get("results", [])
        ]
    except (json.JSONDecodeError, KeyError) as e:
        print(f"sonar: could not parse output for '{query}': {e}", file=sys.stderr)
        return []


def file_overlap(semble_results: list[dict], sonar_results: list[dict]) -> float:
    """Fraction of file_paths that appear in both result sets."""
    semble_files = {r["file_path"] for r in semble_results}
    sonar_files = {r["file_path"] for r in sonar_results}
    if not semble_files and not sonar_files:
        return 1.0
    if not semble_files or not sonar_files:
        return 0.0
    intersection = semble_files & sonar_files
    union = semble_files | sonar_files
    return len(intersection) / len(union)


def rank_correlation(semble_results: list[dict], sonar_results: list[dict]) -> float:
    """Compute a simple rank-based correlation (Kendall-like).

    For each file that appears in both result lists, compare its rank position.
    Returns a score from 0.0 (no correlation) to 1.0 (identical ranking).
    """
    semble_ranks = {r["file_path"]: i for i, r in enumerate(semble_results)}
    sonar_ranks = {r["file_path"]: i for i, r in enumerate(sonar_results)}
    common = set(semble_ranks) & set(sonar_ranks)
    if not common:
        return 0.0

    n = len(semble_results)
    total_displacement = sum(abs(semble_ranks[f] - sonar_ranks[f]) for f in common)
    max_displacement = n * len(common)
    if max_displacement == 0:
        return 1.0
    return 1.0 - (total_displacement / max_displacement)


def compare_results(
    query: str,
    semble_results: list[dict],
    sonar_results: list[dict],
) -> dict:
    """Compare two result sets and return metrics."""
    overlap = file_overlap(semble_results, sonar_results)
    correlation = rank_correlation(semble_results, sonar_results)

    semble_files = [r["file_path"] for r in semble_results]
    sonar_files = [r["file_path"] for r in sonar_results]
    only_semble = [f for f in semble_files if f not in set(sonar_files)]
    only_sonar = [f for f in sonar_files if f not in set(semble_files)]

    return {
        "query": query,
        "overlap": overlap,
        "rank_correlation": correlation,
        "semble_files": semble_files,
        "sonar_files": sonar_files,
        "only_in_semble": only_semble,
        "only_in_sonar": only_sonar,
    }


def load_queries(path: str | None) -> list[str]:
    """Load queries from a file (one per line) or use defaults."""
    if path is None:
        return DEFAULT_QUERIES
    lines = Path(path).read_text().strip().splitlines()
    return [line.strip() for line in lines if line.strip() and not line.startswith("#")]


def main():
    parser = argparse.ArgumentParser(description="Parity test: sonar vs semble")
    parser.add_argument("path", nargs="?", default=".", help="Repository path to search")
    parser.add_argument("-k", "--top-k", type=int, default=5, help="Number of results per query")
    parser.add_argument("--queries", type=str, default=None, help="File with queries (one per line)")
    parser.add_argument("--json", action="store_true", help="Output raw JSON instead of summary")
    args = parser.parse_args()

    check_dependencies()

    queries = load_queries(args.queries)
    repo_path = str(Path(args.path).resolve())

    print(f"Parity test: sonar vs semble on {repo_path}", file=sys.stderr)
    print(f"Queries: {len(queries)}, top_k: {args.top_k}", file=sys.stderr)
    print("---", file=sys.stderr)

    all_results = []
    high_overlap_count = 0

    for i, query in enumerate(queries, 1):
        print(f"[{i}/{len(queries)}] {query!r}...", file=sys.stderr, end=" ", flush=True)

        semble = run_semble(query, repo_path, args.top_k)
        sonar = run_sonar(query, repo_path, args.top_k)
        comparison = compare_results(query, semble, sonar)
        all_results.append(comparison)

        if comparison["overlap"] >= 0.8:
            high_overlap_count += 1

        status = "OK" if comparison["overlap"] >= 0.8 else "DIVERGENT"
        print(
            f"{status} (overlap={comparison['overlap']:.0%}, "
            f"rank_corr={comparison['rank_correlation']:.2f})",
            file=sys.stderr,
        )

    if args.json:
        print(json.dumps(all_results, indent=2))
        return

    # Summary report
    print("\n" + "=" * 60)
    print("PARITY TEST RESULTS")
    print("=" * 60)

    avg_overlap = sum(r["overlap"] for r in all_results) / len(all_results) if all_results else 0
    avg_corr = (
        sum(r["rank_correlation"] for r in all_results) / len(all_results) if all_results else 0
    )

    print(f"\nQueries tested:     {len(all_results)}")
    print(f"High overlap (≥80%): {high_overlap_count}/{len(all_results)}")
    print(f"Average overlap:     {avg_overlap:.1%}")
    print(f"Average rank corr:   {avg_corr:.2f}")

    target_met = avg_overlap >= 0.9
    print(f"\nTarget (≥90% avg overlap): {'PASS ✓' if target_met else 'FAIL ✗'}")

    divergent = [r for r in all_results if r["overlap"] < 0.5]
    if divergent:
        print(f"\nDivergent queries (overlap < 50%):")
        for r in divergent:
            print(f"  - {r['query']!r}: overlap={r['overlap']:.0%}")
            if r["only_in_semble"]:
                print(f"    only in semble: {r['only_in_semble']}")
            if r["only_in_sonar"]:
                print(f"    only in sonar:  {r['only_in_sonar']}")

    print()
    sys.exit(0 if target_met else 1)


if __name__ == "__main__":
    main()
