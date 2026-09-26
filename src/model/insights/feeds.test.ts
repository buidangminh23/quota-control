import { parseArena, parseArena3d } from "./arena";
import { compareCandidates, compareRows, searchCandidates, type CompareInput } from "./compare";
import { parseCsv } from "./csv";
import { benchmarkBoards, benchmarkLabel, benchmarkUrl, BENCHMARKS, parseEpochBenchmarks, parseEpochScores } from "./epoch";
import { EMPTY_COUNTS, modelQuality } from "./quality";
import { activeWatch, announcementPattern, currentWait, forecastResets, parseResets, parseResetStatus, resetCalendar, resetStats, xUrl, type CodexReset } from "./resets";

describe("parseCsv", () => {
  it("reads quoted fields with commas, quotes and line breaks, CRLF and a byte-order mark", () => {
    const text = '﻿Model,Note\r\n"GPT-5.6 Sol","says ""hi"", then\nleaves"\r\nClaude Opus 5,\r\n\r\n';
    expect(parseCsv(text)).toEqual([
      { Model: "GPT-5.6 Sol", Note: 'says "hi", then\nleaves' },
      { Model: "Claude Opus 5", Note: "" },
    ]);
    expect(parseCsv("")).toEqual([]);
  });
});

const SCORES = [
  "Model,Display name,eci,eci_ci_low,eci_ci_high,date,Organization,Country (of organization),Model accessibility,Accessibility group,model_versions",
  "GPT-6 Astra,GPT-6 Astra,166.6,163.0,172.03,2026-09-03,OpenAI,United States of America,API access,Closed weights,",
  "Kimi K3,Kimi K3,157.68,155.12,160.65,2026-07-16,Moonshot,China,Open weights (non-commercial),Open weights,",
  "Claude Opus 5,Claude Opus 5,162.67,,,2026-07-24,Anthropic,United States of America,API access,Closed weights,",
  "Broken,Broken,,1,2,2026-01-01,X,Y,Z,Closed weights,",
].join("\n");

const RESULTS = [
  "model_id,benchmark_id,performance,benchmark,benchmark_release_date,model,model_version,Model,date,source",
  "m1,b13,0.944,GPQA diamond,2023-11-20,GPT-6 Astra,v,GPT-6 Astra,2026-09-03,",
  "m2,b13,0.91,GPQA diamond,2023-11-20,Claude Opus 5,v,Claude Opus 5,2026-07-24,",
  "m1,b19,0.74,CadEval,2025-04-22,GPT-6 Astra,v,Old Model,2025-04-16,",
  "m2,b99,0.5,Some New Bench,2026-09-01,Claude Opus 5,v,Claude Opus 5,2026-07-24,",
  "m1,b98,1.5,Out Of Range,2026-09-01,GPT-6 Astra,v,GPT-6 Astra,2026-09-03,",
].join("\n");

describe("Epoch AI", () => {
  it("ranks the index with its interval and marks open weights", () => {
    const models = parseEpochScores(SCORES);
    expect(models.map((model) => model.name)).toEqual(["GPT-6 Astra", "Claude Opus 5", "Kimi K3"]);
    expect(models[0]).toMatchObject({ eci: 166.6, low: 163, high: 172.03, organization: "OpenAI", openWeights: false });
    expect(models[1]).toMatchObject({ low: null, high: null });
    expect(models[2]!.openWeights).toBe(true);
  });

  it("keeps results on the 0..1 scale and groups them by category, dated benchmarks after current ones", () => {
    const results = parseEpochBenchmarks(RESULTS);
    expect(results).toHaveLength(4);
    const boards = benchmarkBoards(results, new Date(2026, 8, 26));
    expect(boards.map((board) => [board.name, board.info.category, board.dated])).toEqual([
      ["GPQA diamond", "science", false],
      ["CadEval", "vision", true],
      ["Some New Bench", "other", false],
    ]);
    expect(boards[0]!.results.map((result) => result.model)).toEqual(["GPT-6 Astra", "Claude Opus 5"]);
  });

  it("describes and links every benchmark Epoch publishes today", () => {
    expect(Object.keys(BENCHMARKS)).toHaveLength(59);
    for (const [name, info] of Object.entries(BENCHMARKS)) {
      expect(info.vi, name).not.toBe("");
      expect(info.en, name).not.toBe("");
      expect(benchmarkUrl(name), name).toMatch(/^https:\/\/epoch\.ai\/benchmarks\/[a-z0-9-]+$/);
    }
    expect(benchmarkLabel("FrontierMath-Tier-4-v2-Private")).toBe("FrontierMath Tier 4");
    expect(benchmarkUrl("Unknown")).toBeNull();
  });
});

const ARENA = JSON.stringify({
  date: "2026-09-26",
  boards: {
    code: {
      meta: { last_updated: "Sep 25, 2026", source_url: "https://arena.ai/leaderboard/code" },
      models: [
        { ci: 11, license: "proprietary", model: "gpt-6-astra-max", rank: 2, score: 1792, vendor: "OpenAI", votes: 4908 },
        { ci: 18, license: "proprietary", model: "claude-opus-5.5-max", rank: 1, score: 1827, vendor: "Anthropic", votes: 1607 },
        { ci: 7, license: "proprietary", model: "claude-opus-5-high", rank: 7, score: 1662, vendor: "Anthropic", votes: 18930 },
        { ci: 9, license: "open", model: "glm-5.3-max", rank: 18, score: 1619, vendor: "Z.ai", votes: 6410 },
      ],
    },
    agent: {
      meta: { dimensions: ["Net Improvement", "Tool Hallucination"], last_updated: "Sep 25, 2026", source_url: "https://arena.ai/leaderboard/agent" },
      models: [
        { model: "GPT 6 Astra (Max)", rank: 2, sessions: 11353, vendor: "OpenAI", scores: [{ ci: 2.29, name: "Net Improvement", score: 10.85 }] },
        { model: "Claude Opus 5 (High)", rank: 3, sessions: 26539, vendor: "Anthropic", scores: [{ ci: 1.35, name: "Net Improvement", score: 9.8 }] },
      ],
    },
    "made-up": { models: [{ model: "x", rank: 1, score: 1 }] },
    "image-edit": { meta: {}, models: [] },
  },
});

describe("Arena", () => {
  it("reads the rating boards and the agent board, ignoring unknown and empty ones", () => {
    const snapshot = parseArena(ARENA)!;
    expect(snapshot.date).toBe("2026-09-26");
    expect(snapshot.boards.map((board) => board.slug)).toEqual(["code", "agent"]);
    const code = snapshot.boards[0]!;
    expect(code.entries.map((entry) => entry.model)).toEqual(["claude-opus-5.5-max", "gpt-6-astra-max", "claude-opus-5-high", "glm-5.3-max"]);
    expect(code.entries[3]).toMatchObject({ openWeights: true, ci: 9, votes: 6410 });
    expect(code.sourceUrl).toBe("https://arena.ai/leaderboard/code");
    expect(snapshot.boards[1]!.agentEntries[0]).toMatchObject({ model: "GPT 6 Astra (Max)", sessions: 11353 });
    expect(parseArena("not json")).toBeNull();
  });

  it("reads 3D Arena, best first", () => {
    const entries = parseArena3d(JSON.stringify([{ name: "TRELLIS", rank: "5", score: 1304, votes: 5777, open_source: true }, { name: "PicGen3D", rank: "1", score: 1416, votes: 4371, open_source: false }, { name: "", score: 1 }]));
    expect(entries).toEqual([
      { model: "PicGen3D", rank: 1, score: 1416, votes: 4371, openSource: false },
      { model: "TRELLIS", rank: 5, score: 1304, votes: 5777, openSource: true },
    ]);
  });
});

describe("compare", () => {
  const input: CompareInput = {
    quality: [
      modelQuality("claude", "claude-opus-5", { ...EMPTY_COUNTS, turns: 2328, humanTurns: 1226, interruptedTurns: 9, verifiedTurns: 281, greenTurns: 218, edits: 2361, failedEdits: 12 }),
      modelQuality("codex", "gpt-6-astra", { ...EMPTY_COUNTS, turns: 881, humanTurns: 409, interruptedTurns: 6, verifiedTurns: 153, greenTurns: 138, edits: 581, failedEdits: 24 }),
    ],
    epoch: parseEpochScores(SCORES),
    boards: benchmarkBoards(parseEpochBenchmarks(RESULTS), new Date(2026, 8, 26)),
    arena: parseArena(ARENA),
    arena3d: [],
  };

  it("offers the user's models first and finds others by name", () => {
    const candidates = compareCandidates(input);
    expect(candidates.slice(0, 2).map((candidate) => candidate.name)).toEqual(["Claude Opus 5", "GPT-6 Astra"]);
    expect(candidates[0]!.sources).toEqual(["mine", "epoch"]);
    expect(searchCandidates(candidates, "kimi", [], 5).map((candidate) => candidate.name)).toEqual(["Kimi K3"]);
    expect(searchCandidates(candidates, "opus", ["claude-opus-5"], 5).map((candidate) => candidate.key).sort()).toEqual(["claude-opus-5-5-max", "claude-opus-5-high"]);
  });

  it("lines up every source, marks only clear wins and labels Arena efforts", () => {
    const rows = compareRows(["claude-opus-5", "gpt-6-astra"], input);
    const score = rows.find((row) => row.kind === "mineScore")!;
    expect(score.cells.every(Boolean)).toBe(true);
    expect(score.decided).toBe(false);
    const edits = rows.find((row) => row.kind === "minePart" && row.part === "edits")!;
    expect(edits.best).toEqual([0]);
    expect(edits.decided).toBe(true);
    const eci = rows.find((row) => row.kind === "eci")!;
    expect(eci.best).toEqual([1]);
    expect(eci.decided).toBe(false);
    const gpqa = rows.find((row) => row.kind === "benchmark" && row.benchmark === "GPQA diamond")!;
    expect(gpqa.cells[0]!.value).toBeCloseTo(91, 9);
    expect(gpqa.cells[1]!.value).toBeCloseTo(94.4, 9);
    expect(rows.some((row) => row.kind === "benchmark" && row.benchmark === "Some New Bench")).toBe(false);
    const code = rows.find((row) => row.kind === "arena" && row.board === "code")!;
    expect(code.cells.map((cell) => [cell?.value, cell?.effort, cell?.sourceName])).toEqual([
      [1662, "high", "claude-opus-5-high"],
      [1792, "max", "gpt-6-astra-max"],
    ]);
    expect(code.decided).toBe(true);
    const agent = rows.find((row) => row.kind === "arenaAgent")!;
    expect(agent.best).toEqual([1]);
    expect(agent.lowerIsBetter).toBe(true);
    const turns = rows.find((row) => row.kind === "mineTurns")!;
    expect(turns.best).toEqual([]);
  });
});

const RESETS = JSON.stringify({
  data: [
    { id: "3", reset_type: "banked", announced_at: "2026-09-22T18:23:37.000Z", text: "banked", source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/3" } },
    { id: "1", reset_type: "regular", announced_at: "2026-09-08T01:56:57.501Z", text: "observed", source: { type: "observed", url: "https://evil.example/phish" } },
    { id: "2", reset_type: "regular", announced_at: "2026-09-12T08:09:17.000Z", text: "regular", source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/2" } },
    { id: "bad", reset_type: "weird", announced_at: "2026-09-01T00:00:00Z", text: "", source: { type: "x_post", url: "" } },
  ],
  pagination: { has_more: false, next_cursor: null },
  meta: { api_version: "v1", generated_at: "2026-09-26T10:02:26.608Z" },
});

describe("Codex resets", () => {
  it("reads the list newest first, drops invalid rows and only links to X over https", () => {
    const resets = parseResets(RESETS);
    expect(resets.map((item) => [item.id, item.kind, item.source.url])).toEqual([
      ["3", "banked", "https://x.com/thsottiaux/status/3"],
      ["2", "regular", "https://x.com/thsottiaux/status/2"],
      ["1", "regular", null],
    ]);
    expect(xUrl("http://x.com/a")).toBeNull();
    expect(xUrl("https://user:pw@x.com/a")).toBeNull();
    expect(xUrl("https://twitter.com/a")).toBe("https://twitter.com/a");
  });

  it("reads the status with an announced reset and a watch that expires", () => {
    const status = parseResetStatus(
      JSON.stringify({
        data: {
          latest_reset: null,
          scheduled_reset: { id: "9", status: "scheduled", reset_type: "regular", announced_at: "2026-09-26T00:07:13.000Z", scheduled_for: null, text: "we'll reset", source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/9" } },
          active_watch: { level: "strong", reset_chance_percent: 70, forecast_window: "next 24 hours", observed_at: "2026-09-26T01:00:00Z", expires_at: "2026-09-27T01:00:00Z", text: "hint", source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/10" } },
          stats: { total: 54, last_reset_at: "2026-09-22T18:23:37.000Z", days_since_last: 3.7, avg_interval_days: 7 },
        },
        meta: { api_version: "v1", generated_at: "2026-09-26T10:02:14.713Z" },
      }),
    )!;
    expect(status.scheduled).toMatchObject({ id: "9", kind: "regular", scheduledFor: null });
    expect(status.total).toBe(54);
    expect(activeWatch(status, new Date("2026-09-26T12:00:00Z"))).toMatchObject({ level: "strong", chancePercent: 70 });
    expect(activeWatch(status, new Date("2026-09-27T02:00:00Z"))).toBeNull();
    expect(parseResetStatus("{}")).toBeNull();
  });

  function at(days: number, now: Date): CodexReset {
    return { id: String(days), kind: "regular", announcedAt: new Date(now.getTime() - days * 86_400_000), text: "", source: { kind: "x_post", url: null } };
  }

  it("weights recent resets by a 21-day half-life", () => {
    const now = new Date("2026-09-26T00:00:00Z");
    const forecast = forecastResets([at(1, now), at(8, now), at(15, now)], now)!;
    expect(forecast.ratePerDay).toBeCloseTo(0.198212, 5);
    expect(forecast.chance[1]).toBeCloseTo(0.179804, 5);
    expect(forecast.chance[3]).toBeCloseTo(0.448237, 5);
    expect(forecast.chance[7]).toBeCloseTo(0.750297, 5);
    expect(forecastResets([at(1, now)], now)).toBeNull();
    const quiet = forecastResets([at(40, now), at(47, now), at(54, now)], now)!;
    expect(quiet.ratePerDay).toBeLessThan(forecast.ratePerDay);
  });

  it("summarizes the history", () => {
    const now = new Date("2026-09-26T00:00:00Z");
    const stats = resetStats([at(1, now), at(8, now), at(40, now)], now);
    expect(stats).toMatchObject({ total: 3, regular: 3, banked: 0, last30Days: 2, last90Days: 3 });
    expect(stats.daysSinceLast).toBeCloseTo(1, 6);
    expect(stats.averageGapDays).toBeCloseTo(19.5, 6);
    expect(stats.medianGapDays).toBeCloseTo(19.5, 6);
    expect(stats.longestGap!.days).toBeCloseTo(32, 6);
  });

  it("compares the current wait with past gaps and marks where a median gap would end", () => {
    const now = new Date("2026-09-26T00:00:00Z");
    const wait = currentWait([at(4, now), at(6, now), at(9, now), at(17, now), at(18, now)], now)!;
    expect(wait.waitedDays).toBeCloseTo(4, 6);
    expect(wait.gaps).toBe(4);
    expect(wait.medianGapDays).toBeCloseTo(2.5, 6);
    expect(wait.shorterShare).toBeCloseTo(0.75, 6);
    expect(wait.medianMark.toISOString()).toBe("2026-09-24T12:00:00.000Z");
    expect(currentWait([at(1, now), at(2, now)], now)).toBeNull();
    expect(currentWait([at(-1, now), at(1, now), at(2, now)], now)).toBeNull();
  });

  it("counts announcements by local weekday and four-hour block", () => {
    const local = (year: number, month: number, day: number, hour: number): CodexReset => ({
      id: `${month}-${day}-${hour}`,
      kind: "regular",
      announcedAt: new Date(year, month - 1, day, hour, 30),
      text: "",
      source: { kind: "x_post", url: null },
    });
    const pattern = announcementPattern([local(2026, 9, 21, 1), local(2026, 9, 22, 5), local(2026, 9, 22, 23), local(2026, 9, 27, 12)]);
    expect(pattern.weekdays).toEqual([1, 2, 0, 0, 0, 0, 1]);
    expect(pattern.hours).toEqual([1, 1, 0, 1, 0, 1]);
    expect(pattern.total).toBe(4);
  });

  it("lays the last weeks out Monday to Sunday with today and the future marked", () => {
    const now = new Date(2026, 8, 26, 15);
    const banked: CodexReset = { id: "b", kind: "banked", announcedAt: new Date(2026, 8, 22, 1), text: "", source: { kind: "x_post", url: null } };
    const twice: CodexReset[] = [
      { id: "r1", kind: "regular", announcedAt: new Date(2026, 8, 26, 8), text: "", source: { kind: "x_post", url: null } },
      { id: "r2", kind: "banked", announcedAt: new Date(2026, 8, 26, 20), text: "", source: { kind: "x_post", url: null } },
    ];
    const weeks = resetCalendar([banked, ...twice], now, 3);
    expect(weeks).toHaveLength(3);
    expect(weeks[0]![0]!.date.getDay()).toBe(1);
    expect(weeks[0]![0]!.date.getDate()).toBe(7);
    const last = weeks[2]!;
    expect(last.map((day) => day.date.getDate())).toEqual([21, 22, 23, 24, 25, 26, 27]);
    expect(last[1]!.kinds).toEqual(["banked"]);
    expect(last[5]).toMatchObject({ kinds: ["regular", "banked"], isToday: true, future: false });
    expect(last[6]).toMatchObject({ kinds: [], isToday: false, future: true });
  });
});
