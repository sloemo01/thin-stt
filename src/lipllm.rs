//! FLARE-LLM decoding: visual embeddings spliced into Qwen2.5-7B (with the
//! FLARE LoRA) through raw llama.cpp, since llama-cpp-2 can't feed embeddings.
//! The model, context and adapter live only for one call.

use std::ffi::CString;
use std::os::raw::c_char;
use std::path::Path;

use llama_cpp_sys_2 as sys;

const EOS: sys::llama_token = 151643;
const PROMPT: &str = "transcribe the video: ";
const N_CTX: u32 = 1024;
const MAX_NEW_TOKENS: usize = 120;

struct Llm {
    model: *mut sys::llama_model,
    ctx: *mut sys::llama_context,
}

impl Drop for Llm {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                sys::llama_free(self.ctx);
            }
            // Frees the LoRA adapter too.
            sys::llama_model_free(self.model);
        }
    }
}

fn cpath(p: &Path) -> Result<CString, String> {
    CString::new(p.to_string_lossy().as_bytes()).map_err(|e| e.to_string())
}

unsafe fn decode(ctx: *mut sys::llama_context, batch: sys::llama_batch) -> Result<(), String> {
    let rc = sys::llama_decode(ctx, batch);
    sys::llama_batch_free(batch);
    if rc == 0 { Ok(()) } else { Err(format!("llama_decode failed ({})", rc)) }
}

unsafe fn feed_tokens(ctx: *mut sys::llama_context, toks: &[sys::llama_token], pos: &mut i32, logits: bool) -> Result<(), String> {
    let n = toks.len();
    let mut b = sys::llama_batch_init(n as i32, 0, 1);
    for (i, &t) in toks.iter().enumerate() {
        *b.token.add(i) = t;
        *b.pos.add(i) = *pos + i as i32;
        *b.n_seq_id.add(i) = 1;
        **b.seq_id.add(i) = 0;
        *b.logits.add(i) = (logits && i + 1 == n) as i8;
    }
    b.n_tokens = n as i32;
    *pos += n as i32;
    decode(ctx, b)
}

unsafe fn feed_embd(ctx: *mut sys::llama_context, emb: &[f32], dim: usize, pos: &mut i32) -> Result<(), String> {
    let n = emb.len() / dim;
    let mut b = sys::llama_batch_init(n as i32, dim as i32, 1);
    std::ptr::copy_nonoverlapping(emb.as_ptr(), b.embd, n * dim);
    for i in 0..n {
        *b.pos.add(i) = *pos + i as i32;
        *b.n_seq_id.add(i) = 1;
        **b.seq_id.add(i) = 0;
        *b.logits.add(i) = 0;
    }
    b.n_tokens = n as i32;
    *pos += n as i32;
    decode(ctx, b)
}

unsafe fn tokenize(vocab: *const sys::llama_vocab, s: &str) -> Vec<sys::llama_token> {
    let mut v = vec![0; s.len() + 8];
    let n = sys::llama_tokenize(vocab, s.as_ptr() as *const c_char, s.len() as i32, v.as_mut_ptr(), v.len() as i32, false, false);
    v.truncate(n.max(0) as usize);
    v
}

/// `emb` is `[T, dim]` projected visual features. Greedy decode.
pub fn transcribe(gguf: &Path, lora: &Path, emb: &[f32], dim: usize) -> Result<String, String> {
    crate::llm::init_backend();
    let (gguf, lora) = (cpath(gguf)?, cpath(lora)?);
    unsafe {
        let mut mp = sys::llama_model_default_params();
        mp.n_gpu_layers = 999;
        let model = sys::llama_model_load_from_file(gguf.as_ptr(), mp);
        if model.is_null() {
            return Err("couldn't load the FLARE-LLM base model".into());
        }
        let mut llm = Llm { model, ctx: std::ptr::null_mut() };
        let mut cp = sys::llama_context_default_params();
        cp.n_ctx = N_CTX;
        cp.n_batch = N_CTX;
        cp.n_ubatch = N_CTX;
        llm.ctx = sys::llama_init_from_model(model, cp);
        if llm.ctx.is_null() {
            return Err("couldn't create the FLARE-LLM context".into());
        }
        let ad = sys::llama_adapter_lora_init(model, lora.as_ptr());
        if ad.is_null() {
            return Err("couldn't load the FLARE-LLM LoRA".into());
        }
        let mut ads = [ad];
        let mut scales = [1.0f32];
        sys::llama_set_adapters_lora(llm.ctx, ads.as_mut_ptr(), 1, scales.as_mut_ptr());

        let vocab = sys::llama_model_get_vocab(model);
        let n_vocab = sys::llama_vocab_n_tokens(vocab) as usize;
        let t = (emb.len() / dim).min(N_CTX as usize - MAX_NEW_TOKENS - 16);
        let mut pos = 0;
        feed_tokens(llm.ctx, &tokenize(vocab, "A"), &mut pos, false)?;
        feed_embd(llm.ctx, &emb[..t * dim], dim, &mut pos)?;
        let mut prompt = tokenize(vocab, PROMPT);
        prompt.push(EOS);
        feed_tokens(llm.ctx, &prompt, &mut pos, true)?;

        let mut bytes = Vec::new();
        for _ in 0..MAX_NEW_TOKENS {
            let logits = std::slice::from_raw_parts(sys::llama_get_logits_ith(llm.ctx, -1), n_vocab);
            let best = logits
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(EOS, |(i, _)| i as sys::llama_token);
            if best == EOS || sys::llama_vocab_is_eog(vocab, best) {
                break;
            }
            let mut buf = [0u8; 64];
            let n = sys::llama_token_to_piece(vocab, best, buf.as_mut_ptr() as *mut c_char, buf.len() as i32, 0, false);
            bytes.extend_from_slice(&buf[..n.max(0) as usize]);
            feed_tokens(llm.ctx, &[best], &mut pos, true)?;
        }
        Ok(String::from_utf8_lossy(&bytes).trim().to_string())
    }
}
