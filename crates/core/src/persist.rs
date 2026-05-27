use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use crate::bm25::BM25Index;
use crate::index::SonarIndex;
use crate::tokens::tokenize;
use crate::types::Chunk;
use crate::utils::enrich_for_bm25;
use crate::walk::walk_directory;

const MAGIC: &[u8; 4] = b"SONR";
const VERSION: u32 = 2;

/// Compute a BLAKE3 hash of the file listing (sorted paths + modification times).
/// Used to detect whether the index is stale.
pub fn compute_content_hash(root: &Path) -> Result<String, String> {
    let mut entries: Vec<(String, u64)> = Vec::new();

    const IGNORED_DIRS: &[&str] = &[
        ".git", ".hg", ".svn", "node_modules", "__pycache__", ".tox",
        "target", "build", "dist", ".cache", ".eggs", "venv", ".venv",
        ".mypy_cache", ".pytest_cache", ".ruff_cache",
    ];

    let walker = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_str().unwrap_or("");
        if e.file_type().is_dir() {
            let is_hidden = name.starts_with('.') && name != ".";
            !is_hidden && !IGNORED_DIRS.contains(&name)
        } else {
            true
        }
    });

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push((rel, mtime));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = blake3::Hasher::new();
    for (path, mtime) in &entries {
        hasher.update(path.as_bytes());
        hasher.update(&mtime.to_le_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

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

/// Serialize the index to a binary file at `path`.
///
/// Format (v2):
/// - Magic: b"SONR" (4 bytes)
/// - Version: u32 LE
/// - Content hash: length-prefixed UTF-8 string
/// - Chunks JSON: length-prefixed blob
/// - Tokenized documents JSON: length-prefixed blob
/// - Has embeddings: u8 (0 or 1)
/// - If has embeddings: dim u32 + raw f32 vectors blob
pub fn save_index(index: &SonarIndex, content_hash: &str, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create directory: {e}"))?;
    }

    let mut buf: Vec<u8> = Vec::new();

    buf.extend_from_slice(MAGIC);
    write_u32(&mut buf, VERSION)?;

    write_blob(&mut buf, content_hash.as_bytes())?;

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
///
/// Returns `(SonarIndex, stored_content_hash)`.
pub fn load_index(path: &Path) -> Result<(SonarIndex, String), String> {
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

    let hash_bytes = read_blob(&mut cursor)?;
    let content_hash =
        String::from_utf8(hash_bytes).map_err(|e| format!("invalid hash string: {e}"))?;

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
        Ok((index, content_hash))
    } else {
        let index = SonarIndex::new(chunks, bm25, file_mapping, language_mapping);
        Ok((index, content_hash))
    }
}

/// Default cache path: `<root>/.sonar/index.bin`
pub fn default_cache_path(root: &Path) -> std::path::PathBuf {
    root.join(".sonar").join("index.bin")
}

/// Try loading a cached index for `root`. Returns `None` if the cache is missing or stale.
pub fn load_cached(root: &Path) -> Result<Option<SonarIndex>, String> {
    let cache_path = default_cache_path(root);
    if !cache_path.exists() {
        return Ok(None);
    }

    let (index, stored_hash) = load_index(&cache_path)?;
    let current_hash = compute_content_hash(root)?;

    if stored_hash != current_hash {
        return Ok(None);
    }

    Ok(Some(index))
}

/// Build a fresh index from `root`, save it to the cache, and return it.
pub fn build_and_save(root: &Path) -> Result<SonarIndex, String> {
    if !root.exists() {
        return Err(format!("Path does not exist: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: {}", root.display()));
    }

    let walked = walk_directory(root);
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
            crate::index::SonarIndex::new_hybrid(chunks, bm25, file_mapping, language_mapping, emb, flat)
        }
        Err(_) => SonarIndex::new(chunks, bm25, file_mapping, language_mapping),
    };

    let content_hash = compute_content_hash(root)?;
    let cache_path = default_cache_path(root);
    save_index(&index, &content_hash, &cache_path)?;

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
    fn test_save_load_roundtrip() {
        let project_dir = test_dir();
        create_test_project(&project_dir);

        let index = crate::index::SonarIndex::from_path(&project_dir).unwrap();
        let original_stats = index.stats();

        let cache_file = project_dir.join("test_index.bin");
        let hash = compute_content_hash(&project_dir).unwrap();
        save_index(&index, &hash, &cache_file).unwrap();

        let (loaded, loaded_hash) = load_index(&cache_file).unwrap();
        let loaded_stats = loaded.stats();

        assert_eq!(original_stats.indexed_files, loaded_stats.indexed_files);
        assert_eq!(original_stats.total_chunks, loaded_stats.total_chunks);
        assert_eq!(original_stats.languages, loaded_stats.languages);
        assert_eq!(hash, loaded_hash);

        let results = loaded.search("main", 5);
        assert!(!results.is_empty(), "loaded index should support search");

        let _ = fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn test_staleness_detection() {
        let project_dir = test_dir();
        create_test_project(&project_dir);

        let hash_before = compute_content_hash(&project_dir).unwrap();

        std::thread::sleep(std::time::Duration::from_secs(1));
        write_test_file(
            &project_dir,
            "new_file.rs",
            "fn new_function() {\n    // new\n}\n",
        );

        let hash_after = compute_content_hash(&project_dir).unwrap();
        assert_ne!(
            hash_before, hash_after,
            "hash should change when files are added"
        );

        let _ = fs::remove_dir_all(&project_dir);
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
        let result = load_cached(&dir).unwrap();
        assert!(result.is_none(), "should return None when no cache exists");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_cached_detects_staleness() {
        let project_dir = test_dir();
        create_test_project(&project_dir);

        let index = crate::index::SonarIndex::from_path(&project_dir).unwrap();
        let hash = compute_content_hash(&project_dir).unwrap();
        save_index(&index, &hash, &default_cache_path(&project_dir)).unwrap();

        let cached = load_cached(&project_dir).unwrap();
        assert!(cached.is_some(), "cache should be valid initially");

        std::thread::sleep(std::time::Duration::from_secs(1));
        write_test_file(&project_dir, "extra.rs", "fn extra() {\n    // extra\n}\n");

        let stale = load_cached(&project_dir).unwrap();
        assert!(stale.is_none(), "cache should be stale after adding a file");

        let _ = fs::remove_dir_all(&project_dir);
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
        let hash = "test_hash_abc123";
        save_index(&index, hash, &cache_file).unwrap();

        let (loaded, loaded_hash) = load_index(&cache_file).unwrap();

        assert_eq!(loaded_hash, hash);
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
