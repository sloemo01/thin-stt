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
    ModelInfo {
        id: "qwen3-1.7b-polish",
        label: "Qwen3 1.7B Polish",
        badge: "GGUF • ~960MB",
        engine: "llm",
        desc: "Cleans up transcripts after dictation · loads only while polishing",
        repo: "unsloth/Qwen3-1.7B-GGUF",
        files: &[("Qwen3-1.7B-IQ4_XS.gguf", "Qwen3-1.7B-IQ4_XS.gguf")],
    },
    ModelInfo {
        id: "lips-fast",
        label: "Auto-AVSR Lips (fast)",
        badge: "ONNX • 203MB",
        engine: "lips",
        desc: "Silent lip reading from the camera · ~25% WER · research-only weights",
        repo: "eschmechel/auto-avsr-lrs3-vsr-int8-onnx",
        files: &[("lipread_ctc.int8.onnx", "lipread_ctc.int8.onnx"), ("tokens.json", "tokens.json")],
    },
];

/// Id of the transcript-polish model (engine "llm" — no hotkey).
pub const POLISH_MODEL_ID: &str = "qwen3-1.7b-polish";

/// Earlier polish-model ids — a user who already downloaded one keeps
/// polish working after an app update swaps in a newer model.
/// (dir name, gguf filename)
pub const LEGACY_POLISH_MODELS: &[(&str, &str)] = &[
    ("qwen3-0.6b-polish", "Qwen3-0.6B-Q4_K_M.gguf"),
    ("qwen3-1.7b-polish", "Qwen3-1.7B-IQ4_XS.gguf"),
    ("lfm2-1.2b-polish", "LFM2-1.2B-Q4_K_M.gguf"),
    ("lfm25-1.2b-polish", "LFM2.5-1.2B-Instruct-Q4_K_M.gguf"),
    ("qwen35-2b-polish", "Qwen3.5-2B-UD-IQ3_XXS.gguf"),
];

/// Path to the GGUF that should run the polish pass: the current model when
/// downloaded, else any legacy polish model still on disk.
pub fn polish_model_path() -> Option<PathBuf> {
    if let Some(i) = by_id(POLISH_MODEL_ID) {
        if is_downloaded(i) {
            return Some(model_dir(i.id).join(i.files[0].1));
        }
    }
    LEGACY_POLISH_MODELS
        .iter()
        .map(|(id, f)| model_dir(id).join(f))
        .find(|p| p.exists())
}

/// Directory names that hold polish models — current and every legacy one.
/// Startup cleanup must treat these as known even when not in `MODELS`.
pub fn is_polish_dir(name: &str) -> bool {
    name == POLISH_MODEL_ID
        || LEGACY_POLISH_MODELS.iter().any(|(id, _)| *id == name)
}

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
