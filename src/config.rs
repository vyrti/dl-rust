use lazy_static::lazy_static;
use std::collections::HashMap;

pub const CURRENT_APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEVELOPMENT_VERSION: &str = "DEVELOPMENT"; // Used for local builds not matching a git tag

// Updater constants
pub const UPDATER_REPO_OWNER: &str = "vyrti";
pub const UPDATER_REPO_NAME: &str = "dl-rust";

lazy_static! {
    pub static ref MODEL_REGISTRY: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        m.insert("qwen3-0.6b", "https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf?download=true");
        m.insert("qwen3-1.7b", "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf?download=true");
        m.insert("qwen3-4b", "https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf?download=true");
        m.insert("qwen3-8b", "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf?download=true");
        m.insert("qwen3-16b", "https://huggingface.co/Qwen/Qwen3-16B-GGUF/resolve/main/Qwen3-16B-Q4_K_M.gguf?download=true");
        m.insert("qwen3-32b", "https://huggingface.co/Qwen/Qwen3-32B-GGUF/resolve/main/Qwen3-32B-Q4_K_M.gguf?download=true");
        m.insert("qwen3-30b-moe", "https://huggingface.co/Qwen/Qwen3-16B-GGUF/resolve/main/Qwen3-16B-Q4_K_M.gguf?download=true");
        m.insert("gemma3-27b", "https://huggingface.co/unsloth/gemma-3-27b-it-GGUF/resolve/main/gemma-3-27b-it-Q4_0.gguf?download=true");
        m
    };
}

pub fn get_model_registry() -> &'static HashMap<&'static str, &'static str> {
    &MODEL_REGISTRY
}

lazy_static! {
    pub static ref GGUF_SERIES_REGEX: regex::Regex =
        regex::Regex::new(r"^(.*?)-(\d{5})-of-(\d{5})\.gguf$").unwrap();
}