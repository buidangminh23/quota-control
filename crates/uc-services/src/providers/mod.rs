//! One module per service. Each is behind a cargo feature of the same name (all on by default),
//! so a service can be built and tested on its own.

use crate::service::Service;

#[cfg(feature = "anthropic")]
pub(crate) mod anthropic;
#[cfg(feature = "antigravity")]
pub(crate) mod antigravity;
#[cfg(feature = "chutes")]
pub(crate) mod chutes;
#[cfg(feature = "codebuddy")]
pub(crate) mod codebuddy;
#[cfg(feature = "commandcode")]
pub(crate) mod commandcode;
#[cfg(feature = "copilot")]
pub(crate) mod copilot;
#[cfg(feature = "cursor")]
pub(crate) mod cursor;
#[cfg(feature = "deepseek")]
pub(crate) mod deepseek;
#[cfg(feature = "devin")]
pub(crate) mod devin;
#[cfg(feature = "gemini")]
pub(crate) mod gemini;
#[cfg(feature = "grok")]
pub(crate) mod grok;
#[cfg(feature = "groq")]
pub(crate) mod groq;
#[cfg(feature = "kimi")]
pub(crate) mod kimi;
#[cfg(feature = "kiro")]
pub(crate) mod kiro;
#[cfg(feature = "minimax")]
pub(crate) mod minimax;
#[cfg(feature = "ollama")]
pub(crate) mod ollama;
#[cfg(feature = "openai")]
pub(crate) mod openai;
#[cfg(feature = "opencode")]
pub(crate) mod opencode;
#[cfg(feature = "openrouter")]
pub(crate) mod openrouter;
#[cfg(feature = "qoder")]
pub(crate) mod qoder;
#[cfg(feature = "siliconflow")]
pub(crate) mod siliconflow;
#[cfg(feature = "vercel")]
pub(crate) mod vercel;
#[cfg(feature = "xai")]
pub(crate) mod xai;
#[cfg(feature = "zai")]
pub(crate) mod zai;
#[cfg(feature = "zed")]
pub(crate) mod zed;

/// Every service compiled in, in the order the Accounts screen lists them.
pub(crate) static ALL: &[&dyn Service] = &[
    #[cfg(feature = "gemini")]
    &gemini::Gemini,
    #[cfg(feature = "antigravity")]
    &antigravity::Antigravity,
    #[cfg(feature = "copilot")]
    &copilot::Copilot,
    #[cfg(feature = "cursor")]
    &cursor::Cursor,
    #[cfg(feature = "kiro")]
    &kiro::Kiro,
    #[cfg(feature = "grok")]
    &grok::Grok,
    #[cfg(feature = "opencode")]
    &opencode::OpenCode,
    #[cfg(feature = "ollama")]
    &ollama::Ollama,
    #[cfg(feature = "devin")]
    &devin::Devin,
    #[cfg(feature = "zed")]
    &zed::Zed,
    #[cfg(feature = "qoder")]
    &qoder::Qoder,
    #[cfg(feature = "codebuddy")]
    &codebuddy::CodeBuddy,
    #[cfg(feature = "zai")]
    &zai::Zai,
    #[cfg(feature = "minimax")]
    &minimax::MiniMax,
    #[cfg(feature = "kimi")]
    &kimi::Kimi,
    #[cfg(feature = "deepseek")]
    &deepseek::DeepSeek,
    #[cfg(feature = "openrouter")]
    &openrouter::OpenRouter,
    #[cfg(feature = "groq")]
    &groq::Groq,
    #[cfg(feature = "vercel")]
    &vercel::Vercel,
    #[cfg(feature = "siliconflow")]
    &siliconflow::SiliconFlow,
    #[cfg(feature = "chutes")]
    &chutes::Chutes,
    #[cfg(feature = "commandcode")]
    &commandcode::CommandCode,
    #[cfg(feature = "xai")]
    &xai::Xai,
    #[cfg(feature = "openai")]
    &openai::OpenAi,
    #[cfg(feature = "anthropic")]
    &anthropic::Anthropic,
];
