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
    await backend.saveDocument("settings", { fromWeb: true });
    expect(await backend.loadDocument("settings")).toEqual({ fromWeb: true });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("refuses actions that would change accounts or spend a reset", async () => {
    const fetchMock = respondWith({});
    const backend = new HttpBackend(base);
    await expect(backend.redeemLimitReset("codex@a")).rejects.toThrow("Bản web chỉ để xem");
    await expect(backend.removeAccount("codex@a")).rejects.toThrow("Bản web chỉ để xem");
    await expect(backend.addApiKey("cursor", "key")).rejects.toThrow("Bản web chỉ để xem");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
