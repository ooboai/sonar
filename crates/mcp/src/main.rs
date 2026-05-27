use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sonar_core::index::SonarIndex;
use sonar_core::types::SearchResult;
use sonar_core::utils::{format_results, resolve_chunk};

const SERVER_NAME: &str = "sonar";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

const SERVER_INSTRUCTIONS: &str = "\
Instant code search for any local or remote git repository. \
Call `search` to find relevant code; call `find_related` on a result \
to discover similar code elsewhere. When working in a local project, \
pass the project root as `repo`. Prefer sonar for conceptual queries, \
locating definitions, and surface-area explorations. For refactors or \
exhaustive operations, fall back to grep.";

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool schemas
// ---------------------------------------------------------------------------

fn search_tool_schema() -> Value {
    json!({
        "name": "search",
        "description": "Search a codebase with a natural-language or code query. Pass a git URL or local path as `repo` to index it on demand; indexes are cached for the session. Use this to find where something is implemented, understand a library, or locate related code.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Natural language or code query."},
                "repo": {"type": "string", "description": "Local directory path to index and search."},
                "top_k": {"type": "integer", "description": "Number of results to return.", "default": 5}
            },
            "required": ["query"]
        }
    })
}

fn find_related_tool_schema() -> Value {
    json!({
        "name": "find_related",
        "description": "Find code chunks semantically similar to a specific location in a file. Use after search to explore related implementations or callers.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "file_path": {"type": "string", "description": "Path to the file as stored in the index."},
                "line": {"type": "integer", "description": "Line number (1-indexed)."},
                "repo": {"type": "string", "description": "Local directory path."},
                "top_k": {"type": "integer", "description": "Number of similar chunks to return.", "default": 5}
            },
            "required": ["file_path", "line"]
        }
    })
}

// ---------------------------------------------------------------------------
// Result formatting (text output matching semble)
// ---------------------------------------------------------------------------

fn format_results_text(header: &str, results: &[SearchResult]) -> String {
    let mut out = String::new();
    out.push_str(header);
    out.push_str("\n\n");

    for (i, r) in results.iter().enumerate() {
        let lang_tag = r.chunk.language.as_deref().unwrap_or("");
        out.push_str(&format!(
            "## {}. {}:{}-{} [score={:.3}]\n```{}\n{}\n```\n\n",
            i + 1,
            r.chunk.file_path,
            r.chunk.start_line,
            r.chunk.end_line,
            r.score,
            lang_tag,
            r.chunk.content.trim_end(),
        ));
    }

    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

const MAX_CACHED_INDEXES: usize = 10;

struct IndexCache {
    indexes: HashMap<String, SonarIndex>,
    order: VecDeque<String>,
}

impl IndexCache {
    fn new() -> Self {
        Self {
            indexes: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn contains(&self, key: &str) -> bool {
        self.indexes.contains_key(key)
    }

    fn get(&self, key: &str) -> Option<&SonarIndex> {
        self.indexes.get(key)
    }

    fn insert(&mut self, key: String, index: SonarIndex) {
        if self.indexes.len() >= MAX_CACHED_INDEXES
            && let Some(oldest) = self.order.pop_front()
        {
            self.indexes.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.indexes.insert(key, index);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.indexes.len()
    }
}

struct Server {
    cache: Mutex<IndexCache>,
}

impl Server {
    fn new() -> Self {
        Self {
            cache: Mutex::new(IndexCache::new()),
        }
    }

    fn get_or_build_index(&self, repo: &str) -> Result<(), String> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| format!("Internal error: index lock poisoned: {e}"))?;
        if cache.contains(repo) {
            return Ok(());
        }
        eprintln!("sonar-mcp: indexing {repo}...");
        let index = SonarIndex::from_path_cached(Path::new(repo))?;
        let stats = index.stats();
        eprintln!(
            "sonar-mcp: indexed {} files, {} chunks",
            stats.indexed_files, stats.total_chunks
        );
        cache.insert(repo.to_string(), index);
        Ok(())
    }

    fn handle_search(&self, params: &Value) -> Value {
        let query = match params.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => {
                return tool_error("Missing required parameter: query");
            }
        };
        let top_k = params.get("top_k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        if let Err(e) = self.get_or_build_index(&repo) {
            return tool_error(&format!("Failed to index {repo}: {e}"));
        }

        let cache = match self.cache.lock() {
            Ok(c) => c,
            Err(_) => return tool_error("Internal error: index lock poisoned"),
        };
        let index = match cache.get(&repo) {
            Some(idx) => idx,
            None => return tool_error(&format!("Internal error: index missing for {repo}")),
        };
        let results = index.search(&query, top_k);

        if results.is_empty() {
            return tool_result(&format!("No results found for: \"{query}\""));
        }

        let json_output = format_results(&query, &results);
        tool_result(&serde_json::to_string(&json_output).unwrap_or_default())
    }

    fn handle_find_related(&self, params: &Value) -> Value {
        let file_path = match params.get("file_path").and_then(|v| v.as_str()) {
            Some(fp) => fp.to_string(),
            None => {
                return tool_error("Missing required parameter: file_path");
            }
        };
        let line = match params.get("line").and_then(|v| v.as_u64()) {
            Some(l) => l as usize,
            None => {
                return tool_error("Missing required parameter: line");
            }
        };
        let top_k = params.get("top_k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        if let Err(e) = self.get_or_build_index(&repo) {
            return tool_error(&format!("Failed to index {repo}: {e}"));
        }

        let cache = match self.cache.lock() {
            Ok(c) => c,
            Err(_) => return tool_error("Internal error: index lock poisoned"),
        };
        let index = match cache.get(&repo) {
            Some(idx) => idx,
            None => return tool_error(&format!("Internal error: index missing for {repo}")),
        };

        let chunk = match resolve_chunk(index.chunks(), &file_path, line) {
            Some(c) => c.clone(),
            None => {
                return tool_error(&format!(
                    "No chunk found at {file_path}:{line}. Check the file path and line number."
                ));
            }
        };

        let results = index.find_related(&chunk, top_k);

        if results.is_empty() {
            return tool_result(&format!(
                "No related chunks found for {file_path}:{line}. \
                 (Semantic search requires embeddings, which are not yet enabled.)"
            ));
        }

        let query_label = format!("{file_path}:{line}");
        let header = format!("Related to: {query_label}");
        let text = format_results_text(&header, &results);
        let json_output = format_results(&query_label, &results);
        json!({
            "content": [
                {"type": "text", "text": text},
                {"type": "text", "text": serde_json::to_string(&json_output).unwrap_or_default()}
            ]
        })
    }

    fn handle_request(&self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = match &req.id {
            Some(id) => id.clone(),
            None => return None, // notification — no response
        };

        if req.jsonrpc != "2.0" {
            return Some(JsonRpcResponse::error(
                id,
                -32600,
                "Invalid JSON-RPC version",
            ));
        }

        match req.method.as_str() {
            "initialize" => Some(JsonRpcResponse::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    },
                    "instructions": SERVER_INSTRUCTIONS
                }),
            )),

            "tools/list" => Some(JsonRpcResponse::success(
                id,
                json!({
                    "tools": [
                        search_tool_schema(),
                        find_related_tool_schema()
                    ]
                }),
            )),

            "tools/call" => {
                let params = req.params.as_ref().cloned().unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                let result = match tool_name {
                    "search" => self.handle_search(&arguments),
                    "find_related" => self.handle_find_related(&arguments),
                    _ => tool_error(&format!("Unknown tool: {tool_name}")),
                };

                Some(JsonRpcResponse::success(id, result))
            }

            _ => Some(JsonRpcResponse::error(
                id,
                -32601,
                format!("Method not found: {}", req.method),
            )),
        }
    }
}

fn tool_result(text: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": text}]
    })
}

fn tool_error(text: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": true
    })
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_target(false)
        .init();

    let server = Server::new();
    let stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("sonar-mcp: stdin read error: {e}");
                break;
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::error(Value::Null, -32700, format!("Parse error: {e}"));
                let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
                let _ = stdout.flush();
                continue;
            }
        };

        if let Some(resp) = server.handle_request(&req) {
            let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
            let _ = stdout.flush();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_request(json_str: &str) -> JsonRpcRequest {
        serde_json::from_str(json_str).expect("should parse JSON-RPC request")
    }

    // --- JSON-RPC parsing tests ---

    #[test]
    fn test_parse_initialize_request() {
        let req = parse_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
        );
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, Some(json!(1)));
        assert_eq!(req.method, "initialize");
    }

    #[test]
    fn test_parse_notification_no_id() {
        let req =
            parse_request(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#);
        assert!(req.id.is_none());
        assert_eq!(req.method, "notifications/initialized");
    }

    #[test]
    fn test_parse_tools_call_request() {
        let req = parse_request(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search","arguments":{"query":"main function","top_k":3}}}"#,
        );
        assert_eq!(req.method, "tools/call");
        let params = req.params.as_ref().unwrap();
        assert_eq!(params["name"], "search");
        assert_eq!(params["arguments"]["query"], "main function");
        assert_eq!(params["arguments"]["top_k"], 3);
    }

    #[test]
    fn test_parse_string_id() {
        let req =
            parse_request(r#"{"jsonrpc":"2.0","id":"abc-123","method":"tools/list","params":{}}"#);
        assert_eq!(req.id, Some(json!("abc-123")));
    }

    // --- Response serialization tests ---

    #[test]
    fn test_success_response_serialization() {
        let resp = JsonRpcResponse::success(json!(1), json!({"tools": []}));
        let serialized = serde_json::to_string(&resp).unwrap();
        let parsed: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert!(parsed.get("result").is_some());
        assert!(parsed.get("error").is_none());
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = JsonRpcResponse::error(json!(2), -32601, "Method not found");
        let serialized = serde_json::to_string(&resp).unwrap();
        let parsed: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed["error"]["code"], -32601);
        assert_eq!(parsed["error"]["message"], "Method not found");
        assert!(parsed.get("result").is_none());
    }

    // --- Result formatting tests ---

    #[test]
    fn test_format_results_text_empty() {
        let text = format_results_text("Search results for: \"test\"", &[]);
        assert_eq!(text, "Search results for: \"test\"");
    }

    #[test]
    fn test_format_results_text_single() {
        let results = vec![SearchResult {
            chunk: sonar_core::types::Chunk {
                content: "fn main() {\n    println!(\"hello\");\n}".to_string(),
                file_path: "src/main.rs".to_string(),
                start_line: 1,
                end_line: 3,
                language: Some("rust".to_string()),
            },
            score: 0.847,
        }];
        let text = format_results_text("Search results for: \"main\" (mode=hybrid)", &results);
        assert!(text.contains("## 1. src/main.rs:1-3 [score=0.847]"));
        assert!(text.contains("```rust"));
        assert!(text.contains("fn main()"));
        assert!(text.contains("```"));
    }

    #[test]
    fn test_format_results_text_multiple() {
        let results = vec![
            SearchResult {
                chunk: sonar_core::types::Chunk {
                    content: "fn main() {}".to_string(),
                    file_path: "src/main.rs".to_string(),
                    start_line: 1,
                    end_line: 1,
                    language: Some("rust".to_string()),
                },
                score: 0.9,
            },
            SearchResult {
                chunk: sonar_core::types::Chunk {
                    content: "fn helper() {}".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    start_line: 10,
                    end_line: 10,
                    language: None,
                },
                score: 0.7,
            },
        ];
        let text = format_results_text("Search results for: \"code\" (mode=hybrid)", &results);
        assert!(text.contains("## 1."));
        assert!(text.contains("## 2."));
        assert!(text.contains("[score=0.900]"));
        assert!(text.contains("[score=0.700]"));
    }

    #[test]
    fn test_format_no_language_tag() {
        let results = vec![SearchResult {
            chunk: sonar_core::types::Chunk {
                content: "key = value".to_string(),
                file_path: "config.toml".to_string(),
                start_line: 1,
                end_line: 1,
                language: None,
            },
            score: 0.5,
        }];
        let text = format_results_text("header", &results);
        assert!(text.contains("```\nkey = value\n```"));
    }

    // --- Tool result/error helpers ---

    #[test]
    fn test_tool_result_shape() {
        let r = tool_result("hello");
        assert_eq!(r["content"][0]["type"], "text");
        assert_eq!(r["content"][0]["text"], "hello");
        assert!(r.get("isError").is_none());
    }

    #[test]
    fn test_tool_error_shape() {
        let r = tool_error("something went wrong");
        assert_eq!(r["content"][0]["type"], "text");
        assert_eq!(r["content"][0]["text"], "something went wrong");
        assert_eq!(r["isError"], true);
    }

    // --- Server dispatch tests ---

    #[test]
    fn test_initialize_response() {
        let server = Server::new();
        let req = parse_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
        );
        let resp = server.handle_request(&req).expect("should respond");
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["instructions"].as_str().unwrap().contains("search"));
    }

    #[test]
    fn test_tools_list_response() {
        let server = Server::new();
        let req = parse_request(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let resp = server.handle_request(&req).expect("should respond");
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);

        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search"));
        assert!(names.contains(&"find_related"));

        let search = tools.iter().find(|t| t["name"] == "search").unwrap();
        let required = search["inputSchema"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("query")));
    }

    #[test]
    fn test_notification_returns_none() {
        let server = Server::new();
        let req =
            parse_request(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#);
        assert!(server.handle_request(&req).is_none());
    }

    #[test]
    fn test_unknown_method_returns_error() {
        let server = Server::new();
        let req = parse_request(r#"{"jsonrpc":"2.0","id":5,"method":"bogus/method"}"#);
        let resp = server.handle_request(&req).expect("should respond");
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32601);
    }

    #[test]
    fn test_search_missing_query_returns_error() {
        let server = Server::new();
        let req = parse_request(
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"search","arguments":{}}}"#,
        );
        let resp = server.handle_request(&req).expect("should respond");
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("query")
        );
    }

    #[test]
    fn test_find_related_missing_params_returns_error() {
        let server = Server::new();
        let req = parse_request(
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"find_related","arguments":{}}}"#,
        );
        let resp = server.handle_request(&req).expect("should respond");
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn test_unknown_tool_returns_error() {
        let server = Server::new();
        let req = parse_request(
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"nonexistent","arguments":{}}}"#,
        );
        let resp = server.handle_request(&req).expect("should respond");
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Unknown tool")
        );
    }

    #[test]
    fn test_invalid_jsonrpc_version() {
        let server = Server::new();
        let req = parse_request(r#"{"jsonrpc":"1.0","id":1,"method":"initialize"}"#);
        let resp = server.handle_request(&req).expect("should respond");
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32600);
    }

    // --- Integration test: full init + list flow ---

    #[test]
    fn test_full_init_and_list_flow() {
        let server = Server::new();

        // 1. initialize
        let init_req = parse_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1.0"}}}"#,
        );
        let init_resp = server.handle_request(&init_req).unwrap();
        assert!(init_resp.error.is_none());
        let init_result = init_resp.result.unwrap();
        assert_eq!(init_result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(init_result["serverInfo"]["name"], "sonar");

        // 2. notifications/initialized (no response expected)
        let notif =
            parse_request(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#);
        assert!(server.handle_request(&notif).is_none());

        // 3. tools/list
        let list_req = parse_request(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let list_resp = server.handle_request(&list_req).unwrap();
        assert!(list_resp.error.is_none());
        let list_result = list_resp.result.unwrap();
        let tools = list_result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);

        let search_tool = tools.iter().find(|t| t["name"] == "search").unwrap();
        assert!(
            search_tool["description"]
                .as_str()
                .unwrap()
                .contains("Search a codebase")
        );
        let search_props = &search_tool["inputSchema"]["properties"];
        assert!(search_props.get("query").is_some());
        assert!(search_props.get("repo").is_some());
        assert!(search_props.get("top_k").is_some());

        let related_tool = tools.iter().find(|t| t["name"] == "find_related").unwrap();
        assert!(
            related_tool["description"]
                .as_str()
                .unwrap()
                .contains("semantically similar")
        );
        let related_required = related_tool["inputSchema"]["required"].as_array().unwrap();
        assert!(related_required.contains(&json!("file_path")));
        assert!(related_required.contains(&json!("line")));
    }

    // --- Integration test: search against the sonar repo ---

    #[test]
    fn test_search_tool_against_self() {
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let server = Server::new();
        let req_str = format!(
            r#"{{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{{"name":"search","arguments":{{"query":"SonarIndex","repo":"{}","top_k":3}}}}}}"#,
            repo_root.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let req = parse_request(&req_str);
        let resp = server.handle_request(&req).unwrap();
        let result = resp.result.unwrap();
        assert!(result.get("isError").is_none());
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).expect("search output should be valid JSON");
        assert_eq!(parsed["query"], "SonarIndex");
        assert!(parsed["results"].as_array().unwrap().len() > 0);
        assert!(parsed["results"][0].get("score").is_some());
    }

    // --- Integration test: find_related returns stub message ---

    #[test]
    fn test_find_related_tool_against_self() {
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let server = Server::new();
        let req_str = format!(
            r#"{{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{{"name":"find_related","arguments":{{"file_path":"crates/core/src/index.rs","line":14,"repo":"{}"}}}}}}"#,
            repo_root.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let req = parse_request(&req_str);
        let resp = server.handle_request(&req).unwrap();
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No related chunks found") || text.contains("Related to:"));
    }

    // --- Index caching test ---

    #[test]
    fn test_index_is_cached() {
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let server = Server::new();
        server.get_or_build_index(&repo_root).unwrap();
        assert_eq!(server.cache.lock().unwrap().len(), 1);

        // Second call should not fail and cache should still have 1 entry
        server.get_or_build_index(&repo_root).unwrap();
        assert_eq!(server.cache.lock().unwrap().len(), 1);
    }
}
