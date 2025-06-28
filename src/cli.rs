use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "A command-line tool for concurrent downloads.",
    long_about = r#"DL is a command-line tool written in Rust for downloading multiple files concurrently from a list of URLs or a Hugging Face repository. 
It features a dynamic progress bar display for each download, showing speed, percentage, and downloaded/total size.
It also includes utilities for searching Hugging Face models and self-updating."#,
    after_help = r#"Examples:
  Download a file directly:
    dl http://example.com/file.zip

  Download from a list in a file with concurrency 5:
    dl -f urls.txt -c 5

  Download (and select files) from a Hugging Face repo using token:
    dl -H TheBloke/Llama-2-7B-GGUF -s --token

  Search for Hugging Face models using a token:
    dl model search "llama 7b gguf" --token

  Self-update the application:
    dl update
"#
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Direct URLs to download.
    #[arg()]
    pub urls: Vec<String>,

    /// Number of concurrent downloads.
    #[arg(short, long, default_value_t = 3)]
    pub concurrency: usize,

    /// Path to a text file containing URLs to download (one per line).
    #[arg(short, long)]
    pub file: Option<PathBuf>,

    /// Hugging Face repository ID (e.g., 'TheBloke/Llama-2-7B-GGUF') or URL.
    #[arg(short = 'H', long)]
    pub hf: Option<String>,

    /// Predefined model alias to download.
    #[arg(short, long)]
    pub model: Option<String>,

    /// Interactively select GGUF files from a Hugging Face repository.
    #[arg(short = 's', long)]
    pub select: bool,

    /// Use HF_TOKEN environment variable for Hugging Face requests.
    #[arg(long)]
    pub token: bool,

    /// Enable debug logging to log.log.
    #[arg(long)]
    pub debug: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage Hugging Face models.
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },
    /// Check for and apply application self-updates.
    #[command(name = "update")]
    UpdateApp,
}

#[derive(Subcommand, Debug)]
pub enum ModelCommands {
    /// Search for models on Hugging Face.
    Search {
        #[arg(required = true, help = "The search term for models")]
        query: Vec<String>,
    },
}