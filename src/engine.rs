use std::collections::HashMap;
use std::path::PathBuf;

use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::whisper_cpp::WhisperEngine;
use transcribe_rs::{SpeechModel, TranscribeOptions};

use crate::models;

pub struct Engine {
    loaded: HashMap<String, Box<dyn SpeechModel>>,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            loaded: HashMap::new(),
        }
    }

    /// Transcribe 16kHz mono f32 samples with the given model id.
    /// Loads the model on first use and keeps it warm.
    pub fn transcribe(&mut self, model_id: &str, samples: &[f32]) -> Result<String, String> {
        if !self.loaded.contains_key(model_id) {
            let model = load_model(model_id)?;
            self.loaded.insert(model_id.to_string(), model);
        }
        let model = self
            .loaded
            .get_mut(model_id)
            .ok_or_else(|| "model not loaded".to_string())?;
        let result = model
            .transcribe(samples, &TranscribeOptions::default())
            .map_err(|e| format!("{:?}", e))?;
        Ok(result.text.trim().to_string())
    }

    pub fn drop_model(&mut self, model_id: &str) {
        self.loaded.remove(model_id);
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
