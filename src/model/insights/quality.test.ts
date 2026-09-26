import type { QualityCounts, QualityRow } from "@/lib/insightsTypes";
import { displayModel, effortVariant, modelKey } from "./modelNames";
import { EMPTY_COUNTS, modelQuality, perTurn, qualityProjects, qualityScore, rangeQuery, successRates, summarizeQuality } from "./quality";
import { overlaps, wilson } from "./stats";

function counts(patch: Partial<QualityCounts>): QualityCounts {
  return { ...EMPTY_COUNTS, ...patch };
}

const OPUS: QualityCounts = counts({ turns: 2328, humanTurns: 1226, interruptedTurns: 9, verifiedTurns: 281, greenTurns: 218, edits: 2361, failedEdits: 12 });

describe("wilson", () => {
  it("matches the textbook interval, stays inside 0..1 at the edges and refuses empty samples", () => {
    const interval = wilson(81, 263)!;
    expect(interval.rate).toBeCloseTo(0.30798, 5);
    expect(interval.low).toBeCloseTo(0.25529, 5);
    expect(interval.high).toBeCloseTo(0.36621, 5);
    expect(wilson(0, 10)).toMatchObject({ low: 0 });
    expect(wilson(0, 10)!.high).toBeCloseTo(0.27753, 5);
    expect(wilson(10, 10)!.low).toBeCloseTo(0.72247, 5);
    expect(wilson(10, 10)!.high).toBeLessThanOrEqual(1);
    expect(wilson(0, 0)).toBeNull();
    expect(wilson(5, 3)).toBeNull();
  });

  it("tells whether two intervals share a value", () => {
    expect(overlaps({ low: 0.1, high: 0.4 }, { low: 0.35, high: 0.6 })).toBe(true);
    expect(overlaps({ low: 0.1, high: 0.3 }, { low: 0.35, high: 0.6 })).toBe(false);
  });
});

describe("qualityScore", () => {
  it("averages the three rates and bounds them with intervals that hold together", () => {
    const score = qualityScore(OPUS)!;
    expect(score.value).toBeCloseTo(92.1126, 3);
    expect(score.low).toBeCloseTo(89.5066, 3);
    expect(score.high).toBeCloseTo(94.1183, 3);
  });

  it("has no score while any part lacks its minimum samples", () => {
    expect(qualityScore({ ...OPUS, edits: 19, failedEdits: 0 })).toBeNull();
    expect(qualityScore({ ...OPUS, verifiedTurns: 9, greenTurns: 9 })).toBeNull();
    expect(qualityScore({ ...OPUS, humanTurns: 9, interruptedTurns: 0 })).toBeNull();
    expect(qualityScore({ ...OPUS, edits: 20, failedEdits: 0, verifiedTurns: 10, greenTurns: 10, humanTurns: 10, interruptedTurns: 0 })).not.toBeNull();
  });

  it("reports each part with its samples even when it is too small to count", () => {
    const quality = modelQuality("claude", "claude-sonnet-5", counts({ turns: 99, humanTurns: 38, edits: 131 }));
    expect(quality.name).toBe("Claude Sonnet 5");
    expect(quality.score).toBeNull();
    expect(quality.parts.edits).toMatchObject({ successes: 131, samples: 131, enough: true });
    expect(quality.parts.green).toMatchObject({ samples: 0, interval: null, enough: false });
    expect(quality.parts.steady).toMatchObject({ successes: 38, samples: 38, enough: true });
  });
});

describe("summarizeQuality", () => {
  const rows: QualityRow[] = [
    { source: "claude", model: "claude-opus-5", project: "quota-control", counts: OPUS },
    { source: "claude", model: "claude-opus-5", project: "demo-app", counts: counts({ turns: 10, humanTurns: 10, edits: 5 }) },
    { source: "codex", model: "gpt-5.5", project: "demo-app", counts: counts({ turns: 95, humanTurns: 95, verifiedTurns: 20, greenTurns: 19, edits: 159, failedEdits: 6 }) },
    { source: "codex", model: "gpt-5.6-terra", project: "", counts: counts({ turns: 25, humanTurns: 18, edits: 15 }) },
    { source: "codex", model: "o4-mini", project: "", counts: counts({ turns: 1 }) },
  ];

  it("merges a model's projects, puts scored models first by score and the rest by use", () => {
    const summary = summarizeQuality(rows, null);
    expect(summary.map((model) => model.key)).toEqual(["codex:gpt-5.5", "claude:claude-opus-5", "codex:gpt-5.6-terra", "codex:o4-mini"]);
    expect(summary[1]!.counts.turns).toBe(2338);
    expect(summary[1]!.counts.edits).toBe(2366);
  });

  it("filters to one project, including the one the transcripts did not name", () => {
    expect(summarizeQuality(rows, "demo-app").map((model) => model.model)).toEqual(["gpt-5.5", "claude-opus-5"]);
    expect(summarizeQuality(rows, "").map((model) => model.model)).toEqual(["gpt-5.6-terra", "o4-mini"]);
  });

  it("lists projects by use with the unnamed one last", () => {
    expect(qualityProjects(rows)).toEqual([
      { project: "quota-control", turns: 2328 },
      { project: "demo-app", turns: 105 },
      { project: "", turns: 26 },
    ]);
  });
});

describe("period helpers", () => {
  it("covers whole local days ending today", () => {
    const now = new Date(2026, 8, 26, 17, 30);
    expect(rangeQuery("7", now)).toEqual({ from: "2026-09-20", to: "2026-09-26" });
    expect(rangeQuery("30", now)).toEqual({ from: "2026-08-28", to: "2026-09-26" });
    expect(rangeQuery("all", now)).toEqual({ from: null, to: null });
  });

  it("averages per turn and rates checks and commands only when they ran", () => {
    expect(perTurn(counts({ turns: 4, outputTokens: 100, timedTurns: 2, turnMillis: 65_000 }))).toEqual({ tokens: 25, seconds: 32.5 });
    expect(perTurn(EMPTY_COUNTS)).toEqual({ tokens: null, seconds: null });
    expect(successRates(counts({ checkRuns: 4, failedCheckRuns: 1, shellCommands: 10, failedShellCommands: 2 }))).toEqual({ checks: 0.75, shell: 0.8 });
    expect(successRates(EMPTY_COUNTS)).toEqual({ checks: null, shell: null });
  });
});

describe("model names", () => {
  it("prints transcript ids the way people write them", () => {
    expect(displayModel("claude-opus-5-5")).toBe("Claude Opus 5.5");
    expect(displayModel("claude-fable-5-1")).toBe("Claude Fable 5.1");
    expect(displayModel("claude-opus-4-8")).toBe("Claude Opus 4.8");
    expect(displayModel("gpt-5.6-sol")).toBe("GPT-5.6 Sol");
    expect(displayModel("gpt-6-astra")).toBe("GPT-6 Astra");
    expect(displayModel("gpt-5-codex")).toBe("GPT-5 Codex");
    expect(displayModel("gpt-4o")).toBe("GPT-4o");
    expect(displayModel("claude-3-5-sonnet-20241022")).toBe("Claude 3.5 Sonnet 20241022");
    expect(displayModel("o4-mini")).toBe("o4-mini");
    expect(displayModel("cx/gpt-5.5")).toBe("cx/GPT-5.5");
    expect(displayModel("codex-auto-review")).toBe("Codex Auto Review");
  });

  it("joins sources only on the same model", () => {
    expect(modelKey("GPT-5.6 Sol")).toBe(modelKey(displayModel("gpt-5.6-sol")));
    expect(modelKey("Claude Opus 5")).toBe(modelKey(displayModel("claude-opus-5")));
    expect(modelKey("Claude Opus 5")).not.toBe(modelKey("Claude Opus 5.5"));
    expect(modelKey("GPT 6 Astra (Max)")).toBe("gpt-6-astra-max");
  });

  it("recognises an Arena effort variant and nothing looser", () => {
    expect(effortVariant("claude-opus-5-5", modelKey("claude-opus-5.5-high"))).toBe("high");
    expect(effortVariant("gpt-6-astra", modelKey("GPT 6 Astra (Max)"))).toBe("max");
    expect(effortVariant("claude-opus-5", modelKey("claude-opus-5.5-high"))).toBeNull();
    expect(effortVariant("gpt-5", modelKey("gpt-5.5"))).toBeNull();
    expect(effortVariant("gpt-5-6-sol", modelKey("gpt-5.6-sol-xhigh (codex-harness)"))).toBeNull();
  });
});
