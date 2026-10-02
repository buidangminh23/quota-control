import { act, renderHook } from "@testing-library/react";
import { setBackend } from "./backend";
import { MockBackend } from "./mockBackend";
import { usePopupHeight } from "./usePopupHeight";

let observers: Set<() => void>;

beforeEach(() => {
  vi.useFakeTimers();
  observers = new Set();
  vi.stubGlobal("ResizeObserver", class {
    constructor(private callback: () => void) { observers.add(callback); }
    observe() {}
    unobserve() {}
    disconnect() { observers.delete(this.callback); }
  });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => setTimeout(() => callback(performance.now()), 16));
  vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  vi.stubGlobal("innerWidth", 320);
  vi.stubGlobal("innerHeight", 720);
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function advance(milliseconds = 16) {
  await act(async () => { await vi.advanceTimersByTimeAsync(milliseconds); });
}

function setup() {
  const api = new MockBackend();
  setBackend(api);
  const resize = vi.spyOn(api, "resizePopup").mockResolvedValue(undefined);
  const heights = { top: 40, content: 160, footer: 40 };
  const ref = (part: keyof typeof heights) => {
    const element = document.createElement("div");
    Object.defineProperty(element, "offsetHeight", { get: () => heights[part] });
    return { current: element };
  };
  const top = ref("top");
  const content = ref("content");
  const footer = ref("footer");
  const hook = renderHook(({ view, visible }) => usePopupHeight(top, content, footer, view, visible), {
    initialProps: { view: "dashboard", visible: true },
  });
  const contentResized = () => act(() => { for (const callback of observers) callback(); });
  return { ...hook, resize, heights, contentResized };
}

describe("popup height recovery", () => {
  it("deduplicates a successful measurement", async () => {
    const { resize, contentResized } = setup();
    await advance();
    expect(resize).toHaveBeenCalledExactlyOnceWith(240);
    contentResized();
    await advance();
    expect(resize).toHaveBeenCalledTimes(1);
  });

  it("retries a rejected resize even when the content height never changes", async () => {
    const { resize } = setup();
    resize.mockRejectedValueOnce(new Error("Window temporarily unavailable"));
    await advance();
    expect(resize).toHaveBeenCalledTimes(1);
    await advance(266);
    expect(resize).toHaveBeenCalledTimes(2);
    await advance(10_000);
    expect(resize).toHaveBeenCalledTimes(2);
  });

  it("bounds automatic retries and allows reopening to recover after repeated errors", async () => {
    const { resize, rerender } = setup();
    resize.mockRejectedValue(new Error("Window unavailable"));
    await advance();
    await advance(266);
    await advance(1_016);
    await advance(3_016);
    expect(resize).toHaveBeenCalledTimes(4);
    await advance(10_000);
    expect(resize).toHaveBeenCalledTimes(4);
    rerender({ view: "dashboard", visible: false });
    rerender({ view: "dashboard", visible: true });
    resize.mockResolvedValue(undefined);
    await advance();
    expect(resize).toHaveBeenCalledTimes(5);
  });

  it("re-clamps unchanged content after reopening and a viewport change", async () => {
    const { resize, rerender } = setup();
    await advance();
    rerender({ view: "dashboard", visible: false });
    await advance();
    expect(resize).toHaveBeenCalledTimes(1);
    rerender({ view: "dashboard", visible: true });
    await advance();
    expect(resize).toHaveBeenCalledTimes(2);
    vi.stubGlobal("innerHeight", 480);
    act(() => { window.dispatchEvent(new Event("resize")); });
    await advance();
    expect(resize).toHaveBeenCalledTimes(3);
    expect(resize).toHaveBeenLastCalledWith(240);
  });

  it("settles its own native viewport change without a resize loop", async () => {
    const { resize } = setup();
    resize.mockImplementation(async () => {
      vi.stubGlobal("innerHeight", 240);
      window.dispatchEvent(new Event("resize"));
    });
    await advance();
    await advance();
    act(() => { window.dispatchEvent(new Event("resize")); });
    await advance();
    expect(resize).toHaveBeenCalledTimes(2);
  });

  it("re-clamps a viewport change received while an earlier resize is still pending", async () => {
    const { resize } = setup();
    let resolve!: () => void;
    resize.mockImplementationOnce(() => new Promise<void>((done) => { resolve = done; }));
    await advance();
    vi.stubGlobal("innerHeight", 480);
    act(() => { window.dispatchEvent(new Event("resize")); });
    await advance();
    expect(resize).toHaveBeenCalledTimes(1);
    await act(async () => { resolve(); });
    await advance();
    expect(resize.mock.calls).toEqual([[240], [240]]);
    await advance(10_000);
    expect(resize).toHaveBeenCalledTimes(2);
  });

  it("serializes requests and coalesces measurements to the latest content while one is pending", async () => {
    const { resize, heights, contentResized, rerender } = setup();
    let resolve!: () => void;
    resize.mockImplementationOnce(() => new Promise<void>((done) => { resolve = done; }));
    await advance();
    heights.content = 260;
    contentResized();
    await advance();
    heights.content = 360;
    rerender({ view: "settings", visible: true });
    await advance();
    expect(resize).toHaveBeenCalledTimes(1);
    await act(async () => { resolve(); });
    await advance();
    expect(resize.mock.calls).toEqual([[240], [440]]);
    contentResized();
    await advance();
    expect(resize).toHaveBeenCalledTimes(2);
  });

  it("cancels scheduled retries and ignores in-flight rejection after unmount", async () => {
    const first = setup();
    first.resize.mockRejectedValueOnce(new Error("Unavailable"));
    await advance();
    first.unmount();
    await advance(10_000);
    expect(first.resize).toHaveBeenCalledTimes(1);
    const second = setup();
    let reject!: (error: Error) => void;
    second.resize.mockImplementationOnce(() => new Promise<void>((_, fail) => { reject = fail; }));
    await advance();
    second.unmount();
    await act(async () => { reject(new Error("Closed")); });
    await advance(10_000);
    expect(second.resize).toHaveBeenCalledTimes(1);
    expect(observers.size).toBe(0);
  });
});
