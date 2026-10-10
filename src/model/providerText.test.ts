import { chartDayLabel } from "./chart";
import { foldedModels, OTHER_MODEL_NAME, wholePercents } from "./modelUsage";
import { cardIdentity, hasPlanReading, headerNotice, isOutdated, providerTitle, spendLegendName, stalenessHint } from "./providerText";

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
    expect(stalenessHint(snapshot(12), 300_000, now, "vi")).toEqual({ tooltip: "Cập nhật lần cuối 12 phút trước" });
    for (const minutes of [4, 9, 10, 12]) {
      expect(isOutdated(snapshot(minutes).snapshot.refreshedAt, 300_000, now), `${minutes}`).toBe(stalenessHint(snapshot(minutes), 300_000, now, "vi") !== null);
    }
    expect(isOutdated(undefined, 300_000, now)).toBe(false);
    const runtime = snapshot(12);
    expect(hasPlanReading(runtime)).toBe(true);
    expect(hasPlanReading({ ...runtime, snapshot: { ...runtime.snapshot, usageUnavailable: true } })).toBe(false);
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
    const term = { basis: "stated", endsAt: "2026-10-30T03:00:00Z", checkedAt: "2026-09-26T03:00:00Z" } as const;
    const codex = { id: "codex@1", displayName: "Codex · codex", icon: "codex" };
    expect(cardIdentity(codex, snapshot({ planTerm: term }), "vi").planTerm).toEqual(term);
    expect(cardIdentity(codex, undefined, "vi")).toMatchObject({ planTerm: null, notice: null });
    const failed = { ...snapshot({ errorCategory: "network", lines: [{ type: "badge", label: "Error", text: "The request timed out." }] }) } as Parameters<typeof cardIdentity>[1];
    expect(cardIdentity(codex, failed, "vi").notice).toBe("Lỗi mạng");
  });

  it("removes the previous paid term on Free and uses renewed paid metadata without inferring a plan from dates", () => {
    const provider = { id: "codex@1", displayName: "Codex · personal", icon: "codex" };
    const term = { basis: "stated", endsAt: "2026-09-01T00:00:00Z", checkedAt: "2026-08-25T00:00:00Z" } as const;
    expect(cardIdentity(provider, snapshot({ plan: "Free", planTerm: term }), "vi")).toMatchObject({ plan: "Free", planTerm: null });
    expect(cardIdentity(provider, snapshot({ plan: " free ", planTerm: term }), "en").planTerm).toBeNull();
    expect(cardIdentity(provider, snapshot({ plan: "Plus", planTerm: term }), "vi")).toMatchObject({ plan: "Plus", planTerm: term });
    const renewed = { ...term, endsAt: "2026-11-01T00:00:00Z", checkedAt: "2026-10-01T00:00:00Z" };
    expect(cardIdentity(provider, snapshot({ plan: "Plus", planTerm: renewed }), "vi").planTerm).toEqual(renewed);
    expect(cardIdentity(provider, snapshot({ plan: "Plus", planTerm: renewed, errorCategory: "network" }), "vi").planTerm).toBeNull();
  });

  it("rejects legacy inferred dates for every provider even when a cached plan was confirmed", () => {
    const term = { basis: "monthlyFrom", startedAt: "2026-08-30T03:00:00Z", checkedAt: "2026-09-25T00:00:00Z" } as const;
    const { checkedAt: _checkedAt, ...legacyTerm } = term;
    for (const brand of ["codex", "cursor", "windsurf"]) {
      const provider = { id: `${brand}@1`, displayName: brand, icon: brand };
      expect(cardIdentity(provider, snapshot({ plan: "Pro", planTerm: term }), "vi").planTerm).toBeNull();
      expect(cardIdentity(provider, snapshot({ plan: "Pro", planTerm: legacyTerm, planCheckedAt: term.checkedAt }), "vi").planTerm).toBeNull();
    }
  });

  it("rejects historical Claude start dates even when the paid profile is freshly confirmed", () => {
    const term = { basis: "monthlyFrom", startedAt: "2026-07-31T03:40:09Z", checkedAt: "2026-10-10T01:00:00Z" } as const;
    for (const errorCategory of [undefined, "rate_limited"] as const) {
      const runtime = snapshot({ plan: "Max 20x", planTerm: term, planCheckedAt: term.checkedAt, errorCategory });
      expect(cardIdentity(account("personal"), runtime, "vi").planTerm).toBeNull();
      expect(cardIdentity(account("personal"), { ...runtime, error: "Usage updates are rate limited. Try again later." }, "vi").planTerm).toBeNull();
    }
  });

  it("uses fresh plan metadata even if the quota request failed", () => {
    const term = { basis: "stated", endsAt: "2026-11-03T07:03:48Z", checkedAt: "2026-10-10T01:00:00Z" } as const;
    const failedUsage = snapshot({ plan: "Max 20x", planTerm: term, planCheckedAt: term.checkedAt, errorCategory: "rate_limited" });
    expect(cardIdentity(account("personal"), failedUsage, "vi")).toMatchObject({ plan: "Max 20x", planTerm: term, notice: "Bị giới hạn tần suất, sẽ thử lại" });
    expect(cardIdentity(account("personal"), { ...failedUsage, error: "Usage updates are rate limited. Try again later." }, "vi").planTerm).toEqual(term);
    expect(cardIdentity(account("personal"), { ...failedUsage, snapshot: { ...failedUsage.snapshot, plan: "Free" } }, "vi")).toMatchObject({ plan: "Free", planTerm: null });
  });

  it("requires a confirmed paid Claude plan for a stated billing date", () => {
    const term = { basis: "stated", endsAt: "2026-11-03T07:03:48Z", checkedAt: "2026-10-10T01:00:00Z" } as const;
    for (const plan of [undefined, "Free", "Unknown", "Plus"]) {
      expect(cardIdentity(account("personal"), snapshot({ plan, planTerm: term, planCheckedAt: term.checkedAt }), "vi").planTerm).toBeNull();
    }
    expect(cardIdentity(account("personal"), snapshot({ plan: "Max 20x", planTerm: term }), "vi").planTerm).toBeNull();
    for (const plan of ["Pro", "Max", "Max 5x", "Max 20x", "Team"]) {
      expect(cardIdentity(account("personal"), snapshot({ plan, planTerm: term, planCheckedAt: term.checkedAt }), "vi").planTerm).toEqual(term);
    }
  });

  it("hides Claude billing dates after authentication fails even with checked plan metadata", () => {
    const term = { basis: "stated", endsAt: "2026-11-03T07:03:48Z", checkedAt: "2026-10-10T01:00:00Z" } as const;
    const confirmed = { plan: "Max 20x", planTerm: term, planCheckedAt: term.checkedAt };
    for (const errorCategory of ["auth_expired", "auth_invalid", "not_logged_in"]) {
      expect(cardIdentity(account("personal"), snapshot({ ...confirmed, errorCategory }), "vi").planTerm).toBeNull();
    }
    for (const error of ["Session expired. Sign in again.", "Local credentials are invalid. Sign in again.", "The login does not have permission to read usage. Sign in again."]) {
      expect(cardIdentity(account("personal"), { ...snapshot(confirmed), error }, "vi").planTerm).toBeNull();
    }
  });

  it("keeps verified service dates through network errors and hides them after logout", () => {
    const term = { basis: "stated", endsAt: "2026-11-03T07:03:48Z", checkedAt: "2026-10-10T01:00:00Z" } as const;
    for (const brand of ["cursor", "augment", "zai", "warp", "codex"]) {
      const provider = { id: `${brand}@1`, displayName: brand, icon: brand };
      const confirmed = { plan: "Pro", planTerm: term, planCheckedAt: term.checkedAt };
      expect(cardIdentity(provider, { ...snapshot(confirmed), error: "The request timed out." }, "vi").planTerm).toEqual(term);
      for (const errorCategory of ["auth_expired", "auth_invalid", "not_logged_in"]) {
        expect(cardIdentity(provider, snapshot({ ...confirmed, errorCategory }), "vi").planTerm).toBeNull();
      }
      expect(cardIdentity(provider, snapshot({ ...confirmed, plan: "Free" }), "vi").planTerm).toBeNull();
    }
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
