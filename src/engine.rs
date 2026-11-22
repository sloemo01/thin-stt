use std::path::PathBuf;

use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::whisper_cpp::WhisperEngine;
use transcribe_rs::{SpeechModel, TranscribeOptions};

use crate::models;

pub struct Engine;

impl Engine {
    pub fn new() -> Self {
        Engine
    }

    /// Transcribe 16kHz mono f32 samples with the given model id.
    /// The model is loaded for this call and dropped on return, so a
    /// model only occupies RAM while it is actively transcribing.
    pub fn transcribe(&mut self, model_id: &str, samples: &[f32]) -> Result<String, String> {
        let mut model = load_model(model_id)?;
        // Pin the language: auto-detect on short clips mis-fires badly
        // (English speech comes back as Welsh/German). Single-language
        // engines ignore the hint.
        let opts = TranscribeOptions {
            language: Some("en".into()),
            ..Default::default()
        };
        let result = model
            .transcribe(samples, &opts)
            .map_err(|e| format!("{:?}", e))?;
        Ok(result.text.trim().to_string())
    }
}

fn load_model(model_id: &str) -> Result<Box<dyn SpeechModel>, String> {
    let info = models::by_id(model_id).ok_or_else(|| format!("unknown model {}", model_id))?;
    let dir = models::model_dir(model_id);

    match info.engine {
        "whisper" => {
            let model_path: PathBuf = dir.join(info.files[0].1);
            WhisperEngine::load(&model_path)
                .map(|e| Box::new(e) as Box<dyn SpeechModel>)
                .map_err(|e| format!("{:?}", e))
        }
        "parakeet" => ParakeetModel::load(&dir, &Quantization::Int8)
            .map(|m| Box::new(m) as Box<dyn SpeechModel>)
            .map_err(|e| format!("{:?}", e)),
        other => Err(format!("unknown engine {}", other)),
    }
}
