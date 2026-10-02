import { act, cleanup, renderHook } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { fixtureCatalog, fixtureEngineState } from "@/lib/fixtures";
import { reconcileLayout } from "@/model/layout";
import { useApp } from "@/state/store";
import { useTaskbarStrip } from "./useTaskbarStrip";

vi.mock("./render", () => ({
  MENU_BAR_GLYPH_SIDE: 18,
  GLYPH_SIDE: 16,
  renderBarsGlyph: vi.fn(async () => new Uint8Array([1])),
  renderTextStrip: vi.fn(async () => ({ png: new Uint8Array([1]), width: 100, height: 42 })),
  stripText: () => "Claude 50%",
}));

vi.mock("./support", () => ({
  watchTaskbarInfo: () => () => {},
  useTaskbarInfo: () => ({ supported: true, height: 42, scale: 1, theme: "dark", edge: "bottom" }),
  pushStripFrame: (frame: unknown) => frames(frame),
}));

const frames = vi.fn<(frame: unknown) => Promise<void>>();
const fresh = useApp.getState();

beforeEach(() => {
  vi.useFakeTimers();
  frames.mockReset().mockResolvedValue(undefined);
  vi.spyOn(console, "error").mockImplementation(() => {});
  setBackend(new MockBackend());
  const catalog = fixtureCatalog();
  useApp.setState({
    ...fresh,
    ready: true,
    catalog,
    engine: fixtureEngineState(),
    layout: reconcileLayout(null, catalog),
    settings: { ...fresh.settings, showTaskbarStrip: true, iconStyle: "text" },
  }, true);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  useApp.setState(fresh, true);
});

it("retries a failed frame with unchanged readings and stops after recovery", async () => {
  frames.mockRejectedValueOnce(new Error("tray unavailable"));
  renderHook(useTaskbarStrip);
  await act(async () => {});
  expect(frames).toHaveBeenCalledTimes(1);
  expect(frames.mock.calls[0]?.[0]).toEqual(expect.objectContaining({ width: 100 }));
  await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
  expect(frames).toHaveBeenCalledTimes(2);
  await act(async () => { await vi.advanceTimersByTimeAsync(20_000); });
  expect(frames).toHaveBeenCalledTimes(2);
});

it("keeps retrying repeated native failures without requiring a quota change", async () => {
  frames.mockRejectedValueOnce(new Error("offline")).mockRejectedValueOnce(new Error("offline"));
  renderHook(useTaskbarStrip);
  await act(async () => {});
  await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
  await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
  expect(frames).toHaveBeenCalledTimes(3);
});

it("cancels recovery when the hook unmounts", async () => {
  frames.mockRejectedValueOnce(new Error("offline"));
  const hook = renderHook(useTaskbarStrip);
  await act(async () => {});
  hook.unmount();
  await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
  expect(frames).toHaveBeenCalledTimes(1);
});
