use crate::{
    config::GGUF_SERIES_REGEX,
    hf::HFFile,
    util::{format_bytes, format_duration_human, generate_actual_filename, get_client, shorten_error},
};
use anyhow::{anyhow, Context, Result};
use futures_util::stream::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressState, ProgressStyle};
use log::{debug, error, info};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use tokio::io::AsyncWriteExt;

// A dedicated, higher concurrency level for fetching metadata.
// This is much faster than the default download concurrency of 3.
const PRESCAN_CONCURRENCY: usize = 20;

#[derive(Debug)]
pub struct DownloadItem {
    pub url: String,
    pub preferred_filename: Option<String>,
}

struct DownloadTask {
    item: DownloadItem,
    destination_path: PathBuf,
    progress_bar: ProgressBar,
    overall_progress_bar: ProgressBar,
    multi_progress: Arc<MultiProgress>,
    client: reqwest::Client,
}

pub async fn run_downloads(
    items: Vec<DownloadItem>,
    base_dir: PathBuf,
    concurrency: usize,
    hf_token: String,
) -> Result<()> {
    eprintln!(
        "[INFO] Preparing to download {} file(s) to '{}' with concurrency {}.",
        items.len(),
        base_dir.display(),
        concurrency
    );

    let multi_progress = Arc::new(MultiProgress::new());
    
    // --- Pre-scan for file sizes ---
    eprintln!(
        "[INFO] Pre-scanning {} file(s) for sizes (this may take a moment)...",
        items.len()
    );
    let prescan_bar = multi_progress.add(ProgressBar::new(items.len() as u64));
    prescan_bar.set_style(
        ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
        .expect("Invalid progress bar template")
        .progress_chars("#>-"),
    );
    prescan_bar.set_message("Fetching file sizes...");

    let file_sizes = Arc::new(Mutex::new(HashMap::<String, u64>::new()));
    let error_count = Arc::new(AtomicUsize::new(0));

    // Create ONE client that will be cloned for all concurrent tasks. This is efficient and robust.
    let prescan_client = get_client(&hf_token)?;
    let prescan_futs = items.iter().map(|item| {
        let client = prescan_client.clone(); // Use the cloned client
        let item_url = item.url.clone();
        let item_name = item.preferred_filename.as_deref().unwrap_or(&item.url).to_string();
        let prescan_bar = prescan_bar.clone();
        let file_sizes = file_sizes.clone();
        let error_count = error_count.clone();

        async move {
            match fetch_file_size(&client, &item_url).await {
                Ok(s) => {
                    file_sizes.lock().unwrap().insert(item_url, s);
                }
                Err(e) => {
                    log::warn!("Prescan failed for {}: {}", item_name, e);
                    let current_errors = error_count.fetch_add(1, Ordering::SeqCst);
                    if current_errors < 5 {
                         prescan_bar.println(format!("[WARN] Could not get size for '{}'. It will show as 0 B.", item_name));
                    }
                }
            }
            prescan_bar.inc(1);
        }
    });
    
    let stream = futures_util::stream::iter(prescan_futs);
    stream.buffer_unordered(PRESCAN_CONCURRENCY).for_each(|_| async {}).await;
    prescan_bar.finish_with_message("Pre-scan complete.");

    // --- Prepare download tasks ---
    let mut tasks = Vec::new();
    let total_download_size: u64 = items
        .iter()
        .map(|item| *file_sizes.lock().unwrap().get(&item.url).unwrap_or(&0))
        .sum();
    
    let overall_pb = multi_progress.add(ProgressBar::new(total_download_size));
    // The Fix: Overall progress bar template now matches individual bars for consistency and custom formatting.
    let overall_style = ProgressStyle::with_template(
        "Overall Progress: [{bar:40.yellow/blue}] {percent:>3}% │ {bytes_formatted}/{total_bytes_formatted} @ {bytes_per_sec} │ ETA: {eta_formatted}"
    ).expect("Invalid overall progress bar template")
     .with_key("bytes_formatted", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format_bytes(state.pos())).unwrap())
     .with_key("total_bytes_formatted", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format_bytes(state.len().unwrap_or(0))).unwrap())
     .with_key("eta_formatted", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format_duration_human(state.eta(), false)).unwrap())
     .progress_chars("=> ");
    overall_pb.set_style(overall_style);
    
    // Define styles for individual bars
    let download_style = ProgressStyle::with_template(
        "{msg:30!} [{bar:25.cyan/blue}] {percent:>3}% │ {bytes_formatted}/{total_bytes_formatted} @ {bytes_per_sec} │ ETA: {eta_formatted}"
    ).expect("Invalid download progress bar template")
     .with_key("bytes_formatted", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format_bytes(state.pos())).unwrap())
     .with_key("total_bytes_formatted", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format_bytes(state.len().unwrap_or(0))).unwrap())
     .with_key("eta_formatted", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format_duration_human(state.eta(), true)).unwrap())
     .progress_chars("=> ");
    let error_style = ProgressStyle::with_template(
        "{msg:30!} [ERROR: {wide_msg}]"
    ).expect("Invalid error progress bar template");

    let download_client = get_client(&hf_token)?;
    for item in items {
        let actual_filename =
            generate_actual_filename(&item.url, item.preferred_filename.as_deref());
        let destination_path = base_dir.join(&actual_filename);

        let size = *file_sizes.lock().unwrap().get(&item.url).unwrap_or(&0);
        // Create progress bar hidden initially - it will be shown when download starts
        let pb = ProgressBar::new(size);
        pb.set_draw_target(indicatif::ProgressDrawTarget::hidden());
        pb.set_style(download_style.clone());
        pb.set_message(truncate_filename(&actual_filename, 30));

        tasks.push(DownloadTask {
            item,
            destination_path,
            progress_bar: pb,
            overall_progress_bar: overall_pb.clone(),
            multi_progress: multi_progress.clone(),
            client: download_client.clone(),
        });
    }

    // --- Execute downloads ---
    let download_futs = tasks.into_iter().map(|task| {
        let url_for_log = task.item.url.clone();
        // Clone progress bar for post-download handling
        let pb_clone_for_post_download = task.progress_bar.clone();
        let error_style_clone = error_style.clone();

        tokio::spawn(async move {
            if let Err(e) = download_file(task).await {
                error!("Download failed for {}: {:?}", url_for_log, e);
                let short_err = shorten_error(&e, 40);
                pb_clone_for_post_download.set_style(error_style_clone);
                pb_clone_for_post_download.finish_with_message(short_err);
            } else {
                // Clear completed downloads from display
                pb_clone_for_post_download.finish_and_clear();
            }
        })
    });
    
    let stream = futures_util::stream::iter(download_futs);
    // Use the user-provided concurrency for the actual downloads.
    stream.buffer_unordered(concurrency).for_each(|_| async {}).await;
    
    overall_pb.finish_with_message("All downloads finished.");
    
    eprintln!("\nAll downloads processed.");
    Ok(())
}

async fn download_file(task: DownloadTask) -> Result<()> {
    let url = &task.item.url;
    let path = &task.destination_path;
    let overall_pb = &task.overall_progress_bar;
    let client = &task.client;
    
    // Add progress bar to display now that this download is starting
    let pb = task.multi_progress.add(task.progress_bar);
    
    info!("Starting download for URL: {}", url);
    debug!("Destination path: {}", path.display());
    
    let result = (async {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }

        let mut current_size = 0;
        if path.exists() {
            current_size = tokio::fs::metadata(path).await?.len();
        }
        
        let total_size = pb.length().unwrap_or(0);
        if total_size > 0 && current_size >= total_size {
            debug!("File {} already complete.", path.display());
            pb.set_position(total_size);
            overall_pb.inc(total_size.saturating_sub(current_size));
            // The Fix: Set message for finished state here.
            pb.set_message(format!("{} [Done]", truncate_filename(&path.to_string_lossy(), 20)));
            return Ok(());
        }
        
        let mut request = client.get(url);
        if current_size > 0 {
            debug!("Resuming download for {} from byte {}", path.display(), current_size);
            request = request.header(reqwest::header::RANGE, format!("bytes={}-", current_size));
        }
        
        let resp = request.send().await?.error_for_status()?;

        let is_resume = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !is_resume && current_size > 0 {
            eprintln!("[WARN] Server does not support resume for {}. Starting from beginning.", url);
            overall_pb.inc(0_u64.saturating_sub(current_size));
            current_size = 0;
        } else {
            overall_pb.inc(current_size);
        }

        let mut file = if is_resume {
            tokio::fs::OpenOptions::new().append(true).open(path).await?
        } else {
            tokio::fs::File::create(path).await?
        };

        pb.set_position(current_size);

        let mut stream = resp.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read chunk from download stream")?;
            file.write_all(&chunk).await.context("Failed to write chunk to file")?;
            let chunk_len = chunk.len() as u64;
            pb.inc(chunk_len);
            overall_pb.inc(chunk_len);
        }
        
        let final_len = tokio::fs::metadata(path).await?.len();
        if total_size > 0 && final_len < total_size {
            eprintln!("[WARN] Download for {} may be incomplete. Expected {}, got {}.", url, total_size, final_len);
            return Err(anyhow!("Incomplete download for {}", url));
        }

        // The Fix: Set message for finished state here.
        pb.set_message(format!("{} [Done]", truncate_filename(&path.to_string_lossy(), 20)));
        info!("Finished download for {}", url);
        Ok(())
    }).await;

    if let Err(e) = result {
        // Error handling for progress bar is now done in the parent `run_downloads` loop.
        return Err(e);
    }
    
    Ok(())
}


/// Fetches the size of a remote file using a robust, two-stage approach.
async fn fetch_file_size(client: &reqwest::Client, url: &str) -> Result<u64> {
    debug!("Fetching size for URL: {}", url);

    // Stage 1: Attempt HEAD request. The client is configured to follow redirects automatically.
    let head_resp = client.head(url).send().await;

    if let Ok(resp) = head_resp {
        if resp.status().is_success() {
            if let Some(length) = resp.content_length() {
                if length > 0 {
                    debug!("Got size {} via HEAD for {}", length, url);
                    return Ok(length);
                }
            }
        }
    }

    // Stage 2: Fallback to GET request if HEAD fails or provides no size.
    debug!("HEAD failed or gave no size, falling back to GET for {}", url);
    let get_resp = client.get(url).send().await?;
    
    if get_resp.status().is_success() {
        if let Some(length) = get_resp.content_length() {
            debug!("Got size {} via GET for {}", length, url);
            return Ok(length);
        }
    }
    
    Err(anyhow!("Could not determine file size for {}", url))
}


fn truncate_filename(filename: &str, max_len: usize) -> String {
    if filename.chars().count() > max_len {
        let path = Path::new(filename);
        let stem = path.file_stem().unwrap_or_default().to_str().unwrap_or("");
        let ext = path.extension().unwrap_or_default().to_str().unwrap_or("");
        
        let ext_part = if !ext.is_empty() { format!(".{}", ext) } else { String::new() };
        let available_len = max_len.saturating_sub(ext_part.len() + 3);
        
        if available_len > 0 && stem.len() > available_len {
            let start_index = stem.char_indices().map(|(i, _)| i).nth(stem.chars().count() - available_len).unwrap_or(0);
            format!("...{}{}", &stem[start_index..], ext_part)
        } else {
             format!("{}...", filename.chars().take(max_len - 3).collect::<String>())
        }
    } else {
        filename.to_string()
    }
}

#[derive(Debug, Clone)]
struct GGUFSeriesInfo {
    base_name: String,
    total_parts: usize,
    files: Vec<(HFFile, u64)>,
    total_size: u64,
}

#[derive(Debug)]
enum SelectableGGUFItem {
    Series(GGUFSeriesInfo),
    File(HFFile, u64),
}

impl SelectableGGUFItem {
    fn display_name(&self) -> String {
        match self {
            SelectableGGUFItem::Series(info) => {
                let completeness = if info.files.len() == info.total_parts && info.total_parts > 0 {
                    String::new()
                } else {
                    format!(" (INCOMPLETE: {}/{} parts)", info.files.len(), info.total_parts)
                };
                format!(
                    "Series: {} ({} parts, {}){}",
                    info.base_name,
                    info.files.len(),
                    format_bytes(info.total_size),
                    completeness
                )
            }
            SelectableGGUFItem::File(file, size) => {
                format!("File: {} ({})", file.filename, format_bytes(*size))
            }
        }
    }
    
    fn is_complete(&self) -> bool {
        match self {
            SelectableGGUFItem::Series(info) => info.files.len() == info.total_parts && info.total_parts > 0,
            SelectableGGUFItem::File(_, _) => true,
        }
    }

    fn get_files(&self) -> Vec<HFFile> {
        match self {
            SelectableGGUFItem::Series(info) => info.files.iter().map(|(f, _)| f.clone()).collect(),
            SelectableGGUFItem::File(file, _) => vec![file.clone()],
        }
    }
}

pub async fn select_gguf_files(
    all_files: Vec<HFFile>,
    hf_token: &str,
) -> Result<Vec<HFFile>> {
    eprintln!("[INFO] Identifying GGUF files and series for selection...");
    let gguf_files: Vec<_> = all_files.into_iter().filter(|f| f.filename.to_lowercase().ends_with(".gguf")).collect();

    if gguf_files.is_empty() {
        eprintln!("[INFO] No GGUF files found in the repository.");
        return Ok(vec![]);
    }
    
    eprintln!("[INFO] Fetching sizes for {} GGUF file(s)...", gguf_files.len());
    let client = get_client(hf_token)?;
    let pb = ProgressBar::new(gguf_files.len() as u64);
    pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} ({percent}%)").unwrap());

    let error_count = Arc::new(AtomicUsize::new(0));
    let size_futs = gguf_files.iter().map(|file| {
        let client = client.clone();
        let file = file.clone();
        let pb_clone = pb.clone();
        let error_count = error_count.clone();

        async move {
            let size_res = fetch_file_size(&client, &file.url).await;
            pb_clone.inc(1);
            match size_res {
                Ok(size) => (file, size),
                Err(e) => {
                    log::warn!("Failed to get size for {}: {}", file.filename, e);
                    let current_errors = error_count.fetch_add(1, Ordering::SeqCst);
                    if current_errors < 5 {
                        pb_clone.println(format!("[WARN] Could not get size for '{}'", file.filename));
                    } else if current_errors == 5 {
                        pb_clone.println("[WARN] (More size fetch errors suppressed)...");
                    }
                    (file, 0)
                }
            }
        }
    });
    
    let stream = futures_util::stream::iter(size_futs);
    let files_with_sizes: Vec<(HFFile, u64)> = stream.buffer_unordered(PRESCAN_CONCURRENCY).collect().await;
    pb.finish_and_clear();

    let mut series_map: HashMap<String, GGUFSeriesInfo> = HashMap::new();
    let mut standalone_files = Vec::new();

    for (file, size) in files_with_sizes {
        if let Some(caps) = GGUF_SERIES_REGEX.captures(&file.filename) {
            let base_name = caps.get(1).unwrap().as_str().to_string();
            let total_parts: usize = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
            let series_key = format!("{}-of-{}", base_name, total_parts);
            
            let entry = series_map.entry(series_key).or_insert_with(|| GGUFSeriesInfo {
                base_name: base_name.clone(),
                total_parts,
                files: Vec::new(),
                total_size: 0,
            });
            entry.files.push((file, size));
            entry.total_size += size;
        } else {
            standalone_files.push((file, size));
        }
    }
    
    let mut selectable_items: Vec<SelectableGGUFItem> = Vec::new();
    selectable_items.extend(series_map.into_values().map(SelectableGGUFItem::Series));
    selectable_items.extend(standalone_files.into_iter().map(|(f,s)| SelectableGGUFItem::File(f,s)));

    selectable_items.sort_by(|a, b| a.display_name().cmp(&b.display_name()));

    eprintln!("\nAvailable GGUF files/series for download:");
    for (i, item) in selectable_items.iter().enumerate() {
        eprintln!("{:3}. {}", i + 1, item.display_name());
    }
    eprintln!("---");

    loop {
        eprint!("Enter numbers (e.g., 1,3), 'all' (listed GGUFs), or 'none': ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        let choice = input.trim().to_lowercase();
        if choice == "none" {
            return Ok(vec![]);
        }
        if choice == "all" {
            return Ok(selectable_items
                .iter()
                .filter(|item| {
                    if !item.is_complete() {
                        eprintln!("[WARN] Skipping incomplete series: {}", item.display_name());
                        false
                    } else {
                        true
                    }
                })
                .flat_map(|item| item.get_files())
                .collect());
        }

        let mut files_to_download = Vec::new();
        let mut valid_selection = true;
        for part in choice.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match part.parse::<usize>() {
                Ok(num) if num > 0 && num <= selectable_items.len() => {
                    let item = &selectable_items[num - 1];
                    if !item.is_complete() {
                        eprintln!("[WARN] Skipping incomplete series: {}", item.display_name());
                        continue;
                    }
                    files_to_download.extend(item.get_files());
                }
                _ => {
                    eprintln!("[ERROR] Invalid input: '{}'. Please enter numbers from 1 to {}.", part, selectable_items.len());
                    valid_selection = false;
                    break;
                }
            }
        }
        
        if valid_selection {
            let mut unique_files = HashMap::new();
            for file in files_to_download {
                unique_files.insert(file.filename.clone(), file);
            }
            return Ok(unique_files.into_values().collect());
        }
    }
}