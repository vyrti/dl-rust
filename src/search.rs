use crate::util::{format_large_number, get_client};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::Deserialize;
use std::fmt;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct HFApiModelInfo {
    model_id: String,
    author: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    last_modified: DateTime<Utc>,
    #[serde(default)]
    tags: Vec<String>,
    pipeline_tag: Option<String>,
    private: bool,
    #[serde(default)]
    gated: GatedStatus,
}

#[derive(Debug, Default)]
enum GatedStatus {
    True,
    Auto,
    Manual,
    #[default]
    False,
}

impl fmt::Display for GatedStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GatedStatus::True => write!(f, "Gated"),
            GatedStatus::Auto => write!(f, "Gated (auto)"),
            GatedStatus::Manual => write!(f, "Gated (manual)"),
            GatedStatus::False => Ok(()),
        }
    }
}

impl<'de> Deserialize<'de> for GatedStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct GatedStatusVisitor;

        impl<'de> Visitor<'de> for GatedStatusVisitor {
            type Value = GatedStatus;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a boolean, string, or null for gated status")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if v {
                    Ok(GatedStatus::True)
                } else {
                    Ok(GatedStatus::False)
                }
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match v.to_lowercase().as_str() {
                    "auto" => Ok(GatedStatus::Auto),
                    "manual" => Ok(GatedStatus::Manual),
                    _ => Ok(GatedStatus::False),
                }
            }
            
            fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
            where
                D2: de::Deserializer<'de>,
            {
                deserializer.deserialize_any(self)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(GatedStatus::False)
            }
            
            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(GatedStatus::False)
            }
        }

        deserializer.deserialize_any(GatedStatusVisitor)
    }
}

pub async fn handle_model_search(query: &str, hf_token: &str) -> Result<()> {
    eprintln!("[INFO] Searching for models matching '{}' on Hugging Face...", query);

    let client = get_client(hf_token)?;
    let api_url = "https://huggingface.co/api/models";

    let params = [
        ("search", query),
        ("sort", "downloads"),
        ("direction", "-1"),
        ("limit", "20"),
        ("full", "true"),
    ];

    let resp = client
        .get(api_url)
        .query(&params)
        .send()
        .await
        .context("Failed to send search request to Hugging Face API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let error_body =
            resp.text().await.unwrap_or_else(|_| "Could not read error body".to_string());
        return Err(anyhow::anyhow!(
            "Hugging Face API request failed with status {}. Response: {}",
            status,
            error_body
        ));
    }

    let results: Vec<HFApiModelInfo> =
        resp.json().await.context("Failed to parse search results JSON")?;

    if results.is_empty() {
        eprintln!("[INFO] No models found matching your query '{}'.", query);
        return Ok(());
    }

    println!("\nTop {} model results for \"{}\" (sorted by downloads):", results.len(), query);
    println!("{}", "=".repeat(80));

    for (i, model) in results.iter().enumerate() {
        let author =
            model.author.as_deref().unwrap_or_else(|| model.model_id.split('/').next().unwrap_or("N/A"));

        let mut status_addons: Vec<String> = Vec::new();
        if model.private {
            status_addons.push("Private".to_string());
        }
        if !matches!(model.gated, GatedStatus::False) {
            status_addons.push(model.gated.to_string());
        }

        let task_display = model.pipeline_tag.as_deref().unwrap_or("N/A");
        let task_line = if status_addons.is_empty() {
            task_display.to_string()
        } else {
            format!("{} ({})", task_display, status_addons.join(", "))
        };

        println!("{:2}. Model ID: {}", i + 1, model.model_id);
        println!("    Author: {}", author);
        println!(
            "    Stats: Downloads: {} | Likes: {} | Updated: {}",
            format_large_number(model.downloads),
            format_large_number(model.likes),
            model.last_modified.format("%Y-%m-%d")
        );
        println!("    Task: {}", task_line);

        if !model.tags.is_empty() {
            let tags_str =
                model.tags.iter().take(10).map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
            println!("    Tags: {}", tags_str);
        }
        println!("{}", "-".repeat(40));
    }

    Ok(())
}