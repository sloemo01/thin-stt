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
pub const FLARE_LLM_MODEL_ID: &str = "lips-flare-llm";
pub const MIN_FRAMES: usize = 10;

/// Most accurate lip model on disk.
pub fn best_model() -> Option<&'static models::ModelInfo> {
    [FLARE_LLM_MODEL_ID, FLARE_MODEL_ID, FAST_MODEL_ID]
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
    face_crops: Vec<u8>,
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

    /// Flush the tail and hand back the `T × 88 × 88` mouth and face crops.
    pub fn finish(mut self) -> Clip {
        if self.faces > 0 {
            while self.done < self.marks.len() {
                self.step();
            }
        }
        Clip { lips: self.crops, face: self.face_crops }
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
        crop_face(&frame, &smooth, &mut self.face_crops);
    }
}

/// Grayscale crops for one utterance; lives only in memory.
pub struct Clip {
    pub lips: Vec<u8>,
    pub face: Vec<u8>,
}

impl Clip {
    pub fn frames(&self) -> usize {
        self.lips.len() / CROP_PIXELS
    }
}

/// Whole aligned face (LRS3-style face track): the 256² reference canvas
/// resized to 96² and centre-cropped to 88².
fn crop_face(frame: &Frame, marks: &Keypoints, out: &mut Vec<u8>) {
    let [ar, ai, tx, ty] = similarity(marks, &REFERENCE);
    let norm = ar * ar + ai * ai;
    let (ir, ii) = (ar / norm, -ai / norm);
    let scale = REFERENCE_SIZE / 96.0;
    for v in 0..CROP {
        for u in 0..CROP {
            let (rx, ry) = ((u as f32 + 4.5) * scale, (v as f32 + 4.5) * scale);
            let (qx, qy) = (rx - tx, ry - ty);
            out.push(sample(frame, ir * qx - ii * qy, ii * qx + ir * qy));
        }
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
pub fn transcribe(model_id: &str, clip: &Clip) -> Result<String, String> {
    let info = models::by_id(model_id).ok_or("unknown lip model")?;
    let dir = models::model_dir(info.id);
    if info.id == FLARE_MODEL_ID {
        return flare(&dir, clip);
    }
    if info.id == FLARE_LLM_MODEL_ID {
        return flare_llm(&dir, clip);
    }
    let crops = &clip.lips;
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

fn session(path: &std::path::Path) -> Result<ort::session::Session, String> {
    ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(4)
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())
}

const FLARE_BEAM: usize = 10;
const FLARE_EOS: i64 = 2;
const FLARE_SPECIALS: usize = 4;
const ENC_DIM: usize = 768;

/// FLARE Large: joint face + lip encoder, then a beam search over the
/// lightweight Transformer decoder. Each session lives only for its stage.
/// FLARE Large's joint face + lip encoder: `[t, ENC_DIM]` features.
fn flare_encode(dir: &std::path::Path, clip: &Clip) -> Result<(Vec<f32>, usize), String> {
    use ort::value::Tensor;
    let t = clip.frames().min(MAX_FRAMES);
    if t == 0 {
        return Ok((Vec::new(), 0));
    }
    let pixels = |b: &[u8]| -> Vec<f32> { b[..t * CROP_PIXELS].iter().map(|&p| p as f32 / 255.0).collect() };
    let has_face = clip.face.len() >= t * CROP_PIXELS;
    let face = if has_face { pixels(&clip.face) } else { vec![0.0; t * CROP_PIXELS] };
    let shape = [1usize, 1, t, CROP, CROP];
    let enc: Vec<f32> = {
        let mut s = session(&dir.join("flare_enc.int8.onnx"))?;
        let out = s
            .run(ort::inputs![
                "face" => Tensor::from_array((shape, face)).map_err(|e| e.to_string())?,
                "lip" => Tensor::from_array((shape, pixels(&clip.lips))).map_err(|e| e.to_string())?,
                "face_on" => Tensor::from_array(([1usize], vec![if has_face { 1.0f32 } else { 0.0 }])).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| e.to_string())?;
        out["enc"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?.1.to_vec()
    };
    Ok((enc, t))
}

/// FLARE-LLM: FLARE Large's encoder, the modality projection, then
/// Qwen2.5-7B + LoRA. Each stage frees its model before the next loads.
fn flare_llm(dir: &std::path::Path, clip: &Clip) -> Result<String, String> {
    use ort::value::Tensor;
    let (enc, t) = flare_encode(dir, clip)?;
    if t == 0 {
        return Ok(String::new());
    }
    let (emb, dim) = {
        let mut s = session(&dir.join("flare_proj.int8.onnx"))?;
        let out = s
            .run(ort::inputs!["enc" => Tensor::from_array(([1usize, t, ENC_DIM], enc)).map_err(|e| e.to_string())?])
            .map_err(|e| e.to_string())?;
        let (shape, emb) = out["emb"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        (emb.to_vec(), *shape.last().ok_or("bad projection")? as usize)
    };
    let text = crate::lipllm::transcribe(
        &dir.join("qwen2.5-7b.i1-Q4_K_M.gguf"),
        &dir.join("flare_llm_lora.f16.gguf"),
        &emb,
        dim,
    )?;
    Ok(sentence_case(&text))
}

fn flare(dir: &std::path::Path, clip: &Clip) -> Result<String, String> {
    use ort::value::Tensor;
    let dict = std::fs::read_to_string(dir.join("flare_dict.txt")).map_err(|e| e.to_string())?;
    let words: Vec<&str> = dict.lines().filter_map(|l| l.split_whitespace().next()).collect();
    let (enc, t) = flare_encode(dir, clip)?;
    if t == 0 {
        return Ok(String::new());
    }
    let mut dec = session(&dir.join("flare_dec.int8.onnx"))?;
    let mut alive: Vec<(Vec<i64>, f32)> = vec![(vec![FLARE_EOS], 0.0)];
    let mut done: Vec<(Vec<i64>, f32)> = Vec::new();
    for _ in 0..t.min(200) {
        let (b, l) = (alive.len(), alive[0].0.len());
        let toks: Vec<i64> = alive.iter().flat_map(|h| h.0.iter().copied()).collect();
        let out = dec
            .run(ort::inputs![
                "tokens" => Tensor::from_array(([b, l], toks)).map_err(|e| e.to_string())?,
                "enc" => Tensor::from_array(([1usize, t, ENC_DIM], enc.clone())).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| e.to_string())?;
        let (shape, logp) = out["logp"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let vocab = shape[1] as usize;
        let mut cand: Vec<(usize, usize, f32)> = Vec::with_capacity(b * FLARE_BEAM);
        for (i, row) in logp.chunks(vocab).enumerate() {
            let mut idx: Vec<usize> = (0..vocab).collect();
            idx.select_nth_unstable_by(FLARE_BEAM, |&x, &y| row[y].total_cmp(&row[x]));
            cand.extend(idx[..FLARE_BEAM].iter().map(|&k| (i, k, alive[i].1 + row[k])));
        }
        cand.sort_by(|a, b| b.2.total_cmp(&a.2));
        let mut next = Vec::with_capacity(FLARE_BEAM);
        for (i, k, score) in cand {
            let mut seq = alive[i].0.clone();
            seq.push(k as i64);
            if k as i64 == FLARE_EOS {
                done.push((seq, score / (l as f32)));
            } else if next.len() < FLARE_BEAM {
                next.push((seq, score));
            }
            if next.len() == FLARE_BEAM {
                break;
            }
        }
        alive = next;
        if done.len() >= FLARE_BEAM || alive.is_empty() {
            break;
        }
    }
    drop(dec);
    if done.is_empty() {
        done = alive.into_iter().map(|(s, sc)| { let n = s.len() as f32; (s, sc / n) }).collect();
    }
    let best = done.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|d| d.0).unwrap_or_default();
    let text: String = best
        .iter()
        .filter(|&&k| k as usize >= FLARE_SPECIALS && k != FLARE_EOS)
        .filter_map(|&k| words.get(k as usize - FLARE_SPECIALS).copied())
        .collect();
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
