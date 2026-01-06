use anyhow::Result;
use path_clean::PathClean;
use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Formats a size in bytes into a human-readable string (KB, MB, GB, etc. - base 10).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 9] = ["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    if bytes < 1000 { // Use 1000 for base-10 Kilo
        return format!("{} B", bytes);
    }
    let mut num = bytes as f64;
    let mut unit_idx = 0;
    while num >= 1000.0 && unit_idx < UNITS.len() - 1 {
        num /= 1000.0;
        unit_idx += 1;
    }
    format!("{:.2} {}", num, UNITS[unit_idx])
}

/// Formats a duration into a human-readable string (e.g., "10 min", "1 hr 30 min", "5 sec").
pub fn format_duration_human(duration: Duration, show_seconds: bool) -> String {
    let total_seconds = duration.as_secs_f64();

    if total_seconds <= 0.0 {
        return "N/A".to_string();
    }

    if show_seconds {
        if total_seconds < 1.0 {
            return "<1 sec".to_string();
        }
        if total_seconds < 60.0 {
            return format!("{:.0} sec", total_seconds.ceil());
        }
        if total_seconds < 3600.0 {
            let minutes = (total_seconds / 60.0).floor();
            let seconds = (total_seconds % 60.0).ceil();
            if seconds == 60.0 {
                return format!("{:.0} min 0 sec", minutes + 1.0);
            }
            return format!("{:.0} min {:.0} sec", minutes, seconds);
        }
        let hours = (total_seconds / 3600.0).floor();
        let remainder_minutes = (total_seconds % 3600.0) / 60.0;
        let minutes = remainder_minutes.floor();
        let seconds = (remainder_minutes % 1.0 * 60.0).ceil();
        if seconds == 60.0 {
            if minutes == 59.0 {
                return format!("{:.0} hr 0 min 0 sec", hours + 1.0);
            }
            return format!("{:.0} hr {:.0} min 0 sec", hours, minutes + 1.0);
        }
        return format!("{:.0} hr {:.0} min {:.0} sec", hours, minutes, seconds);
    } else {
        if total_seconds < 60.0 {
            return "<1 min".to_string();
        }
        if total_seconds < 3600.0 {
            let minutes = (total_seconds / 60.0).round();
            return format!("{:.0} min", minutes);
        }
        let hours = (total_seconds / 3600.0).floor();
        let minutes = ((total_seconds % 3600.0) / 60.0).round();
        if minutes == 60.0 {
            return format!("{:.0} hr 0 min", hours + 1.0);
        }
        return format!("{:.0} hr {:.0} min", hours, minutes);
    }
}


/// Formats large integers into a more readable string (e.g., 1.2K, 3.4M).
pub fn format_large_number(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        return format!("{:.1}K", n as f64 / 1_000.0);
    }
    if n < 1_000_000_000 {
        return format!("{:.1}M", n as f64 / 1_000_000.0);
    }
    format!("{:.1}B", n as f64 / 1_000_000_000.0)
}

/// Generates a safe and predictable local filename from a URL and an optional preferred name.
pub fn generate_actual_filename(url_str: &str, preferred_name: Option<&str>) -> String {
    let file_name = if let Some(name) = preferred_name {
        let clean_name = PathBuf::from(name).clean();
        if clean_name.is_absolute()
            || clean_name.starts_with("..")
            || clean_name.components().any(|c| c == std::path::Component::ParentDir)
        {
            eprintln!(
                "[WARN] Preferred name '{}' (cleaned to '{}') attempts path traversal or is absolute. Using only its base name.",
                name,
                clean_name.display()
            );
            clean_name
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            clean_name.to_string_lossy().to_string()
        }
    } else if let Ok(parsed_url) = url::Url::parse(url_str) {
        Path::new(parsed_url.path())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    } else {
        Path::new(url_str)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    };

    if file_name.is_empty()
        || file_name == "."
        || file_name == "/"
        || file_name.starts_with('?')
    {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fallback_name = format!("download_{:x}", timestamp);

        let ext = Path::new(&file_name)
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty() && s.len() < 7 && !s.contains(&['?', '=', '&', '/', '\\', '*', '"', '<', '>', '|'][..]))
            .map(|s| format!(".{}", s));

        let final_name = if let Some(e) = ext {
            fallback_name + &e
        } else {
            fallback_name + ".file"
        };
        eprintln!(
            "[WARN] Could not determine a valid filename for URL '{}' (preferred: {:?}). Using fallback: {}",
            url_str,
            preferred_name,
            final_name
        );
        final_name
    } else {
        file_name
    }
}

/// A panic hook that logs the panic information before the program exits.
pub fn log_panic(info: &PanicHookInfo<'_>) {
    // Ensure the cursor is visible
    let term = console::Term::stdout();
    let _ = term.show_cursor();

    let backtrace = Backtrace::capture();

    // Print to stderr for immediate visibility
    eprintln!("\n====================================================");
    eprintln!("              APPLICATION PANICKED");
    eprintln!("====================================================");
    eprintln!("{}\n", info);
    eprintln!("Backtrace:\n{}", backtrace);
    eprintln!("====================================================");
    eprintln!("Please report this bug. If debug logging was enabled,");
    eprintln!("attach the 'log.log' file to your report.");


    // Also log to the file if the logger is available
    log::error!("----------------------------------------------------");
    log::error!("                    PANIC                           ");
    log::error!("----------------------------------------------------");
    log::error!("{}\n", info);
    log::error!("Backtrace:\n{}", backtrace);
}


/// Creates a reqwest client with a default user agent and optional auth token.
pub fn get_client(hf_token: &str) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        "dl-rust-downloader/0.1".parse()?,
    );
    if !hf_token.is_empty() {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", hf_token).parse()?,
        );
    }

    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()?)
}

/// Shortens an error message to a maximum length.
pub fn shorten_error(err: &anyhow::Error, max_len: usize) -> String {
    let s = format!("{}", err);
    if s.chars().count() > max_len {
        if max_len <= 3 {
            s.chars().take(max_len).collect()
        } else {
            format!("{}...", s.chars().take(max_len - 3).collect::<String>())
        }
    } else {
        s
    }
}

/// Cleans a repository ID string to be used as a directory name.
pub fn repo_id_to_safe_path(repo_id: &str) -> String {
    let cleaned_repo_input = repo_id
        .trim_start_matches("https://huggingface.co/")
        .trim_start_matches("http://huggingface.co/");
    
    let parts: Vec<&str> = cleaned_repo_input.split('/').collect();
    if parts.len() >= 2 {
        let owner = sanitize_filename(parts[0]);
        let repo_name = sanitize_filename(parts[1]);
        format!("{}_{}", owner, repo_name)
    } else {
        format!("hf_{}", sanitize_filename(cleaned_repo_input))
    }
}

/// Removes characters that are problematic in filenames.
pub fn sanitize_filename(name: &str) -> String {
    name.replace(&['/', '\\', ':', '*', '?', '"', '<', '>', '|'][..], "_")
}