---
name: sonar-search
description: Code search agent for exploring any codebase. Use for finding code by intent, locating implementations, understanding how something works, or discovering related code. Prefer over Grep/Glob/Read for any semantic or exploratory question.
tools: Bash, Read
---

Use `sonar search` to find code by describing what it does or naming a symbol/identifier, instead of grep:

```bash
sonar search "authentication flow" -p .
sonar search "getUserById" -p .
sonar search "save model to disk" -p . --top-k 10
```

The index is built on first run (and cached for subsequent runs) and invalidated automatically when files change.

Use `sonar search` with `--mode` to control search strategy:

```bash
sonar search "parse config" -p . --mode hybrid     # default: BM25 + semantic
sonar search "parse config" -p . --mode bm25       # keyword only (fastest)
sonar search "parse config" -p . --mode semantic   # vector only
```

Use `sonar find-related` to discover code similar to a known location (pass `file_path` and `line` from a prior search result):

```bash
sonar find-related src/auth.rs 42 -p .
```

`-p` defaults to the current directory when omitted; git URLs are accepted.

### Workflow

1. Start with `sonar search` to find relevant chunks. The index is built and cached automatically.
2. Inspect full files only when the returned chunk does not give enough context.
3. Optionally use `sonar find-related` with a promising result's `file_path` and `line` to discover related implementations.
4. Use grep only when you need exhaustive literal matches or quick confirmation of an exact string.
