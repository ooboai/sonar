use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "sonar", about = "Fast hybrid code search for agents.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    Hybrid,
    Semantic,
    Bm25,
}

impl From<ModeArg> for sonar_core::index::Mode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Hybrid => sonar_core::index::Mode::Hybrid,
            ModeArg::Semantic => sonar_core::index::Mode::Semantic,
            ModeArg::Bm25 => sonar_core::index::Mode::Bm25,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Index a directory for searching.
    Index {
        /// Path to the directory to index.
        path: String,
    },
    /// Search an indexed codebase.
    Search {
        /// Natural language or code query.
        query: String,

        /// Path to the indexed directory.
        #[arg(short, long, default_value = ".")]
        path: String,

        /// Number of results to return.
        #[arg(short = 'k', long, default_value = "5")]
        top_k: usize,

        /// Search mode: hybrid, semantic, or bm25.
        #[arg(short, long, value_enum, default_value = "hybrid")]
        mode: ModeArg,
    },
    /// Download the embedding model from HuggingFace Hub.
    DownloadModel {
        /// Model name in "org/model" format.
        #[arg(short, long, default_value = "minishlab/potion-code-16M")]
        model: String,
    },
    /// Watch a directory and re-index on changes.
    Watch {
        /// Path to the directory to watch.
        path: String,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path } => {
            let path = Path::new(&path);
            eprintln!("Indexing {}...", path.display());
            let index =
                sonar_core::persist::build_and_save(path).map_err(|e| anyhow::anyhow!(e))?;
            let cache_path = sonar_core::persist::default_cache_path(path);
            let stats = index.stats();
            eprintln!(
                "Done. {} files, {} chunks, {} languages.",
                stats.indexed_files,
                stats.total_chunks,
                stats.languages.len()
            );
            eprintln!("Index saved to {}", cache_path.display());
            for (lang, count) in &stats.languages {
                eprintln!("  {lang}: {count} chunks");
            }
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Commands::Search {
            query,
            path,
            top_k,
            mode,
        } => {
            let path = Path::new(&path);
            let mut index = sonar_core::index::SonarIndex::from_path_cached(path)
                .map_err(|e| anyhow::anyhow!(e))?;
            let requested: sonar_core::index::Mode = mode.into();
            index.set_mode(requested);
            eprintln!("Search mode: {}", index.mode());
            let results = index.search(&query, top_k);
            let formatted = sonar_core::utils::format_results(&query, &results);
            println!("{}", serde_json::to_string_pretty(&formatted)?);
        }
        Commands::DownloadModel { model } => {
            eprintln!("Downloading model '{model}' from HuggingFace Hub...");
            let embedder = sonar_core::embed::Embedder::from_pretrained(&model)?;
            eprintln!(
                "Model ready. Embedding dimension: {}",
                embedder.dim()
            );
        }
        Commands::Watch { path } => {
            let path = PathBuf::from(&path);
            if !path.exists() {
                anyhow::bail!("Path does not exist: {}", path.display());
            }

            let running = Arc::new(AtomicBool::new(true));
            let r = running.clone();
            ctrlc::set_handler(move || {
                r.store(false, Ordering::SeqCst);
            })?;

            eprintln!("Watching {}...", path.display());

            let mut watcher = sonar_core::watch::FileWatcher::new(path.clone(), 500)?;

            while running.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let changes = watcher.poll_changes();
                if !changes.is_empty() {
                    eprintln!(
                        "Re-indexing due to changes: {} files modified",
                        changes.len()
                    );
                    match sonar_core::persist::build_and_save(&path) {
                        Ok(index) => {
                            let stats = index.stats();
                            eprintln!(
                                "Re-indexed: {} files, {} chunks.",
                                stats.indexed_files, stats.total_chunks
                            );
                        }
                        Err(e) => {
                            eprintln!("Re-index failed: {e}");
                        }
                    }
                }
            }

            eprintln!("\nStopped watching.");
        }
    }

    Ok(())
}
