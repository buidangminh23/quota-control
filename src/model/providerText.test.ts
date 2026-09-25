import { chartDayLabel } from "./chart";
import { foldedModels, OTHER_MODEL_NAME, wholePercents } from "./modelUsage";
import { headerNotice, providerTitle, spendLegendName, stalenessHint } from "./providerText";

const account = (label: string) => ({ id: "claude@1", displayName: `Claude · ${label}`, icon: "claude" });

describe("provider titles", () => {
  it("drops the default CLI label and keeps a custom one", () => {
    expect(providerTitle(account("claude"), "vi")).toBe("Claude");
    expect(providerTitle(account("Công ty"), "vi")).toBe("Claude · Công ty");
  });

  it("names local-history cards after this computer in both languages", () => {
    const local = { id: "codex-local", displayName: "Codex Local Usage", icon: "codex" };
    expect(providerTitle(local, "vi")).toBe("Codex · Trên máy này");
    expect(providerTitle(local, "en")).toBe("Codex · This Computer");
    expect(spendLegendName(local, "vi")).toBe("Codex");
  });
});

describe("header notice and staleness", () => {
  it("localizes the error category and keeps the detail", () => {
    const runtime = {
      refreshing: false,
      snapshot: {
        providerID: "claude@1",
        displayName: "Claude · x",
        refreshedAt: new Date().toISOString(),
        errorCategory: "network" as const,
        lines: [{ type: "badge" as const, label: "Error", text: "The request timed out." }],
      },
    };
    expect(headerNotice(runtime, "vi")).toBe("Lỗi mạng\nYêu cầu quá thời gian chờ.");
    expect(headerNotice({ refreshing: false, error: "Refresh failed" }, "vi")).toBe("Làm mới thất bại");
  });

  it("flags data older than two refresh intervals", () => {
    const now = new Date(Date.UTC(2026, 8, 26, 12));
    const snapshot = (minutesAgo: number) => ({
      refreshing: false,
      snapshot: { providerID: "x", displayName: "x", lines: [], refreshedAt: new Date(now.getTime() - minutesAgo * 60_000).toISOString() },
    });
    expect(stalenessHint(snapshot(4), 300_000, now, "vi")).toBeNull();
    expect(stalenessHint(snapshot(12), 300_000, now, "vi")).toEqual({ label: "Dữ liệu cũ", tooltip: "Cập nhật lần cuối 12 phút trước" });
  });
});

describe("model breakdown", () => {
  it("ranks models and folds the small tail into Other with summed tokens", () => {
    const usage = (input: number) => ({ inputTokens: input, outputTokens: 1, cachedInputTokens: 0, cacheCreationInputTokens: 0 });
    const models = foldedModels({
      totalTokens: 1_000,
      sourceNote: "",
      models: [
        { model: "a", totalTokens: 10, costUSD: 0.1, tokenUsage: usage(9) },
        { model: "b", totalTokens: 900, costUSD: 9, tokenUsage: usage(890) },
        { model: "c", totalTokens: 20, costUSD: 0.2, tokenUsage: usage(19) },
        { model: "d", totalTokens: 70, costUSD: 0.7, tokenUsage: usage(69) },
      ],
    });
    expect(models.map((model) => model.model)).toEqual(["b", "d", OTHER_MODEL_NAME]);
    expect(models[2]!.totalTokens).toBe(30);
    expect(models[2]!.tokenUsage).toEqual({ inputTokens: 28, outputTokens: 2, cachedInputTokens: 0, cacheCreationInputTokens: 0 });
  });

  it("rounds shares to whole percents that total exactly 100", () => {
    const percents = wholePercents([1 / 3, 1 / 3, 1 / 3]);
    expect(percents.reduce((sum, value) => sum + value, 0)).toBe(100);
    expect(wholePercents([0, 0])).toEqual([0, 0]);
  });
});

describe("chart day labels", () => {
  it("keeps English labels and writes day/month in Vietnamese", () => {
    expect(chartDayLabel("Sep 06", "en")).toBe("Sep 06");
    expect(chartDayLabel("Sep 06", "vi")).toBe("6/9");
    expect(chartDayLabel("weird", "vi")).toBe("weird");
  });
});
