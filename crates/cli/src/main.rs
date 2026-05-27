use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

#[derive(Clone, Copy, ValueEnum, Debug, PartialEq)]
enum ContentArg {
    Code,
    Docs,
    Config,
    All,
}

fn resolve_content_types(content: &[ContentArg]) -> Vec<sonar_core::types::ContentType> {
    use sonar_core::types::ContentType;
    if content.is_empty() || content.contains(&ContentArg::All) {
        return vec![ContentType::Code, ContentType::Docs, ContentType::Config];
    }
    let mut types = Vec::new();
    for c in content {
        match c {
            ContentArg::Code => types.push(ContentType::Code),
            ContentArg::Docs => types.push(ContentType::Docs),
            ContentArg::Config => types.push(ContentType::Config),
            ContentArg::All => unreachable!(),
        }
    }
    types.dedup();
    types
}

#[derive(Subcommand)]
enum Commands {
    /// Index a directory for searching.
    Index {
        /// Path to the directory to index.
        path: String,

        /// Content types to index: code, docs, config, or all.
        #[arg(long, value_enum, default_values_t = vec![ContentArg::Code])]
        content: Vec<ContentArg>,
    },
    /// Search an indexed codebase.
    Search {
        /// Natural language or code query.
        query: String,

        /// Path to directory or git URL (https/http) to search.
        #[arg(short, long, default_value = ".")]
        path: String,

        /// Branch or tag to clone (only used with git URLs).
        #[arg(long, name = "ref")]
        git_ref: Option<String>,

        /// Number of results to return.
        #[arg(short = 'k', long, default_value = "5")]
        top_k: usize,

        /// Search mode: hybrid, semantic, or bm25.
        #[arg(short, long, value_enum, default_value = "hybrid")]
        mode: ModeArg,

        /// Content types to search: code, docs, config, or all.
        #[arg(long, value_enum, default_values_t = vec![ContentArg::Code])]
        content: Vec<ContentArg>,
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
    /// Show token savings from sonar usage.
    Savings {
        /// Show breakdown by call type.
        #[arg(long)]
        verbose: bool,
    },
    /// Generate agent config for an AI coding tool.
    Init {
        /// Agent type: claude, cursor, copilot, gemini, kiro, opencode.
        #[arg(long, short, default_value = "claude")]
        agent: String,
        /// Overwrite existing file.
        #[arg(long)]
        force: bool,
    },
}

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path, content } => {
            let content_types = resolve_content_types(&content);
            let path = Path::new(&path);
            eprintln!("Indexing {}...", path.display());
            let index = sonar_core::persist::build_and_save_content(path, &content_types)
                .map_err(|e| anyhow::anyhow!(e))?;
            let cache_dir =
                sonar_core::persist::cache_dir_for_content(path, &content_types, None)
                    .map_err(|e| anyhow::anyhow!(e))?;
            let stats = index.stats();
            eprintln!(
                "Done. {} files, {} chunks, {} languages.",
                stats.indexed_files,
                stats.total_chunks,
                stats.languages.len()
            );
            eprintln!("Index saved to {}", cache_dir.display());
            for (lang, count) in &stats.languages {
                eprintln!("  {lang}: {count} chunks");
            }
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Commands::Search {
            query,
            path,
            git_ref,
            top_k,
            mode,
            content,
        } => {
            let content_types = resolve_content_types(&content);
            let mut index = if sonar_core::utils::is_git_url(&path) {
                eprintln!("Cloning {}...", path);
                sonar_core::index::SonarIndex::from_git(&path, git_ref.as_deref(), &[])
                    .map_err(|e| anyhow::anyhow!(e))?
            } else {
                sonar_core::index::SonarIndex::from_path_cached_with_content(
                    Path::new(&path),
                    &content_types,
                )
                .map_err(|e| anyhow::anyhow!(e))?
            };
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
            eprintln!("Model ready. Embedding dimension: {}", embedder.dim());
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
        Commands::Savings { verbose } => {
            let records = sonar_core::stats::read_usage().map_err(|e| anyhow::anyhow!(e))?;

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();

            let today_start = now - 86400.0;
            let week_start = now - 7.0 * 86400.0;

            let today = sonar_core::stats::calculate_savings(&records, Some(today_start));
            let week = sonar_core::stats::calculate_savings(&records, Some(week_start));
            let all_time = sonar_core::stats::calculate_savings(&records, None);

            println!("Token savings (estimated):");
            println!(
                "  Today:        {:>6} tokens saved ({} calls)",
                format_number(today.saved_tokens),
                today.calls
            );
            println!(
                "  Last 7 days:  {:>6} tokens saved ({} calls)",
                format_number(week.saved_tokens),
                week.calls
            );
            println!(
                "  All time:     {:>6} tokens saved ({} calls)",
                format_number(all_time.saved_tokens),
                all_time.calls
            );

            if verbose {
                println!("\nBreakdown by call type:");
                let search_records: Vec<_> = records
                    .iter()
                    .filter(|r| r.call == "search")
                    .cloned()
                    .collect();
                let related_records: Vec<_> = records
                    .iter()
                    .filter(|r| r.call == "find_related")
                    .cloned()
                    .collect();

                let search_savings = sonar_core::stats::calculate_savings(&search_records, None);
                let related_savings = sonar_core::stats::calculate_savings(&related_records, None);

                println!(
                    "  search:        {:>6} tokens saved ({} calls)",
                    format_number(search_savings.saved_tokens),
                    search_savings.calls
                );
                println!(
                    "  find_related:  {:>6} tokens saved ({} calls)",
                    format_number(related_savings.saved_tokens),
                    related_savings.calls
                );
            }
        }
        Commands::Init { agent, force } => {
            let agent_path = match agent.as_str() {
                "claude" => ".claude/agents/sonar-search.md",
                "copilot" => ".github/agents/sonar-search.md",
                "cursor" => ".cursor/agents/sonar-search.md",
                "gemini" => ".gemini/agents/sonar-search.md",
                "kiro" => ".kiro/agents/sonar-search.md",
                "opencode" => ".opencode/agents/sonar-search.md",
                other => {
                    anyhow::bail!(
                        "Unknown agent '{}'. Supported: claude, cursor, copilot, gemini, kiro, opencode",
                        other
                    );
                }
            };

            let path = PathBuf::from(agent_path);
            if path.exists() && !force {
                eprintln!(
                    "Error: {} already exists. Use --force to overwrite.",
                    path.display()
                );
                std::process::exit(1);
            }

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            static AGENT_TEMPLATE: &str = include_str!("../agents/sonar-search.md");
            fs::write(&path, AGENT_TEMPLATE)?;
            println!("Created {}", path.display());
        }
    }

    Ok(())
}
