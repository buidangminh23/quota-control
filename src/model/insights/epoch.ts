/**
 * Epoch AI's public benchmark data (CC BY 4.0): the Epoch Capabilities Index (`eci_scores.csv`, one
 * score per model with a 90% interval) and the benchmark results it is built from
 * (`eci_benchmarks.csv`, one row per model and benchmark, every result on Epoch's 0..1 scale).
 *
 * Every benchmark is placed in one category so the tab covers each kind of work, not only code. The
 * descriptions paraphrase Epoch's own page for each benchmark (epoch.ai/benchmarks/<slug>); one that
 * is not in the catalog still shows, under "Khác", with its name only.
 */
import { parseCsv } from "./csv";

export interface EpochModel {
  name: string;
  eci: number;
  /** 90% interval of the index, when Epoch gives one. */
  low: number | null;
  high: number | null;
  /** Release date, `YYYY-MM-DD`. */
  released: string;
  organization: string;
  country: string;
  accessibility: string;
  openWeights: boolean;
}

export interface EpochResult {
  model: string;
  benchmark: string;
  /** 0..1, Epoch's scale for every benchmark. */
  performance: number;
  /** The model's release date, `YYYY-MM-DD`. */
  released: string;
}

function number(text: string | undefined): number | null {
  if (text === undefined || text.trim() === "") return null;
  const value = Number(text);
  return Number.isFinite(value) ? value : null;
}

export function parseEpochScores(csv: string | null | undefined): EpochModel[] {
  if (!csv) return [];
  const models: EpochModel[] = [];
  for (const row of parseCsv(csv)) {
    const name = (row["Display name"] || row.Model || "").trim();
    const eci = number(row.eci);
    if (!name || eci === null) continue;
    models.push({
      name,
      eci,
      low: number(row.eci_ci_low),
      high: number(row.eci_ci_high),
      released: (row.date ?? "").trim(),
      organization: (row.Organization ?? "").trim(),
      country: (row["Country (of organization)"] ?? "").trim(),
      accessibility: (row["Model accessibility"] ?? "").trim(),
      openWeights: (row["Accessibility group"] ?? "").trim() === "Open weights",
    });
  }
  return models.sort((a, b) => b.eci - a.eci);
}

export function parseEpochBenchmarks(csv: string | null | undefined): EpochResult[] {
  if (!csv) return [];
  const results: EpochResult[] = [];
  for (const row of parseCsv(csv)) {
    const model = (row.Model || row.model || "").trim();
    const benchmark = (row.benchmark ?? "").trim();
    const performance = number(row.performance);
    if (!model || !benchmark || performance === null || performance < 0 || performance > 1) continue;
    results.push({ model, benchmark, performance, released: (row.date ?? "").trim() });
  }
  return results;
}

export type BenchmarkCategory =
  | "coding"
  | "agents"
  | "work"
  | "math"
  | "science"
  | "knowledge"
  | "reasoning"
  | "language"
  | "vision"
  | "games"
  | "security"
  | "learning"
  | "other";

export const BENCHMARK_CATEGORIES: readonly BenchmarkCategory[] = [
  "coding",
  "agents",
  "work",
  "math",
  "science",
  "knowledge",
  "reasoning",
  "language",
  "vision",
  "games",
  "security",
  "learning",
  "other",
];

export interface BenchmarkInfo {
  category: BenchmarkCategory;
  /** Epoch's page: `https://epoch.ai/benchmarks/<slug>`. */
  slug: string;
  vi: string;
  en: string;
  /** A newer version replaced it; its results are kept for history. */
  supersededBy?: string;
}

export const BENCHMARKS: Readonly<Record<string, BenchmarkInfo>> = {
  "Aider polyglot": { category: "coding", slug: "aider-polyglot", vi: "Bài lập trình khó lấy từ Exercism, nền tảng học lập trình trực tuyến.", en: "Challenging programming problems from Exercism, an online programming education platform." },
  "SWE-Bench verified": { category: "coding", slug: "swe-bench-verified", vi: "Issue GitHub có thật trong các repo Python: model phải viết được bản sửa hợp lệ.", en: "GitHub issues from real Python repos: can the model implement a valid fix." },
  "GSO-Bench": { category: "coding", slug: "gso", vi: "Tối ưu hiệu năng: sửa code để chương trình chạy nhanh hơn đáng kể.", en: "Performance optimization challenges: change a program's code so it runs significantly faster." },
  DeepSWE: { category: "coding", slug: "deepswe", vi: "Việc kỹ thuật phần mềm dài hơi, viết mới hoàn toàn, trên các repo mã nguồn mở đang hoạt động.", en: "Original, long-horizon software engineering tasks written from scratch across active open-source repositories." },
  FrontierCode: { category: "coding", slug: "frontiercode", vi: "Tác tử lập trình có tạo được bản sửa đủ tốt để merge cho issue khó, có thật trong mã nguồn mở không.", en: "Can coding agents produce mergeable fixes for real, hard open-source issues." },
  MirrorCode: { category: "coding", slug: "mirrorcode", vi: "Lập trình dài hơi: viết lại trọn một chương trình mà không được xem mã nguồn gốc.", en: "Long-horizon coding: reimplement entire programs end to end without the original source." },
  "Terminal Bench": { category: "coding", slug: "terminal-bench", vi: "Làm việc bằng terminal: phải hiểu và dùng được các chương trình có sẵn.", en: "Tasks done in a computer terminal, using the programs available there." },
  WeirdML: { category: "coding", slug: "weirdml", vi: "Bài kỹ thuật machine learning khác thường ở nhiều lĩnh vực.", en: "Nonstandard machine-learning engineering tasks in a variety of domains." },
  "METR Time Horizons": { category: "agents", slug: "metr-time-horizons", vi: "Độ dài của việc dài nhất (kỹ thuật phần mềm và liên quan) model làm đúng quá nửa số lần.", en: "Length of the longest software and related task a model completes correctly more often than not." },
  OSWorld: { category: "agents", slug: "os-world", vi: "Dùng máy tính: làm việc thật trên desktop và web bằng bàn phím, chuột.", en: "Computer use: real desktop and web tasks with keyboard and mouse actions." },
  "OSWorld 2.0": { category: "agents", slug: "osworld-2", vi: "108 việc dài hơi, có thật trên desktop và web cho tác tử dùng máy tính.", en: "108 long-horizon, real-world desktop and web tasks for computer-use agents." },
  "The Agent Company": { category: "agents", slug: "the-agent-company", vi: "Tác tử phần mềm làm trọn việc thực tế trong môi trường tái lập được.", en: "End-to-end software agents attempting realistic tasks in reproducible environments." },
  "DeepResearch Bench": { category: "agents", slug: "deepresearchbench", vi: "Tìm và tổng hợp thông tin trên internet để trả lời câu hỏi.", en: "Gather and synthesize information from the internet to answer questions." },
  PostTrainBench: { category: "agents", slug: "post-train-bench", vi: "Tác tử dòng lệnh post-train một model ngôn ngữ nhỏ trong ngân sách tính toán cố định.", en: "CLI agents post-training small base language models under a fixed compute budget." },
  "APEX-Agents": { category: "work", slug: "apex-agents", vi: "Việc chuyên môn dài hơi trong ngân hàng đầu tư, tư vấn quản lý và luật doanh nghiệp.", en: "Long-horizon professional tasks in investment banking, management consulting and corporate law." },
  GDPval: { category: "work", slug: "gdpval", vi: "Việc được mô tả rõ, lấy từ một số nghề nghiệp có thật.", en: "Well-specified tasks drawn from selected real-world occupations." },
  "Remote Labor Index": { category: "work", slug: "rli", vi: "Dự án freelance từ xa có thật, có giá trị kinh tế, làm trọn từ đầu đến cuối.", en: "Real, economically valuable remote freelance projects completed end to end." },
  GSM8K: { category: "math", slug: "gsm8k", vi: "Toán đố cấp tiểu học, tính nhiều bước.", en: "Grade-school math word problems with multi-step arithmetic." },
  "MATH level 5": { category: "math", slug: "math-level-5", vi: "Mức khó nhất của bộ MATH, từ các kỳ thi AMC 10, AMC 12 và AIME.", en: "The hardest tier of MATH, from competitions like AMC 10, AMC 12 and AIME." },
  "OTIS Mock AIME 2024-2025": { category: "math", slug: "otis-mock-aime-2024-2025", vi: "45 bài toán thi đấu của OTIS, khó hơn MATH level 5, dễ hơn FrontierMath.", en: "45 competition problems from OTIS, harder than MATH Level 5, easier than FrontierMath." },
  "FrontierMath-Tiers-1-3-v2-Private": { category: "math", slug: "frontiermath-tiers-1-3", vi: "Bài toán do chuyên gia viết, từ cuối đại học đến đầu sự nghiệp nghiên cứu.", en: "Expert-written problems from advanced undergraduate to early-career research." },
  "FrontierMath-Tier-4-v2-Private": { category: "math", slug: "frontiermath-tier-4", vi: "Bài toán cấp nghiên cứu, cực kỳ khó.", en: "Exceptionally difficult research-level math problems." },
  "FrontierMath-2025-02-28-Private": { category: "math", slug: "frontiermath-tiers-1-3", vi: "Bản cũ của FrontierMath.", en: "An earlier FrontierMath release.", supersededBy: "FrontierMath-Tiers-1-3-v2-Private" },
  "FrontierMath-Tier-4-2025-07-01-Private": { category: "math", slug: "frontiermath-tier-4", vi: "Bản cũ của FrontierMath Tier 4.", en: "An earlier FrontierMath Tier 4 release.", supersededBy: "FrontierMath-Tier-4-v2-Private" },
  ProofBench: { category: "math", slug: "proofbench", vi: "Bài toán cấp cao học: phải viết chứng minh Lean 4 qua được kiểm chứng hình thức.", en: "Graduate-level problems proved in Lean 4 that must pass formal verification." },
  "GPQA diamond": { category: "science", slug: "gpqa-diamond", vi: "Trắc nghiệm khó về sinh, hoá, lý do chuyên gia trình độ tiến sĩ soạn.", en: "Hard multiple-choice biology, chemistry and physics questions written by PhD-level experts." },
  ScienceQA: { category: "science", slug: "science-qa", vi: "Trắc nghiệm khoa học có chữ, hình ảnh và sơ đồ.", en: "Multimodal multiple-choice science questions with text, images and diagrams." },
  "ARC AI2": { category: "science", slug: "arc-ai2", vi: "Trắc nghiệm kiến thức và suy luận khoa học cấp phổ thông.", en: "Grade-school science knowledge and reasoning, multiple choice." },
  OpenBookQA: { category: "science", slug: "open-book-qa", vi: "Hỏi đáp khoa học: kết hợp một kiến thức cốt lõi với hiểu biết thông thường.", en: "Open-book science questions combining a core fact with commonsense." },
  "Surface Evolver Bench": { category: "science", slug: "surface-evolver-bench", vi: "Tác tử viết mô phỏng Surface Evolver cho bề mặt chất lỏng định hình bởi sức căng bề mặt.", en: "Agents write Surface Evolver simulations of liquid surfaces shaped by surface tension." },
  HLE: { category: "knowledge", slug: "hle", vi: "2.500 câu hỏi của chuyên gia, trải hơn 100 môn học, đòi kiến thức chuyên sâu.", en: "2,500 expert-written questions across 100+ subjects that need deep, specialized knowledge." },
  MMLU: { category: "knowledge", slug: "mmlu", vi: "Đề kiểu thi trải hàng chục môn học thuật và nghề nghiệp.", en: "Exam-style questions across dozens of academic and professional subjects." },
  TriviaQA: { category: "knowledge", slug: "trivia-qa", vi: "Câu hỏi đố vui khó, kèm tài liệu dẫn chứng.", en: "Challenging trivia questions paired with evidence documents." },
  "SimpleQA Verified": { category: "knowledge", slug: "simpleqa-verified", vi: "1.000 câu hỏi dữ kiện về chính trị, khoa học, nghệ thuật, thể thao, địa lý, âm nhạc…", en: "1,000 factoid questions on politics, science, art, sports, geography, music and more." },
  "ARC-AGI": { category: "reasoning", slug: "arc-agi", vi: "Học khái niệm từ vài ví dụ: tìm quy luật từ các cặp đầu vào, đầu ra.", en: "Few-shot concept learning: induce the pattern from input-output examples." },
  "ARC-AGI-2": { category: "reasoning", slug: "arc-agi-2", vi: "Bản khó hơn của ARC-AGI, có tính cả mức tính toán cho mỗi bài giải được.", en: "A harder ARC-AGI that also weighs the compute spent per solved task." },
  BBH: { category: "reasoning", slug: "bbh", vi: "Các bài khó nhất của BIG-bench: suy luận nhiều bước, ký hiệu, kết hợp.", en: "The hardest BIG-bench tasks: compositional, symbolic, multi-step reasoning." },
  SimpleBench: { category: "reasoning", slug: "simplebench", vi: "Lẽ thường, câu hỏi mẹo và tình huống cần hiểu không gian, thời gian, xã hội.", en: "Common sense, trick questions, and situations that need space, time or social cues." },
  HellaSwag: { category: "reasoning", slug: "hella-swag", vi: "Hoàn thành câu theo lẽ thường trong tình huống hằng ngày.", en: "Commonsense sentence completion in everyday scenarios." },
  PIQA: { category: "reasoning", slug: "piqa", vi: "Lẽ thường vật lý: chọn cách làm khả thi hơn cho việc hằng ngày.", en: "Physical common sense: pick the more feasible way to solve an everyday problem." },
  Winogrande: { category: "reasoning", slug: "wino-grande", vi: "Xác định đại từ chỉ ai, cần hiểu biết thông thường.", en: "Pronoun resolution that needs common sense." },
  ANLI: { category: "reasoning", slug: "adversarial-nli", vi: "Suy luận ngôn ngữ tự nhiên với ví dụ được thu thập theo lối đối kháng.", en: "Natural language inference on adversarially collected examples." },
  DTBench: { category: "reasoning", slug: "dtbench", vi: "Trắc nghiệm soạn tay về lý thuyết quyết định trong bài toán kiểu Newcomb.", en: "Handcrafted questions on the decision theory of Newcomb-like problems." },
  LMCA: { category: "reasoning", slug: "lmca", vi: "Chấm chất lượng lập luận khái niệm, so với đánh giá của chuyên gia.", en: "Judging the quality of conceptual arguments against expert ratings." },
  "Lech Mazur Writing": { category: "language", slug: "lech-mazur-writing", vi: "Viết truyện ngắn gài đủ 10 yếu tố cho trước, hội đồng model chấm 0 đến 10. Đã đóng băng từ 8/2025.", en: "Short stories weaving ten assigned elements, graded 0-10 by LLM judges. Frozen since August 2025." },
  "Fiction.LiveBench": { category: "language", slug: "fictionlivebench", vi: "Hiểu các tác phẩm văn chương dài.", en: "Understanding long creative writing pieces." },
  LAMBADA: { category: "language", slug: "lambada", vi: "Đoán từ cuối của đoạn văn từ ngữ cảnh rộng.", en: "Predict the final word of a passage from its broader context." },
  GeoBench: { category: "vision", slug: "geobench", vi: "Đoán nơi chụp một bức ảnh, dựa trên trò GeoGuessr.", en: "Identify where a photo was taken, based on GeoGuessr." },
  VPCT: { category: "vision", slug: "vpct", vi: "Hiểu sơ đồ bóng lăn xuống các dốc rồi rơi vào xô.", en: "Read diagrams of balls rolling down ramps into buckets." },
  CadEval: { category: "vision", slug: "cad-eval", vi: "Từ mô tả chữ sang CAD: tạo thiết kế 3D tham số hợp lệ, qua kiểm tra hình học và kết xuất.", en: "Text to CAD: valid parametric 3D designs that pass geometry and rendering checks." },
  "Furniture Assembly": { category: "vision", slug: "furniture-assembly", vi: "Suy luận không gian: xem ảnh lắp đồ IKEA, nói có đúng không và chỉ ra bước sai.", en: "Spatial reasoning: judge IKEA builds from photos and find the mistaken step." },
  "Chess Puzzles": { category: "games", slug: "chess-puzzles", vi: "100 thế cờ mới do engine tạo, mỗi thế có đúng một nước đi tốt nhất.", en: "100 novel engine-generated puzzles, each with a single best move." },
  "Mystery Game Puzzles": { category: "games", slug: "mystery-game-puzzles", vi: "100 thế từ một biến thể giải đố của một trò chơi nổi tiếng; chi tiết cố ý giữ kín.", en: "100 positions from a puzzle variant of a well-known game whose details stay undisclosed." },
  Balrog: { category: "games", slug: "balrog", vi: "Chơi nhiều trò chơi có độ khó rất khác nhau.", en: "Playing a series of games of widely varying difficulty." },
  Cybench: { category: "security", slug: "cybench", vi: "Tác tử tự tìm và khai thác lỗ hổng trong các thử thách cô lập.", en: "Agents autonomously find and exploit vulnerabilities in sandboxed challenges." },
  ExploitBench: { category: "security", slug: "exploitbench", vi: "Tác tử leo nấc thang khai thác phần mềm trên lỗ hổng thật đã được gia cố.", en: "How far agents climb a ladder of exploitation against real, hardened vulnerabilities." },
  "CL-bench": { category: "learning", slug: "cl-bench", vi: "Học kiến thức hoàn toàn mới từ ngữ cảnh rồi áp dụng vào việc do chuyên gia thiết kế.", en: "Learn genuinely new knowledge from context, then apply it to expert-designed tasks." },
  "CL-bench Life": { category: "learning", slug: "cl-bench-life", vi: "Học và suy luận từ ngữ cảnh đời thường lộn xộn: tin nhắn, ghi chú rời rạc, dấu vết hành vi.", en: "Learn from messy real-life context: everyday messages, scattered notes, behavioral traces." },
  "EBR-bench": { category: "learning", slug: "ebr-bench", vi: "Đo khả năng học: điểm có tăng qua nhiều lần chơi lại trò Earthborne Rangers không.", en: "Learning: do scores improve across repeated playthroughs of Earthborne Rangers." },
};

export function benchmarkInfo(name: string): BenchmarkInfo {
  return BENCHMARKS[name] ?? { category: "other", slug: "", vi: "", en: "" };
}

export function benchmarkUrl(name: string): string | null {
  const slug = BENCHMARKS[name]?.slug;
  return slug ? `https://epoch.ai/benchmarks/${slug}` : null;
}

/** A readable benchmark name; FrontierMath's internal names become its public ones. */
export function benchmarkLabel(name: string): string {
  switch (name) {
    case "FrontierMath-Tiers-1-3-v2-Private":
      return "FrontierMath Tiers 1-3";
    case "FrontierMath-Tier-4-v2-Private":
      return "FrontierMath Tier 4";
    case "FrontierMath-2025-02-28-Private":
      return "FrontierMath (02/2025)";
    case "FrontierMath-Tier-4-2025-07-01-Private":
      return "FrontierMath Tier 4 (07/2025)";
    default:
      return name;
  }
}

export interface BenchmarkBoard {
  name: string;
  info: BenchmarkInfo;
  /** Best first. */
  results: EpochResult[];
  /** Newest model release date with a result. */
  latestRelease: string;
  /** No model released in the year before `asOf` has a result, or a newer version replaced it. */
  dated: boolean;
}

function yearBefore(asOf: Date): string {
  const cutoff = new Date(asOf.getFullYear() - 1, asOf.getMonth(), asOf.getDate());
  return `${cutoff.getFullYear()}-${String(cutoff.getMonth() + 1).padStart(2, "0")}-${String(cutoff.getDate()).padStart(2, "0")}`;
}

/** Every benchmark with its results, grouped by category in catalog order, current ones first. */
export function benchmarkBoards(results: readonly EpochResult[], asOf: Date): BenchmarkBoard[] {
  const byName = new Map<string, EpochResult[]>();
  for (const result of results) {
    const list = byName.get(result.benchmark);
    if (list) list.push(result);
    else byName.set(result.benchmark, [result]);
  }
  const cutoff = yearBefore(asOf);
  const boards = [...byName.entries()].map(([name, list]) => {
    const info = benchmarkInfo(name);
    const latestRelease = list.reduce((latest, result) => (result.released > latest ? result.released : latest), "");
    return {
      name,
      info,
      results: [...list].sort((a, b) => b.performance - a.performance || a.model.localeCompare(b.model)),
      latestRelease,
      dated: Boolean(info.supersededBy) || latestRelease < cutoff,
    };
  });
  const order = (board: BenchmarkBoard) => BENCHMARK_CATEGORIES.indexOf(board.info.category);
  return boards.sort((a, b) => order(a) - order(b) || Number(a.dated) - Number(b.dated) || b.results.length - a.results.length || a.name.localeCompare(b.name));
}
