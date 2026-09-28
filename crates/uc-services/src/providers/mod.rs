//! One module per service. Each is behind a cargo feature of the same name (all on by default),
//! so a service can be built and tested on its own.

use crate::service::Service;

#[cfg(feature = "abacus")]
pub(crate) mod abacus;
#[cfg(feature = "aiand")]
pub(crate) mod aiand;
#[cfg(feature = "aixy")]
pub(crate) mod aixy;
#[cfg(feature = "alibaba")]
pub(crate) mod alibaba;
#[cfg(feature = "amp")]
pub(crate) mod amp;
#[cfg(feature = "anthropic")]
pub(crate) mod anthropic;
#[cfg(feature = "antigravity")]
pub(crate) mod antigravity;
#[cfg(feature = "atlascloud")]
pub(crate) mod atlascloud;
#[cfg(feature = "augment")]
pub(crate) mod augment;
#[cfg(feature = "bedrock")]
pub(crate) mod bedrock;
#[cfg(feature = "bifrost")]
pub(crate) mod bifrost;
#[cfg(feature = "cerebras")]
pub(crate) mod cerebras;
#[cfg(feature = "chutes")]
pub(crate) mod chutes;
#[cfg(feature = "clawrouter")]
pub(crate) mod clawrouter;
#[cfg(feature = "cline")]
pub(crate) mod cline;
#[cfg(feature = "cloudflare")]
pub(crate) mod cloudflare;
#[cfg(feature = "codebuddy")]
pub(crate) mod codebuddy;
#[cfg(feature = "codebuff")]
pub(crate) mod codebuff;
#[cfg(feature = "coderabbit")]
pub(crate) mod coderabbit;
#[cfg(feature = "commandcode")]
pub(crate) mod commandcode;
#[cfg(feature = "copilot")]
pub(crate) mod copilot;
#[cfg(feature = "cursor")]
pub(crate) mod cursor;
#[cfg(feature = "deepgram")]
pub(crate) mod deepgram;
#[cfg(feature = "deepinfra")]
pub(crate) mod deepinfra;
#[cfg(feature = "deepseek")]
pub(crate) mod deepseek;
#[cfg(feature = "devin")]
pub(crate) mod devin;
#[cfg(feature = "devpass")]
pub(crate) mod devpass;
#[cfg(feature = "doubao")]
pub(crate) mod doubao;
#[cfg(feature = "elevenlabs")]
pub(crate) mod elevenlabs;
#[cfg(feature = "factory")]
pub(crate) mod factory;
#[cfg(feature = "fireworks")]
pub(crate) mod fireworks;
#[cfg(feature = "gitkraken")]
pub(crate) mod gitkraken;
#[cfg(feature = "grok")]
pub(crate) mod grok;
#[cfg(feature = "groq")]
pub(crate) mod groq;
#[cfg(feature = "helmcode")]
pub(crate) mod helmcode;
#[cfg(feature = "huggingface")]
pub(crate) mod huggingface;
#[cfg(feature = "hyper")]
pub(crate) mod hyper;
#[cfg(feature = "hyperbolic")]
pub(crate) mod hyperbolic;
#[cfg(feature = "ibmbob")]
pub(crate) mod ibmbob;
#[cfg(feature = "jetbrains")]
pub(crate) mod jetbrains;
#[cfg(feature = "kilo")]
pub(crate) mod kilo;
#[cfg(feature = "kimi")]
pub(crate) mod kimi;
#[cfg(feature = "kiro")]
pub(crate) mod kiro;
#[cfg(feature = "litellm")]
pub(crate) mod litellm;
#[cfg(feature = "llmman")]
pub(crate) mod llmman;
#[cfg(feature = "llmproxy")]
pub(crate) mod llmproxy;
#[cfg(feature = "longcat")]
pub(crate) mod longcat;
#[cfg(feature = "manus")]
pub(crate) mod manus;
#[cfg(feature = "mimo")]
pub(crate) mod mimo;
#[cfg(feature = "minimax")]
pub(crate) mod minimax;
#[cfg(feature = "mistral")]
pub(crate) mod mistral;
#[cfg(feature = "moonshot")]
pub(crate) mod moonshot;
#[cfg(feature = "muse")]
pub(crate) mod muse;
#[cfg(feature = "nanogpt")]
pub(crate) mod nanogpt;
#[cfg(feature = "neuralwatt")]
pub(crate) mod neuralwatt;
#[cfg(feature = "newapi")]
pub(crate) mod newapi;
#[cfg(feature = "notion")]
pub(crate) mod notion;
#[cfg(feature = "nous")]
pub(crate) mod nous;
#[cfg(feature = "novita")]
pub(crate) mod novita;
#[cfg(feature = "ollama")]
pub(crate) mod ollama;
#[cfg(feature = "openai")]
pub(crate) mod openai;
#[cfg(feature = "opencode")]
pub(crate) mod opencode;
#[cfg(feature = "openrouter")]
pub(crate) mod openrouter;
#[cfg(feature = "perplexity")]
pub(crate) mod perplexity;
#[cfg(feature = "poe")]
pub(crate) mod poe;
#[cfg(feature = "qoder")]
pub(crate) mod qoder;
#[cfg(feature = "qwen")]
pub(crate) mod qwen;
#[cfg(feature = "raycast")]
pub(crate) mod raycast;
#[cfg(feature = "replicate")]
pub(crate) mod replicate;
#[cfg(feature = "sakana")]
pub(crate) mod sakana;
#[cfg(feature = "siliconflow")]
pub(crate) mod siliconflow;
#[cfg(feature = "stepfun")]
pub(crate) mod stepfun;
#[cfg(feature = "sub2api")]
pub(crate) mod sub2api;
#[cfg(feature = "synthetic")]
pub(crate) mod synthetic;
#[cfg(feature = "t3chat")]
pub(crate) mod t3chat;
#[cfg(feature = "typesafe")]
pub(crate) mod typesafe;
#[cfg(feature = "v0")]
pub(crate) mod v0;
#[cfg(feature = "venice")]
pub(crate) mod venice;
#[cfg(feature = "vercel")]
pub(crate) mod vercel;
#[cfg(feature = "vertexai")]
pub(crate) mod vertexai;
#[cfg(feature = "warp")]
pub(crate) mod warp;
#[cfg(feature = "wayfinder")]
pub(crate) mod wayfinder;
#[cfg(feature = "windsurf")]
pub(crate) mod windsurf;
#[cfg(feature = "xai")]
pub(crate) mod xai;
#[cfg(feature = "xkiro")]
pub(crate) mod xkiro;
#[cfg(feature = "zai")]
pub(crate) mod zai;
#[cfg(feature = "zed")]
pub(crate) mod zed;
#[cfg(feature = "zenmux")]
pub(crate) mod zenmux;
#[cfg(feature = "zoommate")]
pub(crate) mod zoommate;

/// Every service compiled in, in the order the Accounts screen lists them.
pub(crate) static ALL: &[&dyn Service] = &[
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
    #[cfg(feature = "aiand")]
    &aiand::AiAnd,
    #[cfg(feature = "aixy")]
    &aixy::Aixy,
    #[cfg(feature = "atlascloud")]
    &atlascloud::AtlasCloud,
    #[cfg(feature = "bifrost")]
    &bifrost::Bifrost,
    #[cfg(feature = "clawrouter")]
    &clawrouter::ClawRouter,
    #[cfg(feature = "cline")]
    &cline::Cline,
    #[cfg(feature = "deepgram")]
    &deepgram::Deepgram,
    #[cfg(feature = "deepinfra")]
    &deepinfra::DeepInfra,
    #[cfg(feature = "devpass")]
    &devpass::DevPass,
    #[cfg(feature = "elevenlabs")]
    &elevenlabs::ElevenLabs,
    #[cfg(feature = "fireworks")]
    &fireworks::Fireworks,
    #[cfg(feature = "gitkraken")]
    &gitkraken::GitKraken,
    #[cfg(feature = "helmcode")]
    &helmcode::Helmcode,
    #[cfg(feature = "huggingface")]
    &huggingface::HuggingFace,
    #[cfg(feature = "hyper")]
    &hyper::Hyper,
    #[cfg(feature = "litellm")]
    &litellm::LiteLLM,
    #[cfg(feature = "llmman")]
    &llmman::LlmMan,
    #[cfg(feature = "llmproxy")]
    &llmproxy::LlmProxy,
    #[cfg(feature = "manus")]
    &manus::Manus,
    #[cfg(feature = "moonshot")]
    &moonshot::Moonshot,
    #[cfg(feature = "muse")]
    &muse::Muse,
    #[cfg(feature = "neuralwatt")]
    &neuralwatt::Neuralwatt,
    #[cfg(feature = "nous")]
    &nous::Nous,
    #[cfg(feature = "perplexity")]
    &perplexity::Perplexity,
    #[cfg(feature = "poe")]
    &poe::Poe,
    #[cfg(feature = "raycast")]
    &raycast::Raycast,
    #[cfg(feature = "replicate")]
    &replicate::Replicate,
    #[cfg(feature = "sakana")]
    &sakana::Sakana,
    #[cfg(feature = "sub2api")]
    &sub2api::Sub2Api,
    #[cfg(feature = "synthetic")]
    &synthetic::Synthetic,
    #[cfg(feature = "t3chat")]
    &t3chat::T3Chat,
    #[cfg(feature = "typesafe")]
    &typesafe::TypeSafe,
    #[cfg(feature = "v0")]
    &v0::V0,
    #[cfg(feature = "venice")]
    &venice::Venice,
    #[cfg(feature = "xkiro")]
    &xkiro::XKiro,
    #[cfg(feature = "zenmux")]
    &zenmux::ZenMux,
    #[cfg(feature = "abacus")]
    &abacus::Abacus,
    #[cfg(feature = "alibaba")]
    &alibaba::Alibaba,
    #[cfg(feature = "amp")]
    &amp::Amp,
    #[cfg(feature = "augment")]
    &augment::Augment,
    #[cfg(feature = "bedrock")]
    &bedrock::Bedrock,
    #[cfg(feature = "coderabbit")]
    &coderabbit::CodeRabbit,
    #[cfg(feature = "codebuff")]
    &codebuff::Codebuff,
    #[cfg(feature = "doubao")]
    &doubao::Doubao,
    #[cfg(feature = "factory")]
    &factory::Factory,
    #[cfg(feature = "jetbrains")]
    &jetbrains::JetBrains,
    #[cfg(feature = "kilo")]
    &kilo::Kilo,
    #[cfg(feature = "longcat")]
    &longcat::LongCat,
    #[cfg(feature = "mimo")]
    &mimo::MiMo,
    #[cfg(feature = "mistral")]
    &mistral::Mistral,
    #[cfg(feature = "notion")]
    &notion::Notion,
    #[cfg(feature = "qwen")]
    &qwen::Qwen,
    #[cfg(feature = "stepfun")]
    &stepfun::StepFun,
    #[cfg(feature = "vertexai")]
    &vertexai::VertexAi,
    #[cfg(feature = "warp")]
    &warp::Warp,
    #[cfg(feature = "wayfinder")]
    &wayfinder::Wayfinder,
    #[cfg(feature = "windsurf")]
    &windsurf::Windsurf,
    #[cfg(feature = "zoommate")]
    &zoommate::ZoomMate,
    #[cfg(feature = "ibmbob")]
    &ibmbob::IbmBob,
    #[cfg(feature = "cerebras")]
    &cerebras::Cerebras,
    #[cfg(feature = "cloudflare")]
    &cloudflare::Cloudflare,
    #[cfg(feature = "novita")]
    &novita::Novita,
    #[cfg(feature = "nanogpt")]
    &nanogpt::NanoGpt,
    #[cfg(feature = "hyperbolic")]
    &hyperbolic::Hyperbolic,
    #[cfg(feature = "newapi")]
    &newapi::NewApi,
];
