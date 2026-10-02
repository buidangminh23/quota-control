import { act, renderHook } from "@testing-library/react";
import { useFinalCountdown } from "./hooks";
import { useApp } from "./store";

const BASE = new Date(2026, 9, 2, 16, 4).getTime();
const SECOND = 1000;
const FINAL = 300 * SECOND;

function showPopup(visible: boolean): void {
  act(() => useApp.setState({ popupVisible: visible }));
}

function advance(milliseconds: number): void {
  act(() => {
    vi.advanceTimersByTime(milliseconds);
  });
}

/** A row `leftMs` from its deadline, drawn with the caller's clock still at `BASE`. */
function countdown(leftMs: number | null) {
  const caller = new Date(BASE);
  const deadline = leftMs === null ? null : new Date(BASE + leftMs);
  let draws = 0;
  const hook = renderHook(
    ({ now }) => {
      draws += 1;
      return useFinalCountdown(deadline, now);
    },
    { initialProps: { now: caller } },
  );
  const left = () => (deadline === null ? Number.NaN : deadline.getTime() - hook.result.current.getTime());
  return { ...hook, caller, left, draws: () => draws };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(BASE);
  showPopup(true);
});

afterEach(() => {
  showPopup(true);
  vi.useRealTimers();
});

describe("the clock of a row counting down", () => {
  it("is the caller's clock, with nothing drawn again, until the last five minutes", () => {
    const row = countdown(FINAL + SECOND);
    expect(row.result.current).toBe(row.caller);
    const drawn = row.draws();
    advance(SECOND - 1);
    expect(row.result.current).toBe(row.caller);
    expect(row.draws()).toBe(drawn);
    advance(1);
    expect(row.left()).toBe(FINAL);
  });

  it("crosses into the last five minutes between two of the caller's readings, then steps as each second ends", () => {
    const row = countdown(FINAL + 400);
    expect(row.result.current).toBe(row.caller);
    advance(399);
    expect(row.result.current).toBe(row.caller);
    advance(1);
    expect(row.left()).toBe(FINAL);
    advance(999);
    expect(row.left()).toBe(FINAL);
    advance(1);
    expect(row.left()).toBe(FINAL - SECOND);
    for (let second = 298; second >= 1; second -= 1) {
      advance(SECOND);
      expect(row.left()).toBe(second * SECOND);
    }
  });

  it("stops at the deadline and never reads past it on its own", () => {
    const row = countdown(2 * SECOND + 500);
    expect(row.left()).toBe(2 * SECOND + 500);
    advance(500);
    expect(row.left()).toBe(2 * SECOND);
    advance(2 * SECOND);
    expect(row.left()).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    const drawn = row.draws();
    advance(60 * SECOND);
    expect(row.left()).toBe(0);
    expect(row.draws()).toBe(drawn);
  });

  it("works the time left out from the wall clock at every wake, so a late timer lands on the right second", () => {
    const row = countdown(200 * SECOND);
    expect(row.left()).toBe(200 * SECOND);
    vi.setSystemTime(BASE + 60 * SECOND + 500);
    advance(SECOND);
    expect(row.left()).toBe(138 * SECOND + 500);
    advance(500);
    expect(row.left()).toBe(138 * SECOND);
    advance(SECOND);
    expect(row.left()).toBe(137 * SECOND);
  });

  it("keeps no timer while the popup is hidden and reads the wall clock the moment it reopens", () => {
    const row = countdown(200 * SECOND);
    showPopup(false);
    expect(row.result.current).toBe(row.caller);
    expect(vi.getTimerCount()).toBe(0);
    const drawn = row.draws();
    advance(45 * SECOND);
    expect(row.draws()).toBe(drawn);
    showPopup(true);
    expect(row.left()).toBe(155 * SECOND);
    advance(SECOND);
    expect(row.left()).toBe(154 * SECOND);
  });

  it("hands the clock back to the caller once the caller's reading is the later one", () => {
    const row = countdown(2 * SECOND);
    advance(2 * SECOND);
    expect(row.left()).toBe(0);
    advance(28 * SECOND);
    const later = new Date();
    row.rerender({ now: later });
    expect(row.result.current).toBe(later);
  });

  it("holds one waiting timer for a far deadline and none without a deadline", () => {
    const far = countdown(7 * 24 * 3600 * SECOND);
    expect(vi.getTimerCount()).toBe(1);
    const drawn = far.draws();
    advance(3 * 3600 * SECOND);
    expect(far.result.current).toBe(far.caller);
    expect(far.draws()).toBe(drawn);
    expect(vi.getTimerCount()).toBe(1);
    far.unmount();
    expect(vi.getTimerCount()).toBe(0);
    const none = countdown(null);
    expect(none.result.current).toBe(none.caller);
    expect(vi.getTimerCount()).toBe(0);
  });
});
