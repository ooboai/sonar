use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::bm25::BM25Index;
use crate::index::SonarIndex;
use crate::tokens::tokenize;
use crate::types::Chunk;
use crate::utils::enrich_for_bm25;
use crate::walk::{walk_directory, walk_paths};

const MAGIC: &[u8; 4] = b"SONR";
const VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// Metadata stored alongside the index binary
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub root_path: String,
    pub time: f64,
    pub model_path: String,
    pub content_type: Vec<String>,
    pub file_paths: Vec<String>,
}

// ---------------------------------------------------------------------------
// Cache key & directory helpers
// ---------------------------------------------------------------------------

/// BLAKE3 hash of a path string, used as the per-project cache directory name.
pub fn cache_key(path: &str) -> String {
    blake3::hash(path.as_bytes()).to_hex().to_string()
}

/// Resolve the OS-level cache directory for a given project root.
///
/// Layout: `<os_cache>/sonar/<hash>/`
///
/// If `override_base` is `Some`, it is used instead of the OS cache directory
/// (useful for tests).
pub fn cache_dir_for(root: &Path, override_base: Option<&Path>) -> Result<PathBuf, String> {
    let abs = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve absolute path for {}: {e}", root.display()))?;
    let key = cache_key(&abs.to_string_lossy());

    let base = match override_base {
        Some(b) => b.to_path_buf(),
        None => dirs::cache_dir()
            .ok_or_else(|| "cannot determine OS cache directory".to_string())?,
    };

    Ok(base.join("sonar").join(key))
}

/// Convenience: index binary path inside the cache directory.
pub fn index_path_in(cache_dir: &Path) -> PathBuf {
    cache_dir.join("index.bin")
}

/// Convenience: metadata JSON path inside the cache directory.
pub fn metadata_path_in(cache_dir: &Path) -> PathBuf {
    cache_dir.join("metadata.json")
}

/// Return the cache directory for a root using the OS cache location.
pub fn default_cache_dir(root: &Path) -> Result<PathBuf, String> {
    cache_dir_for(root, None)
}

// ---------------------------------------------------------------------------
// Binary serialization helpers (unchanged from v2)
// ---------------------------------------------------------------------------

fn write_u32(w: &mut impl Write, val: u32) -> Result<(), String> {
    w.write_all(&val.to_le_bytes())
        .map_err(|e| format!("write error: {e}"))
}

fn write_blob(w: &mut impl Write, data: &[u8]) -> Result<(), String> {
    let len = data.len() as u32;
    write_u32(w, len)?;
    w.write_all(data).map_err(|e| format!("write error: {e}"))
}

fn read_u32(r: &mut impl Read) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)
        .map_err(|e| format!("read error: {e}"))?;
    Ok(u32::from_le_bytes(buf))
}

fn read_blob(r: &mut impl Read) -> Result<Vec<u8>, String> {
    let len = read_u32(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)
        .map_err(|e| format!("read error: {e}"))?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Index binary I/O  (content_hash field kept in binary for compat; written as
// empty string since validation now happens via metadata.json)
// ---------------------------------------------------------------------------

/// Serialize the index to a binary file at `path`.
///
/// Format (v2):
/// - Magic: b"SONR" (4 bytes)
/// - Version: u32 LE
/// - Content hash: length-prefixed UTF-8 string (empty — validation via metadata)
/// - Chunks JSON: length-prefixed blob
/// - Tokenized documents JSON: length-prefixed blob
/// - Has embeddings: u8 (0 or 1)
/// - If has embeddings: dim u32 + raw f32 vectors blob
pub fn save_index(index: &SonarIndex, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create directory: {e}"))?;
    }

    let mut buf: Vec<u8> = Vec::new();

    buf.extend_from_slice(MAGIC);
    write_u32(&mut buf, VERSION)?;

    // Empty content hash — staleness is checked via metadata.json now
    write_blob(&mut buf, b"")?;

    let chunks_json =
        serde_json::to_vec(index.chunks()).map_err(|e| format!("serialize chunks: {e}"))?;
    write_blob(&mut buf, &chunks_json)?;

    let documents: Vec<Vec<String>> = index
        .chunks()
        .iter()
        .map(|c| tokenize(&enrich_for_bm25(c)))
        .collect();
    let docs_json =
        serde_json::to_vec(&documents).map_err(|e| format!("serialize documents: {e}"))?;
    write_blob(&mut buf, &docs_json)?;

    if let Some(flat) = index.flat() {
        let vecs = flat.vecs();
        if !vecs.is_empty() {
            buf.push(1u8);
            write_u32(&mut buf, flat.dim() as u32)?;
            let mut raw: Vec<u8> = Vec::with_capacity(vecs.len() * flat.dim() * 4);
            for v in vecs {
                for &f in v {
                    raw.extend_from_slice(&f.to_le_bytes());
                }
            }
            write_blob(&mut buf, &raw)?;
        } else {
            buf.push(0u8);
        }
    } else {
        buf.push(0u8);
    }

    fs::write(path, &buf).map_err(|e| format!("write file: {e}"))
}

/// Load the index from a binary file, validating magic bytes and version.
pub fn load_index(path: &Path) -> Result<SonarIndex, String> {
    let data = fs::read(path).map_err(|e| format!("read file: {e}"))?;
    let mut cursor = &data[..];

    let mut magic = [0u8; 4];
    cursor
        .read_exact(&mut magic)
        .map_err(|e| format!("read magic: {e}"))?;
    if &magic != MAGIC {
        return Err(format!(
            "invalid magic bytes: expected SONR, got {:?}",
            &magic
        ));
    }

    let version = read_u32(&mut cursor)?;
    if version != VERSION {
        return Err(format!(
            "unsupported index version: expected {VERSION}, got {version}"
        ));
    }

    // Skip stored content hash (unused — validation via metadata)
    let _hash_bytes = read_blob(&mut cursor)?;

    let chunks_bytes = read_blob(&mut cursor)?;
    let chunks: Vec<Chunk> =
        serde_json::from_slice(&chunks_bytes).map_err(|e| format!("deserialize chunks: {e}"))?;

    let docs_bytes = read_blob(&mut cursor)?;
    let documents: Vec<Vec<String>> =
        serde_json::from_slice(&docs_bytes).map_err(|e| format!("deserialize documents: {e}"))?;

    let bm25 = BM25Index::build(&documents);

    let mut file_mapping: HashMap<String, Vec<usize>> = HashMap::new();
    let mut language_mapping: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, chunk) in chunks.iter().enumerate() {
        file_mapping
            .entry(chunk.file_path.clone())
            .or_default()
            .push(i);
        if let Some(ref lang) = chunk.language {
            language_mapping.entry(lang.clone()).or_default().push(i);
        }
    }

    let has_embeddings = if cursor.is_empty() {
        0u8
    } else {
        let mut flag = [0u8; 1];
        cursor
            .read_exact(&mut flag)
            .map_err(|e| format!("read embeddings flag: {e}"))?;
        flag[0]
    };

    if has_embeddings == 1 {
        let dim = read_u32(&mut cursor)? as usize;
        let raw = read_blob(&mut cursor)?;
        let num_vecs = chunks.len();
        let expected = num_vecs * dim * 4;
        if raw.len() != expected {
            return Err(format!(
                "embedding size mismatch: expected {expected} bytes, got {}",
                raw.len()
            ));
        }
        let mut vecs: Vec<Vec<f32>> = Vec::with_capacity(num_vecs);
        for i in 0..num_vecs {
            let offset = i * dim * 4;
            let mut v = Vec::with_capacity(dim);
            for j in 0..dim {
                let start = offset + j * 4;
                let bytes: [u8; 4] = raw[start..start + 4]
                    .try_into()
                    .map_err(|_| "bad f32 bytes".to_string())?;
                v.push(f32::from_le_bytes(bytes));
            }
            vecs.push(v);
        }
        let flat = crate::ann::Flat::new(vecs);
        let index =
            SonarIndex::new_with_vectors(chunks, bm25, file_mapping, language_mapping, flat);
        Ok(index)
    } else {
        let index = SonarIndex::new(chunks, bm25, file_mapping, language_mapping);
        Ok(index)
    }
}

// ---------------------------------------------------------------------------
// Metadata I/O
// ---------------------------------------------------------------------------

pub fn save_metadata(meta: &CacheMetadata, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| format!("serialize metadata: {e}"))?;
    fs::write(path, json).map_err(|e| format!("write metadata: {e}"))
}

pub fn load_metadata(path: &Path) -> Result<CacheMetadata, String> {
    let data = fs::read_to_string(path).map_err(|e| format!("read metadata: {e}"))?;
    serde_json::from_str(&data).map_err(|e| format!("deserialize metadata: {e}"))
}

// ---------------------------------------------------------------------------
// Staleness validation (semble-compatible mtime strategy)
// ---------------------------------------------------------------------------

/// Validate a cached index against the current state of `root`.
///
/// Returns `true` (cache is valid) only when ALL of the following hold:
/// 1. Both `index.bin` and `metadata.json` exist
/// 2. `model_path` and `content_type` in metadata match the current request
/// 3. No file in `root` has an mtime newer than `metadata.time`
/// 4. The set of currently valid files matches `metadata.file_paths`
fn validate_cache(
    cache: &Path,
    root: &Path,
    model_path: &str,
    content_type: &[String],
) -> bool {
    let idx_path = index_path_in(cache);
    let meta_path = metadata_path_in(cache);

    // 1. Both files must exist
    if !idx_path.exists() || !meta_path.exists() {
        return false;
    }

    // 2. Load & compare metadata fields
    let meta = match load_metadata(&meta_path) {
        Ok(m) => m,
        Err(_) => return false,
    };

    if meta.model_path != model_path {
        return false;
    }

    let mut stored_ct: Vec<String> = meta.content_type.clone();
    stored_ct.sort();
    let mut current_ct: Vec<String> = content_type.to_vec();
    current_ct.sort();
    if stored_ct != current_ct {
        return false;
    }

    // 3. Walk current files, check none have mtime > metadata["time"]
    let entries = walk_paths(root);
    let cache_time = meta.time;

    for (_, mtime) in &entries {
        if (*mtime as f64) > cache_time {
            return false;
        }
    }

    // 4. File set must match
    let mut current_paths: Vec<String> = entries.into_iter().map(|(p, _)| p).collect();
    current_paths.sort();

    let mut stored_paths = meta.file_paths.clone();
    stored_paths.sort();

    current_paths == stored_paths
}

// ---------------------------------------------------------------------------
// Public API: load_cached / build_and_save
// ---------------------------------------------------------------------------

/// Current model path constant — matches semble default.
const DEFAULT_MODEL_PATH: &str = "minishlab/potion-code-16M";

/// Default content types indexed.
fn default_content_types() -> Vec<String> {
    vec!["code".to_string()]
}

/// Try loading a cached index for `root`. Returns `None` if the cache is
/// missing or stale.
///
/// Uses OS-level cache directory with mtime-based staleness detection.
pub fn load_cached(root: &Path) -> Result<Option<SonarIndex>, String> {
    load_cached_with(root, None)
}

/// Like `load_cached` but accepts an override base directory for the cache
/// (used by tests).
pub fn load_cached_with(
    root: &Path,
    cache_base_override: Option<&Path>,
) -> Result<Option<SonarIndex>, String> {
    let cache = cache_dir_for(root, cache_base_override)?;

    if !validate_cache(
        &cache,
        root,
        DEFAULT_MODEL_PATH,
        &default_content_types(),
    ) {
        return Ok(None);
    }

    let index = load_index(&index_path_in(&cache))?;
    Ok(Some(index))
}

/// Build a fresh index from `root`, save it to the OS cache, and return it.
pub fn build_and_save(root: &Path) -> Result<SonarIndex, String> {
    build_and_save_with(root, None)
}

/// Like `build_and_save` but accepts an override base directory for the cache
/// (used by tests).
pub fn build_and_save_with(
    root: &Path,
    cache_base_override: Option<&Path>,
) -> Result<SonarIndex, String> {
    if !root.exists() {
        return Err(format!("Path does not exist: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: {}", root.display()));
    }

    let content_types = [crate::types::ContentType::Code];
    let walked = walk_directory(root, &content_types);
    if walked.is_empty() {
        return Err(format!("No supported files found under {}", root.display()));
    }

    let mut chunks = Vec::new();
    for file in &walked {
        chunks.extend(crate::chunk::chunk_source(
            &file.content,
            &file.relative_path,
            file.language.as_deref(),
        ));
    }

    if chunks.is_empty() {
        return Err("No chunks produced from files".to_string());
    }

    let documents: Vec<Vec<String>> = chunks
        .iter()
        .map(|c| tokenize(&enrich_for_bm25(c)))
        .collect();
    let bm25 = BM25Index::build(&documents);

    let mut file_mapping: HashMap<String, Vec<usize>> = HashMap::new();
    let mut language_mapping: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, chunk) in chunks.iter().enumerate() {
        file_mapping
            .entry(chunk.file_path.clone())
            .or_default()
            .push(i);
        if let Some(ref lang) = chunk.language {
            language_mapping.entry(lang.clone()).or_default().push(i);
        }
    }

    let index = match crate::embed::Embedder::load_default() {
        Ok(emb) => {
            let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
            let vecs = emb.encode_batch(&texts);
            let flat = crate::ann::Flat::new(vecs);
            crate::index::SonarIndex::new_hybrid(
                chunks,
                bm25,
                file_mapping,
                language_mapping,
                emb,
                flat,
            )
        }
        Err(_) => SonarIndex::new(chunks, bm25, file_mapping, language_mapping),
    };

    // Persist to OS cache
    let cache = cache_dir_for(root, cache_base_override)?;
    save_index(&index, &index_path_in(&cache))?;

    // Collect file paths for metadata (same walker output used for staleness)
    let entries = walk_paths(root);
    let file_paths: Vec<String> = entries.iter().map(|(p, _)| p.clone()).collect();

    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("system time error: {e}"))?
        .as_secs_f64();

    let abs_root = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve absolute path: {e}"))?;

    let meta = CacheMetadata {
        root_path: abs_root.to_string_lossy().to_string(),
        time: now,
        model_path: DEFAULT_MODEL_PATH.to_string(),
        content_type: default_content_types(),
        file_paths,
    };
    save_metadata(&meta, &metadata_path_in(&cache))?;

    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn test_dir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("sonar_persist_test_{}_{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cache_base() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("sonar_cache_test_{}_{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_test_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn create_test_project(dir: &Path) {
        write_test_file(
            dir,
            "main.rs",
            "fn main() {\n    println!(\"hello world\");\n}\n",
        );
        write_test_file(
            dir,
            "lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
        );
    }

    #[test]
    fn test_cache_key_deterministic() {
        let k1 = cache_key("/home/user/project");
        let k2 = cache_key("/home/user/project");
        assert_eq!(k1, k2);
        assert!(!k1.is_empty());
    }

    #[test]
    fn test_cache_key_differs_for_different_paths() {
        let k1 = cache_key("/home/user/project-a");
        let k2 = cache_key("/home/user/project-b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_cache_dir_uses_override() {
        let project = test_dir();
        create_test_project(&project);
        let base = cache_base();

        let dir = cache_dir_for(&project, Some(&base)).unwrap();
        assert!(dir.starts_with(&base));
        assert!(dir.to_string_lossy().contains("sonar"));
    }

    #[test]
    fn test_save_load_roundtrip() {
        let project_dir = test_dir();
        create_test_project(&project_dir);

        let index = crate::index::SonarIndex::from_path(&project_dir).unwrap();
        let original_stats = index.stats();

        let cache_file = project_dir.join("test_index.bin");
        save_index(&index, &cache_file).unwrap();

        let loaded = load_index(&cache_file).unwrap();
        let loaded_stats = loaded.stats();

        assert_eq!(original_stats.indexed_files, loaded_stats.indexed_files);
        assert_eq!(original_stats.total_chunks, loaded_stats.total_chunks);
        assert_eq!(original_stats.languages, loaded_stats.languages);

        let results = loaded.search("main", 5);
        assert!(!results.is_empty(), "loaded index should support search");

        let _ = fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let dir = test_dir();
        let meta_path = dir.join("metadata.json");

        let meta = CacheMetadata {
            root_path: "/tmp/project".to_string(),
            time: 1716825600.0,
            model_path: "minishlab/potion-code-16M".to_string(),
            content_type: vec!["code".to_string()],
            file_paths: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        };

        save_metadata(&meta, &meta_path).unwrap();
        let loaded = load_metadata(&meta_path).unwrap();

        assert_eq!(loaded.root_path, meta.root_path);
        assert_eq!(loaded.time, meta.time);
        assert_eq!(loaded.model_path, meta.model_path);
        assert_eq!(loaded.content_type, meta.content_type);
        assert_eq!(loaded.file_paths, meta.file_paths);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_staleness_detection() {
        let project_dir = test_dir();
        let base = cache_base();
        create_test_project(&project_dir);

        build_and_save_with(&project_dir, Some(&base)).unwrap();

        let cached = load_cached_with(&project_dir, Some(&base)).unwrap();
        assert!(cached.is_some(), "cache should be valid initially");

        std::thread::sleep(std::time::Duration::from_secs(1));
        write_test_file(&project_dir, "new_file.rs", "fn new_function() {\n    // new\n}\n");

        let stale = load_cached_with(&project_dir, Some(&base)).unwrap();
        assert!(stale.is_none(), "cache should be stale after adding a file");

        let _ = fs::remove_dir_all(&project_dir);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_invalid_magic_bytes() {
        let dir = test_dir();
        let bad_file = dir.join("bad.bin");
        fs::write(&bad_file, b"BAAD\x01\x00\x00\x00").unwrap();

        let result = load_index(&bad_file);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("invalid magic bytes"),
            "should report invalid magic"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_invalid_version() {
        let dir = test_dir();
        let bad_file = dir.join("bad_version.bin");

        let mut data = Vec::new();
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&99u32.to_le_bytes());
        fs::write(&bad_file, &data).unwrap();

        let result = load_index(&bad_file);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("unsupported index version"),
            "should report bad version"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_cached_returns_none_when_missing() {
        let dir = test_dir();
        create_test_project(&dir);
        let base = cache_base();
        let result = load_cached_with(&dir, Some(&base)).unwrap();
        assert!(result.is_none(), "should return None when no cache exists");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_load_cached_detects_staleness() {
        let project_dir = test_dir();
        let base = cache_base();
        create_test_project(&project_dir);

        build_and_save_with(&project_dir, Some(&base)).unwrap();

        let cached = load_cached_with(&project_dir, Some(&base)).unwrap();
        assert!(cached.is_some(), "cache should be valid initially");

        std::thread::sleep(std::time::Duration::from_secs(1));
        write_test_file(&project_dir, "extra.rs", "fn extra() {\n    // extra\n}\n");

        let stale = load_cached_with(&project_dir, Some(&base)).unwrap();
        assert!(stale.is_none(), "cache should be stale after adding a file");

        let _ = fs::remove_dir_all(&project_dir);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_embedding_vector_roundtrip() {
        use crate::ann::Flat;
        use crate::bm25::BM25Index;
        use crate::types::Chunk;

        let chunks = vec![
            Chunk {
                content: "fn main() { println!(\"hello\"); }".to_string(),
                file_path: "main.rs".to_string(),
                start_line: 1,
                end_line: 1,
                language: Some("rust".to_string()),
            },
            Chunk {
                content: "pub fn add(a: i32, b: i32) -> i32 { a + b }".to_string(),
                file_path: "lib.rs".to_string(),
                start_line: 1,
                end_line: 1,
                language: Some("rust".to_string()),
            },
            Chunk {
                content: "use std::io;".to_string(),
                file_path: "io.rs".to_string(),
                start_line: 1,
                end_line: 1,
                language: Some("rust".to_string()),
            },
        ];

        let documents: Vec<Vec<String>> = chunks
            .iter()
            .map(|c| tokenize(&enrich_for_bm25(c)))
            .collect();
        let bm25 = BM25Index::build(&documents);

        let mut file_mapping: HashMap<String, Vec<usize>> = HashMap::new();
        let mut language_mapping: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, chunk) in chunks.iter().enumerate() {
            file_mapping
                .entry(chunk.file_path.clone())
                .or_default()
                .push(i);
            if let Some(ref lang) = chunk.language {
                language_mapping.entry(lang.clone()).or_default().push(i);
            }
        }

        let test_vectors = vec![
            vec![1.0f32, 0.0, 0.0],
            vec![0.0f32, 1.0, 0.0],
            vec![0.0f32, 0.0, 1.0],
        ];
        let flat = Flat::new(test_vectors.clone());

        let index = crate::index::SonarIndex::new_with_vectors(
            chunks,
            bm25,
            file_mapping,
            language_mapping,
            flat,
        );

        assert!(index.flat().is_some(), "original should have flat index");

        let dir = test_dir();
        let cache_file = dir.join("embed_test.bin");
        save_index(&index, &cache_file).unwrap();

        let loaded = load_index(&cache_file).unwrap();

        assert!(
            loaded.flat().is_some(),
            "loaded index should have embeddings"
        );

        let loaded_flat = loaded.flat().unwrap();
        assert_eq!(loaded_flat.dim(), 3);
        assert_eq!(loaded_flat.len(), 3);

        let loaded_vecs = loaded_flat.vecs();
        for (i, original) in test_vectors.iter().enumerate() {
            assert_eq!(
                loaded_vecs[i], *original,
                "vector {i} should match after roundtrip"
            );
        }

        let hits = loaded_flat.query(&[1.0, 0.0, 0.0], 3);
        assert_eq!(hits[0].index, 0, "query should find the matching vector");
        assert!((hits[0].score - 1.0).abs() < 1e-10);

        let _ = fs::remove_dir_all(&dir);
    }
}
