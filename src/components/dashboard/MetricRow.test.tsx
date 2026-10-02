import { Profiler, type ReactNode } from "react";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { makeWidget, WEEK_SECONDS } from "@/model/testHelpers";
import { setSystemTimeZone } from "@/model/timeZone";
import type { WidgetData } from "@/model/widgetData";
import { useApp } from "@/state/store";
import { MetricRow } from "./MetricRow";

/** 16:04:00 on Friday 2 October 2026 in Hồ Chí Minh City. */
const BASE = Date.UTC(2026, 9, 2, 9, 4);
const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;

const vietnamese = { language: "vi", timeFormat: "24h", displayMode: "remaining" } as const;

function session(leftMs: number, extra: Partial<WidgetData> = {}): WidgetData {
  return makeWidget("Phiên 5h", "percent", 40, 100, { ...vietnamese, resetsAt: new Date(BASE + leftMs), periodDurationMs: 5 * HOUR, ...extra });
}

function weekly(leftMs: number, extra: Partial<WidgetData> = {}): WidgetData {
  return makeWidget("Tuần", "percent", 28, 100, { ...vietnamese, resetsAt: new Date(BASE + leftMs), periodDurationMs: WEEK_SECONDS * SECOND, ...extra });
}

function show(...rows: WidgetData[]) {
  const now = new Date(BASE);
  return render(rows.map((data) => <MetricRow key={data.title} data={data} now={now} condensedTop={false} />));
}

/** The row titled `title`, as its three stacked parts and the texts on its two lines. */
function rowOf(title: string) {
  const row = screen.getByText(title).closest<HTMLElement>(".uc-row")!;
  const parts = Array.from(row.children) as HTMLElement[];
  const texts = (line: HTMLElement | undefined) => Array.from(line?.children ?? []).map((part) => part.textContent);
  return { row, parts: parts.map((part) => part.className.split(" ")[0]), meter: parts[1]!, titleLine: texts(parts[0]), readingLine: texts(parts[2]) };
}

function advance(milliseconds: number): void {
  act(() => {
    vi.advanceTimersByTime(milliseconds);
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(BASE);
  setSystemTimeZone("Asia/Saigon");
  act(() => useApp.setState({ popupVisible: true }));
});

afterEach(() => {
  cleanup();
  act(() => useApp.setState({ popupVisible: true }));
  setSystemTimeZone(null);
  vi.useRealTimers();
});

describe("a limit's row", () => {
  it("puts the countdown on the title line and the exact moment beside the reading, the meter between them", () => {
    show(session(2 * HOUR + 34 * MINUTE + 30 * SECOND, { used: 100 }), weekly(25 * HOUR + 55 * MINUTE));
    const five = rowOf("Phiên 5h");
    expect(five.parts).toEqual(["uc-row-label", "uc-meter", "uc-row-primary"]);
    expect(five.titleLine).toEqual(["Phiên 5h", "Đặt lại sau 2 giờ 35 phút"]);
    expect(five.readingLine).toEqual(["Còn 0%", "Đặt lại lúc 18:38 · hôm nay"]);
    expect(five.meter.querySelector(".uc-meter-fill")).toBeNull();
    const week = rowOf("Tuần");
    expect(week.parts).toEqual(["uc-row-label", "uc-meter", "uc-row-primary"]);
    expect(week.titleLine).toEqual(["Tuần", "Đặt lại sau 1 ngày 1 giờ"]);
    expect(week.readingLine).toEqual(["Còn 72%", "Đặt lại lúc 17:59 · ngày mai"]);
    expect(week.meter.querySelector<HTMLElement>(".uc-meter-fill")?.style.width).toBe("72%");
  });

  it("keeps both texts where they are, once each, when Reset Times is saved as Exact Time", () => {
    show(session(2 * HOUR + 34 * MINUTE + 30 * SECOND, { resetDisplayMode: "absolute" }), weekly(25 * HOUR + 55 * MINUTE, { resetDisplayMode: "absolute", language: "en", timeFormat: "12h" }));
    const five = rowOf("Phiên 5h");
    expect(five.titleLine).toEqual(["Phiên 5h", "Đặt lại sau 2 giờ 35 phút"]);
    expect(five.readingLine).toEqual(["Còn 60%", "Đặt lại lúc 18:38 · hôm nay"]);
    expect(within(five.row).getAllByText(/Đặt lại/)).toHaveLength(2);
    expect(within(five.row).getAllByRole("button").map((button) => button.textContent)).toEqual(["Còn 60%"]);
    const week = rowOf("Tuần");
    expect(week.titleLine).toEqual(["Tuần", "Resets in 1d 1h"]);
    expect(week.readingLine.map((text) => text?.replace(/[  ]/g, " "))).toEqual(["72% left", "Resets at 5:59 PM · tomorrow"]);
  });

  it("counts the last five minutes down second by second, on the 5h row and the weekly row alike", () => {
    show(session(5 * MINUTE + 400), weekly(2 * MINUTE + 16 * SECOND + 700));
    expect(rowOf("Phiên 5h").titleLine[1]).toBe("Đặt lại sau 6 phút");
    expect(rowOf("Tuần").titleLine[1]).toBe("Đặt lại sau 02:17");
    advance(400);
    expect(rowOf("Phiên 5h").titleLine[1]).toBe("Đặt lại sau 05:00");
    advance(300);
    expect(rowOf("Tuần").titleLine[1]).toBe("Đặt lại sau 02:16");
    advance(700);
    expect(rowOf("Phiên 5h").titleLine[1]).toBe("Đặt lại sau 04:59");
    advance(300);
    expect(rowOf("Tuần").titleLine[1]).toBe("Đặt lại sau 02:15");
    expect(rowOf("Phiên 5h").readingLine).toEqual(["Còn 60%", "Đặt lại lúc 16:09 · hôm nay"]);
    expect(rowOf("Tuần").readingLine).toEqual(["Còn 72%", "Đặt lại lúc 16:06 · hôm nay"]);
  });

  it("ends on 00:01, then says the reset is due without going below zero or changing the reading", () => {
    show(session(3 * SECOND));
    const seen = [rowOf("Phiên 5h").titleLine[1]];
    for (let second = 0; second < 3; second += 1) {
      advance(SECOND);
      seen.push(rowOf("Phiên 5h").titleLine[1]);
    }
    expect(seen).toEqual(["Đặt lại sau 00:03", "Đặt lại sau 00:02", "Đặt lại sau 00:01", "Sắp đặt lại"]);
    expect(rowOf("Phiên 5h").readingLine).toEqual(["Còn 60%"]);
    expect(vi.getTimerCount()).toBe(0);
    advance(MINUTE);
    expect(rowOf("Phiên 5h").titleLine[1]).toBe("Sắp đặt lại");
    expect(rowOf("Phiên 5h").readingLine).toEqual(["Còn 60%"]);
  });

  it("draws only the row in its last five minutes each second and asks no provider for anything", () => {
    const api = new MockBackend();
    setBackend(api);
    const refresh = vi.spyOn(api, "refresh");
    const draws: Record<string, number> = {};
    const counted = (id: string, row: ReactNode) => (
      <Profiler id={id} onRender={() => (draws[id] = (draws[id] ?? 0) + 1)}>
        {row}
      </Profiler>
    );
    const now = new Date(BASE);
    render(
      <>
        {counted("session", <MetricRow data={session(4 * MINUTE)} now={now} condensedTop={false} />)}
        {counted("weekly", <MetricRow data={weekly(3 * 24 * HOUR)} now={now} condensedTop={false} />)}
      </>,
    );
    const before = { ...draws };
    for (let second = 0; second < 30; second += 1) advance(SECOND);
    expect(rowOf("Phiên 5h").titleLine[1]).toBe("Đặt lại sau 03:30");
    expect(draws.session! - before.session!).toBe(30);
    expect(draws.weekly).toBe(before.weekly);
    expect(rowOf("Tuần").titleLine[1]).toBe("Đặt lại sau 3 ngày 0 giờ");
    expect(refresh).not.toHaveBeenCalled();
  });

  it("stops its timers while the popup is hidden and shows the right second when it reopens", () => {
    show(session(4 * MINUTE));
    act(() => useApp.setState({ popupVisible: false }));
    expect(vi.getTimerCount()).toBe(0);
    advance(90 * SECOND);
    act(() => useApp.setState({ popupVisible: true }));
    expect(rowOf("Phiên 5h").titleLine[1]).toBe("Đặt lại sau 02:30");
    advance(SECOND);
    expect(rowOf("Phiên 5h").titleLine[1]).toBe("Đặt lại sau 02:29");
  });

  it("leaves the pace note to the tooltips beside a countdown, and on the title line of a row with none", () => {
    const closeToLimit = weekly(WEEK_SECONDS * SECOND * 0.5, { used: 46 });
    const spentSession = session(2 * HOUR, { used: 100 });
    const spentCredits = makeWidget("Tín dụng", "dollars", 20, 20, vietnamese);
    show(closeToLimit, spentSession, spentCredits);
    expect(rowOf("Tuần").titleLine).toEqual(["Tuần", "Đặt lại sau 3 ngày 12 giờ"]);
    expect(within(rowOf("Tuần").row).queryByText(/Dư ~/)).toBeNull();
    expect(rowOf("Phiên 5h").titleLine).toEqual(["Phiên 5h", "Đặt lại sau 2 giờ"]);
    expect(within(rowOf("Phiên 5h").row).queryByText("Đã hết hạn mức")).toBeNull();
    expect(rowOf("Tín dụng").titleLine).toEqual(["Tín dụng", "Đã hết hạn mức"]);
  });

  it("keeps a status that is not a reset beside the reading: not started, no data, a provider's own words", () => {
    const notStarted = session(2 * HOUR, { used: 0, sessionStartSignal: "zeroUsage" });
    const noData = weekly(3 * 24 * HOUR, { hasData: false });
    const paused = makeWidget("Fable", "percent", 10, 100, { ...vietnamese, resetsAt: new Date(BASE + HOUR), subtitleOverride: "Paused" });
    const rolledOver = makeWidget("Sonnet", "percent", 0, 100, { ...vietnamese, periodDurationMs: WEEK_SECONDS * SECOND });
    show(notStarted, noData, paused, rolledOver);
    expect([rowOf("Phiên 5h").titleLine, rowOf("Phiên 5h").readingLine]).toEqual([["Phiên 5h"], ["Còn 100%", "Chưa bắt đầu"]]);
    expect([rowOf("Tuần").titleLine, rowOf("Tuần").readingLine]).toEqual([["Tuần"], ["—", "Không có dữ liệu"]]);
    expect([rowOf("Fable").titleLine, rowOf("Fable").readingLine]).toEqual([["Fable"], ["Còn 90%", "Paused"]]);
    expect([rowOf("Sonnet").titleLine, rowOf("Sonnet").readingLine]).toEqual([["Sonnet", "Đặt lại sau 7 ngày 0 giờ"], ["Còn 100%"]]);
    expect(vi.getTimerCount()).toBe(0);
  });
});
