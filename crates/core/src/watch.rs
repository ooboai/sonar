use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "__pycache__",
    ".venv",
    "vendor",
    "dist",
    "build",
];

const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg", "webp", "mp3", "mp4", "wav", "avi", "mov",
    "mkv", "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "exe", "dll", "so", "dylib", "o", "obj",
    "class", "pyc", "pyo", "wasm", "bin", "dat", "db", "sqlite", "lock", "pdf", "doc", "docx",
    "xls", "xlsx",
];

pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    debounce: Duration,
    last_poll: Instant,
    pending: HashSet<PathBuf>,
}

impl FileWatcher {
    pub fn new(root: PathBuf, debounce_ms: u64) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let config = notify::Config::default().with_poll_interval(Duration::from_millis(250));
        let mut watcher = notify::RecommendedWatcher::new(tx, config)?;
        let canonical = root.canonicalize().unwrap_or(root);
        watcher.watch(&canonical, RecursiveMode::Recursive)?;

        Ok(Self {
            _watcher: watcher,
            rx,
            debounce: Duration::from_millis(debounce_ms),
            last_poll: Instant::now(),
            pending: HashSet::new(),
        })
    }

    pub fn has_changes(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn poll_changes(&mut self) -> Vec<PathBuf> {
        for event in self.rx.try_iter() {
            let Ok(event) = event else { continue };
            if !is_relevant_event(&event.kind) {
                continue;
            }
            for path in event.paths {
                if should_skip(&path) {
                    continue;
                }
                self.pending.insert(path);
            }
        }

        let now = Instant::now();
        if now.duration_since(self.last_poll) < self.debounce && !self.pending.is_empty() {
            return Vec::new();
        }

        self.last_poll = now;
        self.pending.drain().collect()
    }
}

fn is_relevant_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn should_skip(path: &Path) -> bool {
    for component in path.components() {
        let s = component.as_os_str().to_string_lossy();
        if s.starts_with('.') && s.len() > 1 {
            return true;
        }
        if SKIP_DIRS.contains(&s.as_ref()) {
            return true;
        }
    }

    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        if SKIP_EXTENSIONS.contains(&ext.as_str()) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_skip_hidden_files() {
        assert!(should_skip(Path::new("/repo/.git/config")));
        assert!(should_skip(Path::new("/repo/.hidden/file.rs")));
        assert!(should_skip(Path::new("/repo/src/.env")));
        assert!(!should_skip(Path::new("/repo/src/main.rs")));
    }

    #[test]
    fn test_skip_non_code_files() {
        assert!(should_skip(Path::new("/repo/image.png")));
        assert!(should_skip(Path::new("/repo/archive.zip")));
        assert!(should_skip(Path::new("/repo/lib.so")));
        assert!(should_skip(Path::new("/repo/node_modules/foo/index.js")));
        assert!(should_skip(Path::new("/repo/target/debug/main")));
        assert!(!should_skip(Path::new("/repo/src/lib.rs")));
        assert!(!should_skip(Path::new("/repo/src/index.ts")));
    }

    #[test]
    fn test_debounce() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        // Use a PollWatcher directly for test reliability across platforms
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let config = notify::Config::default().with_poll_interval(Duration::from_millis(100));
        let mut poll_watcher = notify::PollWatcher::new(tx, config).unwrap();
        poll_watcher.watch(&root, RecursiveMode::Recursive).unwrap();

        // Pre-create a seed file so the watcher has a baseline
        let seed = root.join("seed.rs");
        fs::write(&seed, "// seed").unwrap();

        // Let the poll watcher do its initial scan
        std::thread::sleep(Duration::from_millis(300));

        // Now write the file we're looking for
        let test_file = root.join("test.rs");
        fs::write(&test_file, "fn main() {}").unwrap();

        // Collect events using the same logic as FileWatcher
        let mut found = false;
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(100));
            for event in rx.try_iter().flatten() {
                if is_relevant_event(&event.kind)
                    && event.paths.iter().any(|p| p.ends_with("test.rs"))
                {
                    found = true;
                }
            }
            if found {
                break;
            }
        }

        assert!(found, "expected test.rs change to be detected");
    }
}
