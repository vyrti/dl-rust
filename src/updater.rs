use crate::config::{CURRENT_APP_VERSION, DEVELOPMENT_VERSION, UPDATER_REPO_NAME, UPDATER_REPO_OWNER};
use crate::util;
use anyhow::{anyhow, Context, Result};
use log::{debug, info};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
struct GHAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Deserialize, Debug)]
struct GHRelease {
    tag_name: String,
    name: String,
    assets: Vec<GHAsset>,
}

fn platform_arch_to_asset_name() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    let name = match (os, arch) {
        ("linux", "x86_64") => "dl.linux.x64",
        ("linux", "aarch64") => "dl.linux.arm",
        ("windows", "x86_64") => "dl.win.x64.exe",
        ("windows", "aarch64") => "dl.win.arm.exe",
        ("macos", "x86_64") => "dl.apple.intel",
        ("macos", "aarch64") => "dl.apple.arm",
        _ => return Err(anyhow!("Unsupported platform for auto-update: {}/{}", os, arch)),
    };
    Ok(name.to_string())
}

async fn fetch_latest_release() -> Result<GHRelease> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        UPDATER_REPO_OWNER, UPDATER_REPO_NAME
    );
    debug!("Fetching latest release from {}", url);
    let client = util::get_client("")?;
    let release = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json::<GHRelease>()
        .await?;
    Ok(release)
}

async fn download_update(url: &str, dest_path: &PathBuf, size: u64) -> Result<()> {
    let client = util::get_client("")?;
    let mut resp = client.get(url).send().await?.error_for_status()?;
    
    let pb = ProgressBar::new(size);
    pb.set_style(ProgressStyle::with_template("{spinner:.green} Downloading update [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")?);

    let mut dest_file = fs::File::create(dest_path)?;
    let mut downloaded: u64 = 0;

    while let Some(chunk) = resp.chunk().await? {
        dest_file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }
    
    pb.finish_with_message("Download complete.");
    Ok(())
}

pub async fn handle_update() -> Result<()> {
    info!("Starting self-update process.");
    eprintln!("[INFO] Checking for updates...");

    let target_asset_name = platform_arch_to_asset_name()?;
    debug!("Target asset for this platform: {}", target_asset_name);

    let release = fetch_latest_release().await.context("Could not fetch update information")?;
    info!("Latest release is '{}' with tag '{}'", release.name, release.tag_name);
    
    let current_version = if CURRENT_APP_VERSION == "0.0.0" { DEVELOPMENT_VERSION } else { CURRENT_APP_VERSION };
    
    let should_update = if current_version == DEVELOPMENT_VERSION {
        eprintln!("[INFO] Running a development build. The latest release is {}.", release.tag_name);
        true
    } else {
        let current_v = semver::Version::parse(current_version.trim_start_matches('v'))?;
        let latest_v = semver::Version::parse(release.tag_name.trim_start_matches('v'))?;
        if latest_v > current_v {
            eprintln!("[INFO] A new version {} is available (current: {}).", latest_v, current_v);
            true
        } else {
            eprintln!("[INFO] Your version ({}) is up to date.", current_v);
            false
        }
    };
    
    if !should_update {
        return Ok(());
    }

    if let Some(asset) = release.assets.iter().find(|a| a.name == target_asset_name) {
        eprintln!(
            "[INFO] Found update: {} (Version: {}, Size: {})",
            asset.name, release.tag_name, util::format_bytes(asset.size)
        );
        
        let current_exe = env::current_exe()?;
        let update_dir = current_exe.parent().unwrap();
        let temp_path = update_dir.join(format!("{}.new", asset.name));
        
        download_update(&asset.browser_download_url, &temp_path, asset.size).await?;
        
        // On unix, set executable permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&temp_path, perms)?;
        }
        
        eprintln!("[INFO] Applying update...");
        self_replace::self_replace(&temp_path).map_err(|e| anyhow!("Failed to apply update: {}", e))?;
        fs::remove_file(&temp_path)?;
        
        eprintln!("[SUCCESS] Update applied! Please restart the application.");
        Ok(())
    } else {
        Err(anyhow!("No update asset found for your platform in the latest release."))
    }
}