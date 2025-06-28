use anyhow::Result;
use clap::Parser;
use log::info;
use std::path::{Path, PathBuf};

mod cli;
mod config;
mod downloader;
mod hf;
mod search;
mod updater;
mod util;

use cli::{Cli, Commands, ModelCommands};
use downloader::{run_downloads, DownloadItem};
use hf::fetch_hugging_face_urls;
use search::handle_model_search;
use updater::handle_update;
use util::log_panic;

#[tokio::main]
async fn main() -> Result<()> {
    // This will be useful if the program panics
    std::panic::set_hook(Box::new(log_panic));

    let cli = Cli::parse();
    setup_logging_for_debug(cli.debug)?;

    let hf_token = if cli.token {
        let token = std::env::var("HF_TOKEN").unwrap_or_default();
        if token.is_empty() {
            eprintln!("[WARN] --token flag is set, but HF_TOKEN environment variable is not set or is empty.");
        }
        token
    } else {
        String::new()
    };

    match cli.command {
        Some(Commands::Model { command }) => match command {
            ModelCommands::Search { query } => {
                handle_model_search(&query.join(" "), &hf_token).await?;
            }
        },
        Some(Commands::UpdateApp) => {
            handle_update().await?;
        }
        None => {
            // This is the downloader path
            run_downloader_flow(cli, &hf_token).await?;
        }
    }

    info!("Program finished successfully.");
    Ok(())
}

fn setup_logging_for_debug(debug: bool) -> Result<()> {
    if debug {
        fern::Dispatch::new()
            .format(|out, message, record| {
                out.finish(format_args!(
                    "[{}][{}] {}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                    record.level(),
                    message
                ))
            })
            .level(log::LevelFilter::Debug)
            .chain(fern::log_file("log.log")?)
            .apply()
            .map_err(|e| anyhow::anyhow!("Failed to apply logger configuration: {}", e))?;
        log::debug!("Debug logging enabled. Log will be written to log.log");
    }
    // If not in debug mode, the logger is not initialized.
    Ok(())
}


async fn run_downloader_flow(cli: Cli, hf_token: &str) -> Result<()> {
    let mut modes_set = 0;
    if cli.file.is_some() {
        modes_set += 1;
    }
    if cli.hf.is_some() {
        modes_set += 1;
    }
    if cli.model.is_some() {
        modes_set += 1;
    }
    if !cli.urls.is_empty() {
        modes_set += 1;
    }

    if modes_set == 0 {
        return Err(anyhow::anyhow!(
            "No download source provided. Use URLs, -f, -h, or -m. Use --help for more info."
        ));
    }
    if modes_set > 1 {
        return Err(anyhow::anyhow!(
            "Flags -f, -h, -m, and direct URLs are mutually exclusive."
        ));
    }

    let mut download_items = Vec::new();
    let mut download_dir = PathBuf::from("downloads");

    if let Some(model_alias) = cli.model {
        let registry = config::get_model_registry();
        if let Some(url) = registry.get(model_alias.as_str()) {
            let preferred_filename = Path::new(url)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("download.file")
                .to_string();
            download_items.push(DownloadItem {
                url: url.to_string(),
                preferred_filename: Some(preferred_filename),
            });
            download_dir.push(util::sanitize_filename(&model_alias));
        } else {
            return Err(anyhow::anyhow!("Model alias '{}' not found in the registry.", model_alias));
        }
    } else if let Some(hf_repo) = cli.hf {
        eprintln!("[INFO] Fetching file list from Hugging Face repository: {}", hf_repo);
        let all_repo_files = fetch_hugging_face_urls(&hf_repo, hf_token).await?;
        if all_repo_files.is_empty() {
            eprintln!("[INFO] No files found in the repository. Exiting.");
            return Ok(());
        }

        let files_to_download = if cli.select {
            // The Fix: `select_gguf_files` now manages its own concurrency and no longer needs the `cli.concurrency` argument.
            downloader::select_gguf_files(all_repo_files, hf_token).await?
        } else {
            all_repo_files
        };

        for hf_file in files_to_download {
            download_items.push(DownloadItem {
                url: hf_file.url,
                preferred_filename: Some(hf_file.filename),
            });
        }
        
        let safe_repo_name = util::repo_id_to_safe_path(&hf_repo);
        download_dir.push(safe_repo_name);

    } else {
        let mut input_urls = cli.urls;
        if let Some(file_path) = cli.file {
            let content = tokio::fs::read_to_string(file_path).await?;
            let urls_from_file = content
                .lines()
                .map(str::trim)
                .filter(|&s| !s.is_empty() && !s.starts_with('#'))
                .map(String::from);
            input_urls.extend(urls_from_file);
        }
        for url in input_urls {
            download_items.push(DownloadItem {
                url,
                preferred_filename: None,
            });
        }
    }
    
    if download_items.is_empty() {
        eprintln!("[INFO] No files to download. Exiting.");
        return Ok(());
    }

    if !download_dir.exists() {
        tokio::fs::create_dir_all(&download_dir).await?;
    }

    run_downloads(download_items, download_dir, cli.concurrency, hf_token.to_string()).await?;

    Ok(())
}