import { afterEach, describe, expect, it, vi } from "vitest";
import { HttpBackend, isWebPopup } from "./httpBackend";

const base = new URL("https://hub.example/quota/v1/app/");

function respondWith(body: unknown, status = 200) {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL) => new Response(JSON.stringify(body), { status }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

afterEach(() => {
  vi.unstubAllGlobals();
  window.localStorage.clear();
});

describe("HttpBackend", () => {
  it("is chosen only for the page the app serves under /web/", () => {
    expect(isWebPopup({ pathname: "/quota/web/" } as Location)).toBe(true);
    expect(isWebPopup({ pathname: "/web/index.html" } as Location)).toBe(true);
    expect(isWebPopup({ pathname: "/web" } as Location)).toBe(true);
    expect(isWebPopup({ pathname: "/" } as Location)).toBe(false);
    expect(isWebPopup({ pathname: "/web/assets/x.js" } as Location)).toBe(false);
  });

  it("asks the app the same command and arguments the popup sends over IPC", async () => {
    const fetchMock = respondWith([]);
    await new HttpBackend(base).usageSummary({ groupBy: "day", from: "2026-11-01", to: "2026-11-01" });
    const url = new URL(String(fetchMock.mock.calls[0]?.[0]));
    expect(url.pathname).toBe("/quota/v1/app/usage_summary");
    expect(JSON.parse(url.searchParams.get("q") ?? "")).toEqual({ query: { groupBy: "day", from: "2026-11-01", to: "2026-11-01" } });
  });

  it("reports a failed request instead of returning stale data", async () => {
    respondWith({ error: "unavailable" }, 500);
    await expect(new HttpBackend(base).engineState()).rejects.toThrow("500");
  });

  it("keeps settings changed on the web in this browser and never writes them back", async () => {
    const fetchMock = respondWith({ fromApp: true });
    const backend = new HttpBackend(base);
    expect(await backend.loadDocument("settings")).toEqual({ fromApp: true });
    const callsBeforeSave = fetchMock.mock.calls.length;
    await backend.saveDocument("settings", { fromWeb: true });
    expect(fetchMock).toHaveBeenCalledTimes(callsBeforeSave);
    expect(await backend.loadDocument("settings")).toEqual({ fromWeb: true });
  });

  it("shows accounts connected after the web saved its settings copy", async () => {
    const backend = new HttpBackend(base);
    respondWith({ enabledProviders: ["codex@old"], density: "regular" });
    await backend.saveDocument("settings", { ...(await backend.loadDocument<Record<string, unknown>>("settings")), density: "compact" });
    respondWith({ enabledProviders: ["codex@old", "claude@new"], density: "regular" });
    expect(await backend.loadDocument("settings")).toEqual({ enabledProviders: ["codex@old", "claude@new"], density: "compact" });
    respondWith({ density: "regular" });
    expect(await backend.loadDocument("settings")).toEqual({ density: "compact" });
  });

  it("falls back to the browser's settings copy when the app cannot be reached", async () => {
    const backend = new HttpBackend(base);
    await backend.saveDocument("settings", { enabledProviders: ["codex@old"] });
    respondWith({ error: "unavailable" }, 502);
    expect(await backend.loadDocument("settings")).toEqual({ enabledProviders: ["codex@old"] });
  });

  it("keeps the browser's layout copy as it was saved", async () => {
    const backend = new HttpBackend(base);
    await backend.saveDocument("layout", { providerOrder: ["a"] });
    const fetchMock = respondWith({ providerOrder: ["b"] });
    expect(await backend.loadDocument("layout")).toEqual({ providerOrder: ["a"] });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("sends actions as POST with the write header and reports the app's own error", async () => {
    const fetchMock = respondWith({ error: "This account is not connected." }, 422);
    await expect(new HttpBackend(base).redeemLimitReset("codex@a")).rejects.toThrow("This account is not connected.");
    const [url, init] = fetchMock.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(url)).toBe("https://hub.example/quota/v1/app/redeem_limit_reset");
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>)["x-quota-control"]).toBe("write");
    expect(JSON.parse(String(init.body))).toEqual({ providerId: "codex@a" });
  });

  it("opens a remote sign-in in the viewer's browser and finishes it with the pasted address", async () => {
    const fetchMock = respondWith({ flowId: "f1", authorizationUrl: "https://auth.example/login", expiresInSeconds: 600, browser: "remote" });
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const backend = new HttpBackend(base);
    await backend.beginAccountLogin("codex", "vi");
    expect(await backend.reopenAccountLogin("f1")).toBe("remote");
    expect(open).toHaveBeenCalledWith("https://auth.example/login", "_blank", "noopener,noreferrer");
    await backend.completeAccountLogin("f1", "http://localhost:1455/auth/callback?code=x");
    const [, init] = fetchMock.mock.calls[1] as unknown as [URL, RequestInit];
    expect(JSON.parse(String(init.body))).toEqual({ flowId: "f1", address: "http://localhost:1455/auth/callback?code=x" });
    await expect(backend.beginCliLogin("codex")).rejects.toThrow("Đăng nhập CLI");
  });

  it("opens the sign-in page in a tab reserved during the click, and closes it when starting fails", async () => {
    const page = { opener: {} as unknown, location: { href: "" }, close: vi.fn() };
    const open = vi.fn(() => page);
    vi.stubGlobal("open", open);
    respondWith({ flowId: "f3", authorizationUrl: "https://auth.example/claude", expiresInSeconds: 600, browser: "remote" });
    const backend = new HttpBackend(base);
    await backend.beginAccountLogin("claude", "vi");
    expect(open).toHaveBeenCalledWith("", "_blank");
    expect(page.opener).toBeNull();
    expect(page.location.href).toBe("https://auth.example/claude");
    respondWith({ error: "No sign-in" }, 422);
    await expect(backend.beginAccountLogin("claude", "vi")).rejects.toThrow("No sign-in");
    expect(page.close).toHaveBeenCalledTimes(1);
  });

  it("picks up a sign-in still waiting after the page reloads and can reopen its page", async () => {
    respondWith([{ flowId: "f2", provider: "claude", authorizationUrl: "https://auth.example/claude", expiresInSeconds: 600, browser: "remote" }]);
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const backend = new HttpBackend(base);
    const pending = await backend.listPendingLogins();
    expect(pending.map((login) => [login.flowId, login.provider])).toEqual([["f2", "claude"]]);
    expect(await backend.reopenAccountLogin("f2")).toBe("remote");
    expect(open).toHaveBeenCalledWith("https://auth.example/claude", "_blank", "noopener,noreferrer");
  });
});
