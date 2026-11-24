//! Optional local-LLM polish pass: a small instruct model loads only
//! while cleaning a transcript and drops immediately after, so its RAM
//! is held for the duration of one dictation and nothing else.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::OnceLock;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::{params::LlamaModelParams, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

const SYSTEM: &str = "Rewrite the raw speech-to-text transcript as clean written text. \
Rules: capitalize sentence starts and \"I\"; fix punctuation and obvious transcription \
errors; remove ALL filler (um, uh, like, you know, sort of, kind of) and false starts; \
never delete or merge the transcript's existing punctuation — periods and commas the \
raw text already carries must survive into the output; \
never summarize, answer questions, or add your own content; when the speaker \
corrects themselves (\"remove X\", \"I meant\", \"no wait\", \"actually\", \"scratch that\", \
\"make that\", \"or rather\", \"delete that\"), \
output their final intended statement as a complete sentence — the correction replaces \
what came before and the correction phrases themselves are dropped; convert every \
spoken symbol and key name to its character: \"dot\"/\"period\"/\"full stop\" → \".\", \
\"comma\" → \",\", \"question mark\" → \"?\", \"exclamation (mark/point)\" → \"!\", \
\"colon\" → \":\", \"semicolon\" → \";\", \"dash\"/\"hyphen\" → \"-\", \"underscore\" → \"_\", \
\"slash\"/\"forward slash\" → \"/\", \"backslash\" → \"\\\\\", \"asterisk\"/\"star\" → \"*\", \
\"tilde\" → \"~\", \"at sign\" → \"@\" (bare \"at\" too inside emails), \"hashtag\"/\"hash\"/\
\"pound sign\" → \"#\", \"dollar sign\" → \"$\", \"percent (sign)\" → \"%\", \"caret\" → \"^\", \
\"ampersand\"/\"and sign\" → \"&\", \"plus sign\" → \"+\", \"equals (sign)\" → \"=\", \
\"pipe\"/\"vertical bar\" → \"|\", \"backtick\"/\"grave\" → \"`\", \"apostrophe\" → \"'\", \
text between \"quote\" and \"end quote\" goes in \"...\", \"open/close paren\" → \"(\"/\")\", \
\"open/close bracket\" → \"[\"/\"]\", \"open/close brace\" → \"{\"/\"}\", \
\"less-than sign\"/\"greater-than sign\" → \"<\"/\">\"; spoken keys act like presses: \
\"enter\"/\"return\"/\"new line\" start a new line, \"new paragraph\" adds a blank line, \
\"tab\" inserts a tab, \"backspace\"/\"delete that\" removes the last phrase; caps \
commands: text after \"all caps\" goes uppercase until \"end caps\", \"lowercase\" \
lowercases; formatting commands apply markdown: \"bullet point\"/\"bulleted list\" \
makes \"- \" items, \"numbered list\" or \"first ... second ... third\" makes a \
numbered list, \"heading\"/\"title\" makes a \"# \" heading, \"checklist\" makes \
\"- [ ]\" items, \"bold\"/\"italic\" wraps the span in ** or *; spoken emails and \
URLs become real addresses (\"john dot doe at gmail dot com\" becomes \
john.doe@gmail.com); write numbers as digits (\"five thirty p m\" becomes 5:30 PM, \
\"twenty dollars\" becomes $20, \"january fifth twenty twenty four\" becomes \
January 5, 2024); keep the speaker's words and meaning otherwise. Output ONLY the \
cleaned text — no preamble, no quotes.";

const MAX_NEW_TOKENS: usize = 1024;

// llama.cpp only initializes a backend once per process.
pub fn init_backend() {
    backend();
}

fn backend() -> &'static LlamaBackend {
    static B: OnceLock<LlamaBackend> = OnceLock::new();
    B.get_or_init(|| LlamaBackend::init().expect("llama backend"))
}

/// Load the GGUF at `path`, polish `text`, drop the model before returning.
pub fn polish(path: &Path, text: &str) -> Result<String, String> {
    let model = LlamaModel::load_from_file(backend(), path, &LlamaModelParams::default())
        .map_err(|e| e.to_string())?;
    let tmpl = model.chat_template(None).map_err(|e| e.to_string())?;
    // One worked example steadies the 0.6B model's formatting.
    let messages = [
        LlamaChatMessage::new("system".into(), SYSTEM.into()).map_err(|e| e.to_string())?,
        LlamaChatMessage::new(
            "user".into(),
            "Raw transcript:\num so like i uh finished the thing yesterday and jane wasnt there\n/no_think"
                .into(),
        )
        .map_err(|e| e.to_string())?,
        LlamaChatMessage::new(
            "assistant".into(),
            "So I finished the thing yesterday, and Jane wasn't there.".into(),
        )
        .map_err(|e| e.to_string())?,
        LlamaChatMessage::new(
            "user".into(),
            "Raw transcript:\nsend the invoice to jane dot smith at acme dot com before friday\n/no_think"
                .into(),
        )
        .map_err(|e| e.to_string())?,
        LlamaChatMessage::new(
            "assistant".into(),
            "Send the invoice to jane.smith@acme.com before Friday.".into(),
        )
        .map_err(|e| e.to_string())?,
        LlamaChatMessage::new(
            "user".into(),
            "Raw transcript:\nmeet me at five no wait i meant six on friday\n/no_think".into(),
        )
        .map_err(|e| e.to_string())?,
        LlamaChatMessage::new(
            "assistant".into(),
            "Meet me at six on Friday.".into(),
        )
        .map_err(|e| e.to_string())?,
        LlamaChatMessage::new(
            "user".into(),
            // "/no_think" keeps Qwen3 out of reasoning mode — thinking
            // burns the token budget and adds seconds per dictation.
            format!("Raw transcript:\n{}\n/no_think", text),
        )
        .map_err(|e| e.to_string())?,
    ];
    let prompt = model
        .apply_chat_template(&tmpl, &messages, true)
        .map_err(|e| e.to_string())?;

    let vocab = model.vocab();
    let tokens = vocab.tokenize(prompt.as_bytes(), true, true);
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(4096))
        // n_batch defaults to 2048; decoding a >2048-token prompt batch asserts.
        .with_n_batch(4096);
    let mut ctx = model
        .new_context(backend(), ctx_params)
        .map_err(|e| e.to_string())?;

    let mut batch = LlamaBatch::new(4096, 1);
    let last = tokens.len().saturating_sub(1);
    for (i, t) in tokens.iter().enumerate() {
        batch.add(*t, i as i32, &[0], i == last).map_err(|e| e.to_string())?;
    }
    ctx.decode(&mut batch).map_err(|e| e.to_string())?;

    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.2),
        LlamaSampler::dist(0),
    ]);
    sampler.accept_many(tokens.iter().copied());

    let mut out_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
    let mut pos = tokens.len() as i32;
    let mut finished = false;
    for _ in 0..MAX_NEW_TOKENS {
        let t = sampler.sample(&ctx, -1);
        if vocab.is_eog(t) {
            finished = true;
            break;
        }
        sampler.accept(t);
        out_tokens.push(t);
        batch.clear();
        batch.add(t, pos, &[0], true).map_err(|e| e.to_string())?;
        pos += 1;
        ctx.decode(&mut batch).map_err(|e| e.to_string())?;
    }
    if !finished {
        // Ran out of budget — a half-cleaned transcript is worse than
        // the raw one, so fail and let the caller type the original.
        return Err("polish hit token limit".into());
    }

    let out = vocab.detokenize(&out_tokens, false, true);
    let text_out = String::from_utf8_lossy(&out);
    // Defend against template tags or reasoning leaks into the reply —
    // and only strip a real markdown fence, never dictated backticks or
    // dictated edge whitespace.
    let s = text_out
        .split("<|im_end|>")
        .next()
        .unwrap_or("")
        .split("</think>")
        .last()
        .unwrap_or("");
    // an empty think block can leak as a literal leading "<think>"
    let s = s.strip_prefix("<think>").unwrap_or(s);
    let cleaned = if let Some(rest) = s.strip_prefix("```") {
        let inner = rest.splitn(2, '\n').nth(1).unwrap_or("");
        inner
            .strip_suffix("```")
            .unwrap_or(inner)
            .trim_end_matches('\n')
    } else {
        s.trim_matches('\n')
    };
    Ok(cleaned.to_string())
}
