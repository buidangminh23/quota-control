import { act } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { useApp } from "@/state/store";
import { parseGlanceAction, performGlanceAction, startGlanceActions } from "./glanceActions";

const FRESH = useApp.getState();

async function ready(): Promise<MockBackend> {
  const api = new MockBackend();
  setBackend(api);
  const engine = await api.engineState();
  act(() => useApp.setState({ engine }));
  return api;
}

function codexAccount(): string {
  const id = Object.keys(useApp.getState().engine!.providers).find((providerId) => providerId.startsWith("codex@"));
  if (!id) throw new Error("the mock engine has no Codex account");
  return id;
}

beforeEach(() => useApp.setState(FRESH, true));

describe("parseGlanceAction", () => {
  it("accepts only the three requests the surfaces send", () => {
    expect(parseGlanceAction({ kind: "redeemLimitReset", providerId: "codex@5aef39" })).toEqual({ kind: "redeemLimitReset", providerId: "codex@5aef39" });
    expect(parseGlanceAction({ kind: "markBankedReset", resetId: "2102438800836489554", used: true })).toEqual({ kind: "markBankedReset", resetId: "2102438800836489554", used: true });
    expect(parseGlanceAction({ kind: "openResets", provider: "claude" })).toEqual({ kind: "openResets", provider: "claude" });
    for (const value of [
      null,
      "redeemLimitReset",
      { kind: "redeemLimitReset" },
      { kind: "redeemLimitReset", providerId: "codex" },
      { kind: "redeemLimitReset", providerId: "codex@../../etc" },
      { kind: "markBankedReset", resetId: "x y", used: true },
      { kind: "markBankedReset", resetId: "1", used: "yes" },
      { kind: "openResets", provider: "gemini" },
      { kind: "deleteEverything" },
    ]) {
      expect(parseGlanceAction(value)).toBeNull();
    }
  });
});

describe("performGlanceAction", () => {
  it("spends one reset of a connected Codex account and says so on the island", async () => {
    const api = await ready();
    const redeem = vi.spyOn(api, "redeemLimitReset");
    await act(() => performGlanceAction({ kind: "redeemLimitReset", providerId: codexAccount() }));
    expect(redeem).toHaveBeenCalledOnce();
    expect(redeem).toHaveBeenCalledWith(codexAccount());
    expect(useApp.getState().notice?.text).toBeTruthy();
  });

  it("never spends a reset for an account that is not a connected Codex account", async () => {
    const api = await ready();
    const redeem = vi.spyOn(api, "redeemLimitReset");
    const claude = Object.keys(useApp.getState().engine!.providers).find((id) => id.startsWith("claude@"))!;
    await performGlanceAction({ kind: "redeemLimitReset", providerId: claude });
    await performGlanceAction({ kind: "redeemLimitReset", providerId: "codex@gone" });
    expect(redeem).not.toHaveBeenCalled();
  });

  it("marks a Claude banked reset used and takes the mark back", async () => {
    await ready();
    await performGlanceAction({ kind: "markBankedReset", resetId: "2102438800836489554", used: true });
    expect(useApp.getState().settings.usedBankedResets).toEqual(["2102438800836489554"]);
    await performGlanceAction({ kind: "markBankedReset", resetId: "2102438800836489554", used: false });
    expect(useApp.getState().settings.usedBankedResets).toEqual([]);
  });

  it("opens the Reset tab on the tracker asked for", async () => {
    await ready();
    await act(() => performGlanceAction({ kind: "openResets", provider: "claude" }));
    expect(useApp.getState().settings.resetsProvider).toBe("claude");
    expect(useApp.getState().settings.dashboardTab).toBe("resets");
  });
});

describe("startGlanceActions", () => {
  it("does what the core relays and ignores anything malformed", async () => {
    const api = await ready();
    const stop = startGlanceActions();
    act(() => api.glanceAction({ kind: "openResets", provider: "claude" }));
    await act(() => Promise.resolve());
    expect(useApp.getState().settings.resetsProvider).toBe("claude");
    act(() => api.glanceAction({ kind: "openResets", provider: "nope" }));
    stop();
    act(() => api.glanceAction({ kind: "openResets", provider: "codex" }));
    await act(() => Promise.resolve());
    expect(useApp.getState().settings.resetsProvider).toBe("claude");
  });
});
