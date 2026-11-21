use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Active microphone capture. Samples are accumulated as f32 mono at the
/// device's native rate; `stop` resamples to 16 kHz.
///
/// cpal's CoreAudio stream is !Send — the Recorder must live on one thread.
/// The pump owns it: start/stop/drop all happen on the main thread.
pub struct Recorder {
    stream: Mutex<Option<cpal::Stream>>,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
}

fn downmix(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks_exact(channels)
        .map(|c| c.iter().sum::<f32>() / channels as f32)
        .collect()
}

impl Recorder {
    pub fn start(device_name: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .ok_or_else(|| format!("microphone '{}' not found", name))?,
            None => host
                .default_input_device()
                .ok_or_else(|| "no default input device".to_string())?,
        };

        // Prefer 16 kHz mono f32; otherwise take the device default.
        let mut cfg: Option<cpal::SupportedStreamConfig> = None;
        if let Ok(configs) = device.supported_input_configs() {
            for c in configs {
                if c.channels() == 1
                    && c.sample_format() == cpal::SampleFormat::F32
                    && c.min_sample_rate() <= cpal::SampleRate(16000)
                    && c.max_sample_rate() >= cpal::SampleRate(16000)
                {
                    cfg = Some(c.with_sample_rate(cpal::SampleRate(16000)));
                    break;
                }
            }
        }
        let config = match cfg {
            Some(c) => c,
            None => {
                let def = device.default_input_config().map_err(|e| e.to_string())?;
                if def.channels() > 2 {
                    return Err("unsupported channel count".to_string());
                }
                def
            }
        };
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        log::info!(
            "recording '{}' at {}Hz x{}ch ({:?})",
            device.name().unwrap_or_default(),
            sample_rate,
            channels,
            config.sample_format()
        );

        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let samples_cb = samples.clone();
        let err_cb = |e| log::error!("audio stream error: {}", e);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.config(),
                move |data: &[f32], _| {
                    if let Ok(mut buf) = samples_cb.lock() {
                        buf.extend(downmix(data, channels));
                    }
                },
                err_cb,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.config(),
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|v| *v as f32 / 32768.0).collect();
                    if let Ok(mut buf) = samples_cb.lock() {
                        buf.extend(downmix(&f, channels));
                    }
                },
                err_cb,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.config(),
                move |data: &[u16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|v| (*v as f32 - 32768.0) / 32768.0)
                        .collect();
                    if let Ok(mut buf) = samples_cb.lock() {
                        buf.extend(downmix(&f, channels));
                    }
                },
                err_cb,
                None,
            ),
            fmt => return Err(format!("unsupported sample format {:?}", fmt)),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;

        Ok(Recorder {
            stream: Mutex::new(Some(stream)),
            samples,
            sample_rate,
        })
    }

    /// Stop recording and return 16kHz mono f32 samples. Nothing is
    /// persisted — dictated audio never touches disk.
    pub fn stop(&self) -> Vec<f32> {
        if let Some(stream) = self.stream.lock().unwrap().take() {
            let _ = stream.pause();
            drop(stream);
        }
        let raw = std::mem::take(&mut *self.samples.lock().unwrap());
        resample(&raw, self.sample_rate, 16000)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.lock().unwrap().take() {
            let _ = stream.pause();
        }
    }
}

/// Linear-interpolation resampler.
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = (samples.len() as f64 * ratio).max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let idx = src.floor() as usize;
        let frac = (src - idx as f64) as f32;
        let a = samples.get(idx).copied().unwrap_or(0.0);
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    if let Some(d) = host.default_input_device() {
        if let Ok(n) = d.name() {
            names.push(n);
        }
    }
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            if let Ok(n) = d.name() {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
    }
    names
}
