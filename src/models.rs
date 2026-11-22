use std::{fs, path::PathBuf};

use crate::config;

#[derive(Clone)]
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub badge: &'static str,
    pub engine: &'static str, // "whisper" | "parakeet"
    pub desc: &'static str,   // one-line speed/accuracy hint shown in the UI
    pub repo: &'static str,
    pub files: &'static [(&'static str, &'static str)], // (remote path, local name)
}

const PARAKEET_FILES: &[(&str, &str)] = &[
    ("encoder-model.int8.onnx", "encoder-model.int8.onnx"),
    ("decoder_joint-model.int8.onnx", "decoder_joint-model.int8.onnx"),
    ("nemo128.onnx", "nemo128.onnx"),
    ("vocab.txt", "vocab.txt"),
];

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "whisper-tiny",
        label: "Whisper Tiny",
        badge: "GGML • 75MB",
        engine: "whisper",
        desc: "Fastest · lighter accuracy",
        repo: "ggerganov/whisper.cpp",
        files: &[("ggml-tiny.bin", "ggml-tiny.bin")],
    },
    ModelInfo {
        id: "whisper-base",
        label: "Whisper Base",
        badge: "GGML • 142MB",
        engine: "whisper",
        desc: "Fast · lighter accuracy",
        repo: "ggerganov/whisper.cpp",
        files: &[("ggml-base.bin", "ggml-base.bin")],
    },
    ModelInfo {
        id: "whisper-small",
        label: "Whisper Small",
        badge: "GGML • 466MB",
        engine: "whisper",
        desc: "Balanced speed and accuracy",
        repo: "ggerganov/whisper.cpp",
        files: &[("ggml-small.bin", "ggml-small.bin")],
    },
    ModelInfo {
        id: "whisper-large-v3-turbo",
        label: "Whisper Large v3 Turbo",
        badge: "GGML • 1.6GB",
        engine: "whisper",
        desc: "Best Whisper accuracy · heavier",
        repo: "ggerganov/whisper.cpp",
        files: &[("ggml-large-v3-turbo.bin", "ggml-large-v3-turbo.bin")],
    },
    ModelInfo {
        id: "parakeet-tdt-v2",
        label: "Parakeet TDT v2",
        badge: "ONNX • 632MB",
        engine: "parakeet",
        desc: "Very fast · English only",
        repo: "istupakov/parakeet-tdt-0.6b-v2-onnx",
        files: PARAKEET_FILES,
    },
    ModelInfo {
        id: "parakeet-tdt-v3",
        label: "Parakeet TDT v3",
        badge: "ONNX • 641MB",
        engine: "parakeet",
        desc: "Very fast · best Parakeet accuracy · English only",
        repo: "istupakov/parakeet-tdt-0.6b-v3-onnx",
        files: PARAKEET_FILES,
    },
];

pub fn by_id(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn model_dir(id: &str) -> PathBuf {
    config::models_dir().join(id)
}

pub fn is_downloaded(m: &ModelInfo) -> bool {
    let dir = model_dir(m.id);
    m.files.iter().all(|(_, local)| {
        let p = dir.join(local);
        p.is_file() && fs::metadata(&p).map(|md| md.len() > 0).unwrap_or(false)
    })
}

pub fn disk_size_mb(id: &str) -> f64 {
    let dir = model_dir(id);
    if !dir.is_dir() {
        return 0.0;
    }
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    total += md.len();
                }
            }
        }
    }
    total as f64 / (1024.0 * 1024.0)
}

/// Query huggingface for the size (bytes) of just the files we download.
pub fn remote_size_bytes(repo: &str, wanted: &[&str]) -> Option<u64> {
    let url = format!("https://huggingface.co/api/models/{}?blobs=true", repo);
    let body = ureq::get(&url).call().ok()?.body_mut().read_to_string().ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let mut total = 0u64;
    if let Some(sibs) = json.get("siblings").and_then(|s| s.as_array()) {
        for s in sibs {
            let name = s.get("rfilename").and_then(|v| v.as_str()).unwrap_or("");
            if wanted.contains(&name) {
                if let Some(sz) = s.get("size").and_then(|v| v.as_u64()) {
                    total += sz;
                }
            }
        }
    }
    if total == 0 {
        None
    } else {
        Some(total)
    }
}

pub fn download_url(m: &ModelInfo, remote: &str) -> String {
    format!("https://huggingface.co/{}/resolve/main/{}", m.repo, remote)
}
