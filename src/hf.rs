use crate::util::get_client;
use anyhow::{Context, Result};
use log::debug;
use serde::Deserialize;
use urlencoding::encode;

#[derive(Deserialize, Debug, Clone)]
pub struct HFFile {
    pub url: String,
    #[serde(rename = "rfilename")]
    pub filename: String,
}

#[derive(Deserialize, Debug)]
struct Sibling {
    rfilename: String,
}

#[derive(Deserialize, Debug)]
struct RepoInfo {
    siblings: Vec<Sibling>,
}

pub async fn fetch_hugging_face_urls(repo_id: &str, hf_token: &str) -> Result<Vec<HFFile>> {
    let repo_id_clean = repo_id
        .trim_start_matches("https://huggingface.co/")
        .trim_start_matches("http://huggingface.co/");

    let api_url = format!("https://huggingface.co/api/models/{}", repo_id_clean);
    debug!("Fetching HF repo info from: {}", api_url);

    let client = get_client(hf_token)?;
    let resp = client
        .get(&api_url)
        .send()
        .await
        .with_context(|| format!("Failed to send request to HF API at {}", api_url))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let error_body = resp.text().await.unwrap_or_else(|_| "Could not read error body".to_string());
        return Err(anyhow::anyhow!(
            "Hugging Face API request failed with status {}. Response: {}",
            status,
            error_body
        ));
    }
    
    let repo_info = resp
        .json::<RepoInfo>()
        .await
        .with_context(|| "Failed to decode JSON response from Hugging Face API")?;

    let branch = "main";
    let hf_files: Vec<HFFile> = repo_info
        .siblings
        .into_iter()
        .map(|sibling| {
            let safe_rfilename_path = sibling
                .rfilename
                .split('/')
                .map(encode)
                .collect::<Vec<_>>()
                .join("/");
            let url = format!(
                "https://huggingface.co/{}/resolve/{}/{}?download=true",
                repo_id_clean, branch, safe_rfilename_path
            );
            HFFile {
                url,
                filename: sibling.rfilename,
            }
        })
        .collect();

    debug!("Found {} files in repo {}", hf_files.len(), repo_id);
    Ok(hf_files)
}