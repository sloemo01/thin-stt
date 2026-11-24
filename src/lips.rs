//! Silent dictation: read the mouth from the camera and decode it to text.
//!
//! Memory: camera frames are never stored whole. Each frame is reduced to
//! an 88×88 grayscale mouth crop as soon as its ±6-frame smoothing window
//! is known (≈13 frames buffered), so a 20 s clip holds ~4 MB. Nothing
//! is written to disk, and the lip model is loaded only while decoding.

use std::collections::VecDeque;

use crate::models;

pub const CROP: usize = 88;
pub const FAST_MODEL_ID: &str = "lips-fast";
pub const FLARE_MODEL_ID: &str = "lips-flare";
pub const MIN_FRAMES: usize = 10;

/// Most accurate lip model on disk.
pub fn best_model() -> Option<&'static models::ModelInfo> {
    [FLARE_MODEL_ID, FAST_MODEL_ID]
        .iter()
        .filter_map(|id| models::by_id(id))
        .find(|m| models::is_downloaded(m))
}
const PATCH_HALF: f32 = 48.0;
const CROP_PIXELS: usize = CROP * CROP;
const SMOOTH: usize = 6;
pub const MAX_FRAMES: usize = 500; // model position table: 20 s at 25 fps
const FRAME_SECS: f64 = 0.04; // the model is trained on 25 fps video
const MAX_FACELESS_PENDING: usize = 50;
// Auto-AVSR mean face (256×256): right eye, left eye, nose, mouth centre.
const REFERENCE: [[f32; 2]; 4] = [
    [102.073_94, 94.272_3],
    [156.361_3, 93.578_156],
    [129.003_74, 135.903_43],
    [129.313_37, 157.823],
];
const REFERENCE_SIZE: f32 = 256.0;

/// Four keypoints in image pixels: right eye, left eye, nose, mouth centre.
pub type Keypoints = [[f32; 2]; 4];

struct Frame {
    luma: Vec<u8>,
    w: usize,
    h: usize,
}

/// Streaming mouth-crop extractor (Auto-AVSR preprocessing).
#[derive(Default)]
pub struct Frontend {
    pending: VecDeque<Frame>,
    marks: Vec<Option<Keypoints>>,
    done: usize,
    crops: Vec<u8>,
    last_slot: Option<i64>,
    pub faces: usize,
    pub face_now: bool,
    pub openness: f32,
}

impl Frontend {
    /// Resample to 25 fps by presentation time; true when `t` opens a new slot.
    pub fn wants(&mut self, t: f64) -> bool {
        let slot = (t / FRAME_SECS).round() as i64;
        match self.last_slot {
            Some(s) if slot <= s => false,
            _ => self.frames() < MAX_FRAMES,
        }
    }

    pub fn frames(&self) -> usize {
        self.marks.len()
    }

    pub fn push(&mut self, t: f64, luma: Vec<u8>, w: usize, h: usize, marks: Option<Keypoints>, open: f32) {
        let slot = (t / FRAME_SECS).round() as i64;
        // A camera running under 25 fps leaves gaps: repeat the frame so
        // the clip keeps the model's time base.
        let repeats = match self.last_slot {
            Some(s) => (slot - s).clamp(1, 3) as usize,
            None => 1,
        };
        self.last_slot = Some(slot);
        self.face_now = marks.is_some();
        if marks.is_some() {
            self.faces += 1;
            self.openness = open;
        }
        for _ in 0..repeats {
            if self.frames() >= MAX_FRAMES {
                break;
            }
            self.marks.push(marks);
            self.pending.push_back(Frame { luma: luma.clone(), w, h });
        }
        if self.faces == 0 {
            while self.pending.len() > MAX_FACELESS_PENDING {
                self.pending.pop_front();
                self.done += 1;
            }
            return;
        }
        while self.marks.len() > self.done + SMOOTH {
            self.step();
        }
    }

    /// Flush the tail and hand back `T × 88 × 88` crops.
    pub fn finish(mut self) -> Vec<u8> {
        if self.faces > 0 {
            while self.done < self.marks.len() {
                self.step();
            }
        }
        self.crops
    }

    fn filled(&self, j: usize) -> Option<Keypoints> {
        if let Some(m) = self.marks[j] {
            return Some(m);
        }
        let prev = (0..j).rev().find_map(|i| self.marks[i].map(|m| (i, m)));
        let next = (j + 1..self.marks.len()).find_map(|i| self.marks[i].map(|m| (i, m)));
        match (prev, next) {
            (Some((a, ma)), Some((b, mb))) => {
                let f = (j - a) as f32 / (b - a) as f32;
                let mut out = ma;
                for k in 0..4 {
                    for c in 0..2 {
                        out[k][c] = ma[k][c] + f * (mb[k][c] - ma[k][c]);
                    }
                }
                Some(out)
            }
            (Some((_, m)), None) | (None, Some((_, m))) => Some(m),
            (None, None) => None,
        }
    }

    fn step(&mut self) {
        let i = self.done;
        self.done += 1;
        let Some(frame) = self.pending.pop_front() else { return };
        let n = self.marks.len();
        let m = SMOOTH.min(i).min(n - 1 - i);
        let Some(own) = self.filled(i) else { return };
        let mut smooth = [[0f32; 2]; 4];
        let mut count = 0.0;
        for j in i - m..=i + m {
            if let Some(p) = self.filled(j) {
                for k in 0..4 {
                    smooth[k][0] += p[k][0];
                    smooth[k][1] += p[k][1];
                }
                count += 1.0;
            }
        }
        let mean = |p: &Keypoints, c: usize| p.iter().map(|q| q[c]).sum::<f32>() / 4.0;
        for k in 0..4 {
            smooth[k][0] /= count;
            smooth[k][1] /= count;
        }
        let (dx, dy) = (mean(&own, 0) - mean(&smooth, 0), mean(&own, 1) - mean(&smooth, 1));
        for k in 0..4 {
            smooth[k][0] += dx;
            smooth[k][1] += dy;
        }
        crop_mouth(&frame, &smooth, &mut self.crops);
    }
}

/// Least-squares similarity transform `dst ≈ a·src + t` (a complex).
fn similarity(src: &Keypoints, dst: &Keypoints) -> [f32; 4] {
    let c = |p: &Keypoints| {
        let (x, y) = p.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0], y + q[1]));
        (x / 4.0, y / 4.0)
    };
    let (sx, sy) = c(src);
    let (dx, dy) = c(dst);
    let (mut re, mut im, mut den) = (0f32, 0f32, 0f32);
    for k in 0..4 {
        let (ax, ay) = (src[k][0] - sx, src[k][1] - sy);
        let (bx, by) = (dst[k][0] - dx, dst[k][1] - dy);
        re += bx * ax + by * ay;
        im += by * ax - bx * ay;
        den += ax * ax + ay * ay;
    }
    let (ar, ai) = (re / den.max(1e-6), im / den.max(1e-6));
    [ar, ai, dx - (ar * sx - ai * sy), dy - (ai * sx + ar * sy)]
}

fn crop_mouth(frame: &Frame, marks: &Keypoints, out: &mut Vec<u8>) {
    let [ar, ai, tx, ty] = similarity(marks, &REFERENCE);
    let mouth = marks[3];
    let cx = ar * mouth[0] - ai * mouth[1] + tx;
    let cy = ai * mouth[0] + ar * mouth[1] + ty;
    let x0 = (cx - PATCH_HALF).clamp(0.0, REFERENCE_SIZE).round() + 4.0;
    let y0 = (cy - PATCH_HALF).clamp(0.0, REFERENCE_SIZE).round() + 4.0;
    let norm = ar * ar + ai * ai;
    let (ir, ii) = (ar / norm, -ai / norm);
    for v in 0..CROP {
        for u in 0..CROP {
            let (rx, ry) = (x0 + u as f32, y0 + v as f32);
            let px = if rx >= REFERENCE_SIZE || ry >= REFERENCE_SIZE {
                0
            } else {
                let (qx, qy) = (rx - tx, ry - ty);
                sample(frame, ir * qx - ii * qy, ii * qx + ir * qy)
            };
            out.push(px);
        }
    }
}

fn sample(f: &Frame, x: f32, y: f32) -> u8 {
    if x < 0.0 || y < 0.0 || x > (f.w - 1) as f32 || y > (f.h - 1) as f32 {
        return 0;
    }
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(f.w - 1), (y0 + 1).min(f.h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |x: usize, y: usize| f.luma[y * f.w + x] as f32;
    let top = p(x0, y0) * (1.0 - fx) + p(x1, y0) * fx;
    let bot = p(x0, y1) * (1.0 - fx) + p(x1, y1) * fx;
    (top * (1.0 - fy) + bot * fy).round() as u8
}

/// Decode mouth crops with the given lip model. The ONNX session lives
/// only for this call.
pub fn transcribe(model_id: &str, crops: &[u8]) -> Result<String, String> {
    let info = models::by_id(model_id).ok_or("unknown lip model")?;
    let dir = models::model_dir(info.id);
    let tokens: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(dir.join("tokens.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let t = (crops.len() / CROP_PIXELS).min(MAX_FRAMES);
    if t == 0 {
        return Ok(String::new());
    }
    let data: Vec<f32> = crops[..t * CROP_PIXELS]
        .iter()
        .map(|&p| (p as f32 / 255.0 - 0.421) / 0.165)
        .collect();
    let mut session = ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(4)
        .map_err(|e| e.to_string())?
        .commit_from_file(dir.join(info.files[0].1))
        .map_err(|e| e.to_string())?;
    let input = ort::value::Tensor::from_array(([1usize, 1, t, CROP, CROP], data)).map_err(|e| e.to_string())?;
    let outputs = session.run(ort::inputs!["video" => input]).map_err(|e| e.to_string())?;
    let (shape, logp) = outputs["log_probs"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    let vocab = *shape.last().ok_or("bad output")? as usize;
    let eos = vocab - 1;
    let mut text = String::new();
    let mut prev = usize::MAX;
    for row in logp.chunks(vocab) {
        let best = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(0);
        if best != prev && best != 0 && best != eos {
            if let Some(piece) = tokens.get(best) {
                text.push_str(piece);
            }
        }
        prev = best;
    }
    Ok(sentence_case(&text.replace('▁', " ")))
}

fn sentence_case(raw: &str) -> String {
    let words: Vec<String> = raw
        .split_whitespace()
        .map(|w| {
            let w = w.to_lowercase();
            match w.as_str() {
                "i" | "i'm" | "i'll" | "i've" | "i'd" => w.replacen('i', "I", 1),
                _ => w,
            }
        })
        .collect();
    let mut s = words.join(" ");
    if let Some(c) = s.get(0..1) {
        s = c.to_uppercase() + &s[1..];
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_recovers_reference() {
        let t = similarity(&REFERENCE, &REFERENCE);
        assert!((t[0] - 1.0).abs() < 1e-4 && t[1].abs() < 1e-4);
    }

    #[test]
    fn sentence_case_lowercases_model_output() {
        assert_eq!(sentence_case(" HELLO I AM HERE"), "Hello I am here");
    }
}
