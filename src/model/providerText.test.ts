import { chartDayLabel } from "./chart";
import { foldedModels, OTHER_MODEL_NAME, wholePercents } from "./modelUsage";
import { cardIdentity, headerNotice, isOutdated, providerTitle, spendLegendName, stalenessHint } from "./providerText";

const account = (label: string) => ({ id: "claude@1", displayName: `Claude · ${label}`, icon: "claude" });

describe("provider titles", () => {
  it("drops the default CLI label and keeps a custom one", () => {
    expect(providerTitle(account("claude"), "vi")).toBe("Claude");
    expect(providerTitle(account("Công ty"), "vi")).toBe("Claude · Công ty");
  });

  it("names local-history sections after the brand alone in both languages", () => {
    const local = { id: "codex-local", displayName: "Codex Local Usage", icon: "codex" };
    expect(providerTitle(local, "vi")).toBe("Codex");
    expect(providerTitle(local, "en")).toBe("Codex");
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
    for (const minutes of [4, 9, 10, 12]) {
      expect(isOutdated(snapshot(minutes).snapshot.refreshedAt, 300_000, now), `${minutes}`).toBe(stalenessHint(snapshot(minutes), 300_000, now, "vi") !== null);
    }
    expect(isOutdated(undefined, 300_000, now)).toBe(false);
  });
});

describe("card identity", () => {
  const snapshot = (extra: object = {}) => ({ refreshing: false, snapshot: { providerID: "x", displayName: "x", lines: [], refreshedAt: "2026-09-26T03:00:00Z", ...extra } });

  it("names the account by the email the provider reports when its label is not one, as the card does", () => {
    const copilot = { id: "copilot@1", displayName: "Copilot", icon: "copilot" };
    expect(cardIdentity(copilot, snapshot({ account: "octo@example.com", plan: "Business" }), "vi")).toMatchObject({ name: "Copilot", account: "octo@example.com", plan: "Business" });
    expect(cardIdentity(copilot, snapshot(), "vi").account).toBeNull();
    const labelled = { id: "claude@1", displayName: "Claude · me@example.com", icon: "claude" };
    expect(cardIdentity(labelled, snapshot({ account: "other@example.com" }), "vi")).toMatchObject({ name: "Claude", account: "me@example.com" });
    const local = { id: "claude-local", displayName: "Claude Local Usage", icon: "claude" };
    expect(cardIdentity(local, snapshot({ account: "me@example.com", plan: "Max", planTerm: { basis: "stated", endsAt: "2026-10-01T00:00:00Z" } }), "vi")).toMatchObject({ account: null, plan: null, planTerm: null });
  });

  it("passes on the plan's paid period and the header notice's first line", () => {
    const term = { basis: "monthlyFrom", startedAt: "2026-08-30T03:00:00Z" } as const;
    const codex = { id: "codex@1", displayName: "Codex · codex", icon: "codex" };
    expect(cardIdentity(codex, snapshot({ planTerm: term }), "vi").planTerm).toEqual({ ...term, checkedAt: "2026-09-26T03:00:00Z" });
    expect(cardIdentity(codex, undefined, "vi")).toMatchObject({ planTerm: null, notice: null });
    const failed = { ...snapshot({ errorCategory: "network", lines: [{ type: "badge", label: "Error", text: "The request timed out." }] }) } as Parameters<typeof cardIdentity>[1];
    expect(cardIdentity(codex, failed, "vi").notice).toBe("Lỗi mạng");
  });

  it("removes the previous paid term on Free and uses renewed paid metadata without inferring a plan from dates", () => {
    const provider = account("personal");
    const term = { basis: "stated", endsAt: "2026-09-01T00:00:00Z", checkedAt: "2026-08-25T00:00:00Z" } as const;
    expect(cardIdentity(provider, snapshot({ plan: "Free", planTerm: term }), "vi")).toMatchObject({ plan: "Free", planTerm: null });
    expect(cardIdentity(provider, snapshot({ plan: " free ", planTerm: term }), "en").planTerm).toBeNull();
    expect(cardIdentity(provider, snapshot({ plan: "Plus", planTerm: term }), "vi")).toMatchObject({ plan: "Plus", planTerm: term });
    const renewed = { ...term, endsAt: "2026-11-01T00:00:00Z", checkedAt: "2026-10-01T00:00:00Z" };
    expect(cardIdentity(provider, snapshot({ plan: "Plus", planTerm: renewed }), "vi").planTerm).toEqual(renewed);
    expect(cardIdentity(provider, snapshot({ plan: "Plus", planTerm: renewed, errorCategory: "network" }), "vi").planTerm).toBeNull();
  });

  it("preserves a billing confirmation separately from the usage snapshot timestamp", () => {
    const term = { basis: "monthlyFrom", startedAt: "2026-08-30T03:00:00Z", checkedAt: "2026-09-25T00:00:00Z" } as const;
    expect(cardIdentity(account("personal"), snapshot({ plan: "Pro", planTerm: term }), "vi").planTerm).toEqual(term);
    const { checkedAt: _checkedAt, ...legacyTerm } = term;
    expect(cardIdentity(account("personal"), snapshot({ plan: "Pro", planTerm: legacyTerm, planCheckedAt: term.checkedAt }), "vi").planTerm).toEqual(term);
  });

  it("uses fresh plan metadata even if the quota request failed", () => {
    const term = { basis: "stated", endsAt: "2026-11-01T00:00:00Z", checkedAt: "2026-10-01T00:00:00Z" } as const;
    const failedUsage = snapshot({ plan: "Plus", planTerm: term, planCheckedAt: term.checkedAt, errorCategory: "rate_limited" });
    expect(cardIdentity(account("personal"), failedUsage, "vi")).toMatchObject({ plan: "Plus", planTerm: term, notice: "Bị giới hạn tần suất, sẽ thử lại" });
    expect(cardIdentity(account("personal"), { ...failedUsage, snapshot: { ...failedUsage.snapshot, plan: "Free" } }, "vi")).toMatchObject({ plan: "Free", planTerm: null });
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
