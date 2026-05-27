# sonar-search

Use the sonar MCP server for code search. Prefer `search` and `find_related` tools over Grep, Glob, or Read for any question about how code works.

## Setup

Add to your MCP configuration:

```json
{
  "mcpServers": {
    "sonar": {
      "command": "sonar-mcp"
    }
  }
}
```

## Tools

- **search** — Hybrid code search (BM25 + semantic). Pass a natural language query or code snippet.
- **find_related** — Find chunks semantically related to a given file and line range.

## Tips

- Use `search` for broad questions like "how does authentication work?" or "where is the database connection configured?"
- Use `find_related` when you have a specific function and want to find related code (callers, similar patterns, tests).
- Both tools return ranked code chunks with file paths and line numbers.
