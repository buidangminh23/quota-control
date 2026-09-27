/**
 * What the Token ring can measure and each brand's own color. The ring itself reads the usage ledger
 * (`usage.ts`); the palette is upstream `TotalSpendPalette`, which also tints each brand's mark.
 */
import type { TotalSpendMetric } from "./format";

export const TOTAL_SPEND_METRICS: readonly TotalSpendMetric[] = ["cost", "costPerMtok", "tokens"];

type Hex = `#${string}`;

/**
 * Brand tints keyed by brand (never by rank); `[light, dark]`. The first ones are upstream
 * `TotalSpendPalette`; the rest come from each brand's own logo colors, darkened for a light
 * background or lightened for a dark one where the logo color is too faint there.
 */
const BRAND_COLORS: Readonly<Record<string, readonly [Hex, Hex]>> = {
  claude: ["#DE7356", "#DE7356"],
  codex: ["#10A37F", "#10A37F"],
  cursor: ["#13120A", "#F5F5F7"],
  grok: ["#8E8E93", "#98989D"],
  opencode: ["#6E6E73", "#AEAEB2"],
  openrouter: ["#6467F2", "#6467F2"],
  antigravity: ["#4285F4", "#4285F4"],
  copilot: ["#A855F7", "#A855F7"],
  amp: ["#F34E3F", "#F34E3F"],
  factory: ["#48484A", "#C7C7CC"],
  kimi: ["#0A66FF", "#0A66FF"],
  minimax: ["#F5433C", "#F5433C"],
  zai: ["#2D2D2D", "#D1D1D6"],
  abacus: ["#1C9DD9", "#38BDF8"],
  aiand: ["#1F1F1F", "#E5E5EA"],
  aixy: ["#123650", "#4A7FA8"],
  alibaba: ["#FF6A00", "#FF7F24"],
  anthropic: ["#191919", "#F0EEE6"],
  atlascloud: ["#2563EB", "#60A5FA"],
  augment: ["#6366F1", "#818CF8"],
  bedrock: ["#FF9900", "#FFAC33"],
  bifrost: ["#2A9D80", "#33C09E"],
  cerebras: ["#F15A29", "#F47B53"],
  chutes: ["#3184FF", "#5B9DFF"],
  clawrouter: ["#596EF6", "#7A8BF8"],
  cline: ["#1F1F1F", "#E5E5EA"],
  cloudflare: ["#F38020", "#F6923F"],
  codebuddy: ["#6C4DFF", "#8A70FF"],
  codebuff: ["#2E9E00", "#44FF00"],
  coderabbit: ["#FF5C35", "#FF7A59"],
  commandcode: ["#A04DFD", "#B574FD"],
  deepgram: ["#0A121B", "#E6E8EA"],
  deepinfra: ["#2A3275", "#5699DB"],
  deepseek: ["#4D6BFE", "#6F87FE"],
  devin: ["#1E9E6A", "#46B482"],
  devpass: ["#1F1F1F", "#E5E5EA"],
  doubao: ["#3370FF", "#5C8DFF"],
  elevenlabs: ["#1A1A1A", "#EBEBE6"],
  fireworks: ["#5019C5", "#7B4BE0"],
  gemini: ["#3186FF", "#5B9DFF"],
  gitkraken: ["#179287", "#1DB9AB"],
  groq: ["#F55036", "#F55036"],
  helmcode: ["#4934E1", "#6B5AE7"],
  huggingface: ["#E8A500", "#FFD21E"],
  hyper: ["#E040E0", "#FF60FF"],
  hyperbolic: ["#594CE9", "#7C72EE"],
  ibmbob: ["#0E61FA", "#4D8BFB"],
  jetbrains: ["#FF318C", "#FF5AA4"],
  kilo: ["#F27027", "#F58A4C"],
  kiro: ["#9046FF", "#A66BFF"],
  litellm: ["#4C89F0", "#6E9FF3"],
  llmman: ["#4FA590", "#6CC5B0"],
  llmproxy: ["#1F9A6B", "#24B47E"],
  longcat: ["#18B83F", "#29E154"],
  manus: ["#34322D", "#E8E6E1"],
  mimo: ["#FF6900", "#FF8533"],
  mistral: ["#FA500F", "#FF6A2B"],
  moonshot: ["#16191E", "#E8E8ED"],
  muse: ["#0668E1", "#3A8AE8"],
  nanogpt: ["#2563EB", "#60A5FA"],
  neuralwatt: ["#20B06E", "#38D98C"],
  newapi: ["#B02EE8", "#C738FB"],
  notion: ["#191919", "#EDEDED"],
  nous: ["#B8863B", "#D6A55C"],
  novita: ["#1BAF65", "#23D57C"],
  ollama: ["#1F1F1F", "#E5E5EA"],
  openai: ["#0D0D0D", "#ECECEC"],
  perplexity: ["#1FB8CD", "#22C7DD"],
  poe: ["#8364FF", "#9C85FF"],
  qoder: ["#13B347", "#2ADB5C"],
  qwen: ["#615CED", "#7F7BF2"],
  raycast: ["#FF6363", "#FF7A7A"],
  replicate: ["#111111", "#EDEDED"],
  sakana: ["#E10600", "#FF3B30"],
  siliconflow: ["#6E29F6", "#8C57F8"],
  stepfun: ["#0160FF", "#3D87FF"],
  sub2api: ["#1FA3B3", "#2DC6D8"],
  synthetic: ["#141414", "#EBEBEB"],
  t3chat: ["#F56647", "#F7806A"],
  typesafe: ["#111111", "#EDEDED"],
  v0: ["#111111", "#EDEDED"],
  venice: ["#E05A2D", "#E8744D"],
  vercel: ["#111111", "#EDEDED"],
  vertexai: ["#4285F4", "#669DF6"],
  warp: ["#7B6FB0", "#A39BCB"],
  wayfinder: ["#0E8C6D", "#10A37F"],
  windsurf: ["#0FB88F", "#34E8BB"],
  xai: ["#1D1D1F", "#F5F5F7"],
  xkiro: ["#9046FF", "#A66BFF"],
  zed: ["#084EFF", "#409CFF"],
  zenmux: ["#1F1F1F", "#E5E5EA"],
  zoommate: ["#0B5CFF", "#4D87FF"],
};

/** A brand's own color in the given appearance (it also tints the brand's mark), or `null` when the palette has none. */
export function knownBrandColor(brand: string, dark: boolean): Hex | null {
  const known = BRAND_COLORS[brand];
  if (!known) return null;
  return dark ? known[1] : known[0];
}
