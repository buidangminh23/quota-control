import { roundHalfAwayFromZero } from "./decimal";
import {
  clockCountdown,
  compactDuration,
  deadlineLabel,
  FINAL_COUNTDOWN_SECONDS,
  formatCostPerMtok,
  formatNumber,
  formatValue,
  hourWindowLabel,
  resetAbsoluteLabel,
  resetCountdownLabel,
  resetMomentLabel,
  restoreLabel,
  secondsUntil,
  setDongRate,
  setSystemClockPreference,
  shortTime,
  totalSpendRingCenter,
  usesTwentyFourHour,
  type TotalSpendMetric,
} from "./format";
import { plainSpaces } from "./testHelpers";

describe("rounding", () => {
  it("rounds half-even on the shortest decimal like ICU", () => {
    expect(formatNumber(2.25, "count", "full", "en")).toBe("2.2");
    expect(formatNumber(2.35, "count", "full", "en")).toBe("2.4");
    expect(formatNumber(0.125, "dollars", "full", "en")).toBe("$0.12");
    expect(formatNumber(0.295, "dollars", "full", "en")).toBe("$0.30");
  });

  it("matches Swift rounded() for half-away rounding", () => {
    expect(roundHalfAwayFromZero(36.5)).toBe(37);
    expect(roundHalfAwayFromZero(-2.5)).toBe(-3);
    expect(roundHalfAwayFromZero(2.4)).toBe(2);
  });
});

describe("MetricFormatter (English, upstream parity)", () => {
  it("abbreviates dollars above a thousand per style", () => {
    expect(formatNumber(42, "dollars", "tray", "en")).toBe("$42");
    expect(formatNumber(129.81, "dollars", "tray", "en")).toBe("$130");
    expect(formatNumber(2059.07, "dollars", "tray", "en")).toBe("$2.1K");
    expect(formatNumber(40.76, "dollars", "row", "en")).toBe("$40.76");
    expect(formatNumber(2059.07, "dollars", "row", "en")).toBe("$2.1K");
    expect(formatNumber(2059.07, "dollars", "full", "en")).toBe("$2,059.07");
  });

  it("abbreviates counts in tray and row but keeps every digit in full", () => {
    expect(formatNumber(56_904_995, "count", "tray", "en")).toBe("56.9M");
    expect(formatNumber(56_904_995, "count", "row", "en")).toBe("56.9M");
    expect(formatNumber(56_904_995, "count", "full", "en")).toBe("56,904,995");
    expect(formatNumber(1_485_201_513, "count", "row", "en")).toBe("1.5B");
    expect(formatNumber(999_960, "count", "row", "en")).toBe("1M");
    expect(formatNumber(738_500, "count", "row", "en")).toBe("738.5K");
    expect(formatNumber(820.6, "count", "row", "en")).toBe("820.6");
  });

  it("rounds percent to whole and clamps out-of-range samples", () => {
    expect(formatNumber(95, "percent", "full", "en")).toBe("95%");
    expect(formatNumber(95.4, "percent", "tray", "en")).toBe("95%");
    expect(formatNumber(-5, "percent", "full", "en")).toBe("0%");
    expect(formatNumber(130, "percent", "full", "en")).toBe("100%");
    expect(formatNumber(100.6, "percent", "row", "en")).toBe("100%");
    expect(formatNumber(Number.NaN, "percent", "row", "en")).toBe("0%");
  });

  it("appends unit labels when present", () => {
    const credits = { number: 772, kind: "count" as const, label: "credits", estimated: false };
    expect(formatValue(credits, "row", "en")).toBe("772 credits");
    expect(formatValue(credits, "full", "en")).toBe("772 credits");
    expect(formatValue({ number: 56_904_995, kind: "count", estimated: false }, "row", "en")).toBe("56.9M");
    expect(formatValue({ number: 56_904_995, kind: "count", estimated: false }, "full", "en")).toBe("56,904,995");
  });

  it("formats cost per million tokens", () => {
    expect(formatCostPerMtok(32, "tray", "en")).toBe("$32/MTok");
    expect(formatCostPerMtok(32.1, "row", "en")).toBe("$32.10/MTok");
    expect(formatCostPerMtok(32.1, "full", "en")).toBe("$32.10/MTok");
    expect(formatCostPerMtok(2059.07, "tray", "en")).toBe("$2.1K/MTok");
    expect(formatCostPerMtok(2059.07, "full", "en")).toBe("$2,059.07/MTok");
  });

  it("splits the ring center into a figure and a unit", () => {
    const cases: Array<[number, TotalSpendMetric, string, string]> = [
      [533, "cost", "$533", "dollars"],
      [2059.07, "cost", "$2.1K", "dollars"],
      [12_400_000, "tokens", "12.4", "million"],
      [1_500_000_000, "tokens", "1.5", "billion"],
      [820.6, "tokens", "820.6", "tokens"],
      [1.37, "costPerMtok", "$1.37", "MTok"],
    ];
    for (const [value, metric, primary, unit] of cases) {
      expect(totalSpendRingCenter(value, metric, "en")).toEqual({ primary, unit });
    }
  });
});

describe("MetricFormatter (Vietnamese)", () => {
  it("uses vi-VN separators and CLDR compact units", () => {
    expect(plainSpaces(formatNumber(56_904_995, "count", "row", "vi"))).toBe("56,9 Tr");
    expect(plainSpaces(formatNumber(1_485_201_513, "count", "row", "vi"))).toBe("1,5 T");
    expect(plainSpaces(formatNumber(1_234, "count", "row", "vi"))).toBe("1,2 N");
    expect(formatNumber(56_904_995, "count", "full", "vi")).toBe("56.904.995");
    expect(formatNumber(820.6, "count", "row", "vi")).toBe("820,6");
  });

  it("writes dollars after the amount", () => {
    expect(plainSpaces(formatNumber(40.76, "dollars", "row", "vi"))).toBe("40,76 $");
    expect(plainSpaces(formatNumber(2059.07, "dollars", "full", "vi"))).toBe("2.059,07 $");
    expect(plainSpaces(formatNumber(129.81, "dollars", "tray", "vi"))).toBe("130 $");
    expect(plainSpaces(formatNumber(2059.07, "dollars", "tray", "vi"))).toBe("2,1 N $");
  });

  it("keeps percent identical across languages", () => {
    expect(formatNumber(95.4, "percent", "row", "vi")).toBe("95%");
  });

  it("translates unit labels", () => {
    const credits = { number: 772, kind: "count" as const, label: "credits", estimated: false };
    expect(formatValue(credits, "row", "vi")).toBe("772 tín dụng");
    const tokens = { number: 1_200_000, kind: "count" as const, label: "tokens", estimated: false };
    expect(plainSpaces(formatValue(tokens, "row", "vi"))).toBe("1,2 Tr token");
  });

  it("formats cost per million tokens and ring centers", () => {
    expect(plainSpaces(formatCostPerMtok(32.1, "row", "vi"))).toBe("32,10 $/triệu token");
    expect(totalSpendRingCenter(12_400_000, "tokens", "vi")).toEqual({ primary: "12,4", unit: "triệu" });
    expect(totalSpendRingCenter(1_500_000_000, "tokens", "vi")).toEqual({ primary: "1,5", unit: "tỷ" });
    const cost = totalSpendRingCenter(533, "cost", "vi");
    expect(plainSpaces(cost.primary)).toBe("533 $");
    expect(cost.unit).toBe("đô la");
  });
});

describe("Formatters", () => {
  it("always shows hours at the day scale", () => {
    expect(compactDuration(4 * 24 * 3600 + 52 * 60, "en")).toBe("4d 0h");
    expect(compactDuration(7 * 24 * 3600, "en")).toBe("7d 0h");
    expect(compactDuration(9 * 24 * 3600 + 21 * 3600, "en")).toBe("9d 21h");
    expect(compactDuration(5 * 3600, "en")).toBe("5h");
    expect(compactDuration(3 * 3600 + 25 * 60, "en")).toBe("3h 25m");
    expect(compactDuration(52 * 60, "en")).toBe("52m");
    expect(compactDuration(0, "en")).toBeNull();
    expect(compactDuration(Number.POSITIVE_INFINITY, "en")).toBeNull();
  });

  it("names a limit window of whole hours under a day", () => {
    expect(hourWindowLabel(5 * 3_600_000)).toBe("5h");
    expect(hourWindowLabel(3_600_000)).toBe("1h");
    expect(hourWindowLabel(90 * 60_000)).toBeNull();
    expect(hourWindowLabel(24 * 3_600_000)).toBeNull();
    expect(hourWindowLabel(0)).toBeNull();
    expect(hourWindowLabel(undefined)).toBeNull();
  });

  it("spells durations out in Vietnamese", () => {
    expect(compactDuration(24 * 3600 + 6 * 3600, "vi")).toBe("1 ngày 6 giờ");
    expect(compactDuration(3 * 3600 + 25 * 60, "vi")).toBe("3 giờ 25 phút");
    expect(compactDuration(5 * 3600, "vi")).toBe("5 giờ");
    expect(compactDuration(52 * 60, "vi")).toBe("52 phút");
  });

  it("buckets absolute labels by local day", () => {
    const now = new Date(2024, 5, 1, 12);
    expect(resetAbsoluteLabel(new Date(now.getTime() + 2 * 3_600_000), now, "12h", "en")?.startsWith("Resets today at ")).toBe(true);
    expect(resetAbsoluteLabel(new Date(2024, 5, 2, 13), now, "12h", "en")?.startsWith("Resets tomorrow at ")).toBe(true);
    expect(resetAbsoluteLabel(new Date(2024, 5, 6, 12), now, "12h", "en")?.startsWith("Resets Jun 6 at ")).toBe(true);
    expect(resetAbsoluteLabel(new Date(now.getTime() - 1000), now, "12h", "en")).toBe("Resets soon");
  });

  it("gives the exact restore time with today, tomorrow or the weekday and date", () => {
    const now = new Date(2026, 8, 26, 12);
    expect(restoreLabel(new Date(2026, 8, 26, 18, 38), now, "24h", "vi")).toBe("Hồi lại lúc 18:38 · hôm nay");
    expect(restoreLabel(new Date(2026, 8, 27, 9, 5), now, "24h", "vi")).toBe("Hồi lại lúc 9:05 · ngày mai");
    expect(restoreLabel(new Date(2026, 9, 2, 13, 5), now, "24h", "vi")).toBe("Hồi lại lúc 13:05 · T6 02/10");
    expect(restoreLabel(new Date(2026, 9, 4, 8, 0), now, "24h", "vi")).toBe("Hồi lại lúc 8:00 · CN 04/10");
    expect(plainSpaces(restoreLabel(new Date(2026, 9, 2, 13, 5), now, "12h", "en"))).toBe("Back at 1:05 PM · Fri, Oct 2");
    expect(plainSpaces(restoreLabel(new Date(2026, 8, 27, 9, 5), now, "12h", "en"))).toBe("Back at 9:05 AM · tomorrow");
    expect(restoreLabel(now, now, "24h", "vi")).toBeNull();
    expect(restoreLabel(new Date(now.getTime() - 1000), now, "24h", "vi")).toBeNull();
  });

  it("counts whole seconds left, a second that has begun as a whole one, and never below zero", () => {
    const now = new Date(2026, 9, 2, 16, 4);
    const left = (milliseconds: number) => secondsUntil(new Date(now.getTime() + milliseconds), now);
    expect([left(300_000), left(299_001), left(299_000), left(1), left(0), left(-1), left(-60_000)]).toEqual([300, 300, 299, 1, 0, 0, 0]);
    expect([300, 299.2, 299, 61, 60, 59.5, 1, 0.001, 0, -4].map(clockCountdown)).toEqual(["05:00", "05:00", "04:59", "01:01", "01:00", "01:00", "00:01", "00:01", "00:00", "00:00"]);
  });

  it("counts a reset down to the second through its last five minutes only", () => {
    const now = new Date(2026, 9, 2, 16, 4);
    const label = (milliseconds: number, language: "vi" | "en" = "vi") => resetCountdownLabel(new Date(now.getTime() + milliseconds), now, language);
    expect(FINAL_COUNTDOWN_SECONDS).toBe(300);
    expect(label(3_600_000)).toBe("Đặt lại sau 1 giờ");
    expect(label(300_001)).toBe("Đặt lại sau 6 phút");
    expect(label(300_000)).toBe("Đặt lại sau 05:00");
    expect(label(299_000)).toBe("Đặt lại sau 04:59");
    expect(label(1_000)).toBe("Đặt lại sau 00:01");
    expect(label(400)).toBe("Đặt lại sau 00:01");
    expect(label(0)).toBe("Sắp đặt lại");
    expect(label(-90_000)).toBe("Sắp đặt lại");
    expect(label(299_000, "en")).toBe("Resets in 04:59");
    expect(label(300_001, "en")).toBe("Resets in 6m");
    expect(label(-1, "en")).toBe("Resets soon");
    expect(resetCountdownLabel(new Date(Number.NaN), now, "vi")).toBeNull();
  });

  it("keeps calling the last five minutes soon everywhere else a deadline is worded", () => {
    const now = new Date(2026, 9, 2, 16, 4);
    const inFourMinutes = new Date(now.getTime() + 240_000);
    expect(deadlineLabel("resets", inFourMinutes, "relative", now, "24h", "vi")).toBe("Sắp đặt lại");
    expect(deadlineLabel("limit", inFourMinutes, "relative", now, "12h", "en")).toBe("Limit soon");
    expect(deadlineLabel("resetExpires", inFourMinutes, "relative", now, "24h", "vi")).toBe("Sắp hết hạn");
    expect(deadlineLabel("resets", new Date(now.getTime() + 300_001), "relative", now, "24h", "vi")).toBe("Đặt lại sau 6 phút");
  });

  it("names the exact moment a limit resets beside its reading", () => {
    const now = new Date(2026, 8, 26, 12);
    expect(resetMomentLabel(new Date(2026, 8, 26, 18, 38), now, "24h", "vi")).toBe("Đặt lại lúc 18:38 · hôm nay");
    expect(resetMomentLabel(new Date(2026, 8, 27, 9, 5), now, "24h", "vi")).toBe("Đặt lại lúc 9:05 · ngày mai");
    expect(resetMomentLabel(new Date(2026, 9, 2, 13, 5), now, "24h", "vi")).toBe("Đặt lại lúc 13:05 · T6 02/10");
    expect(plainSpaces(resetMomentLabel(new Date(2026, 9, 2, 13, 5), now, "12h", "en"))).toBe("Resets at 1:05 PM · Fri, Oct 2");
    expect(plainSpaces(resetMomentLabel(new Date(2026, 8, 27, 9, 5), now, "12h", "en"))).toBe("Resets at 9:05 AM · tomorrow");
    expect(resetMomentLabel(new Date(2026, 8, 26, 18, 38), now, "24h", "en")).toBe("Resets at 18:38 · today");
    expect(resetMomentLabel(now, now, "24h", "vi")).toBeNull();
    expect(resetMomentLabel(new Date(now.getTime() - 1000), now, "24h", "vi")).toBeNull();
  });

  it("buckets absolute labels by local day in Vietnamese", () => {
    const now = new Date(2024, 5, 1, 12);
    expect(resetAbsoluteLabel(new Date(2024, 5, 1, 18, 38), now, "24h", "vi")).toBe("Đặt lại lúc 18:38 hôm nay");
    expect(resetAbsoluteLabel(new Date(2024, 5, 2, 9, 5), now, "24h", "vi")).toBe("Đặt lại lúc 9:05 ngày mai");
    expect(resetAbsoluteLabel(new Date(2024, 5, 6, 12), now, "24h", "vi")).toBe("Đặt lại lúc 12:00 ngày 6/6");
    expect(resetAbsoluteLabel(new Date(now.getTime() - 1000), now, "24h", "vi")).toBe("Sắp đặt lại");
  });

  it("shares one deadline format across verbs and modes", () => {
    const now = new Date(2024, 5, 1, 12);
    const inTwoHours = new Date(now.getTime() + (2 * 3600 + 360) * 1000);
    expect(deadlineLabel("limit", inTwoHours, "relative", now, "12h", "en")).toBe("Limit in 2h 6m");
    expect(deadlineLabel("limit", inTwoHours, "absolute", now, "12h", "en")?.startsWith("Limit today at ")).toBe(true);
    expect(deadlineLabel("limit", new Date(now.getTime() + 60_000), "relative", now, "12h", "en")).toBe("Limit soon");
    expect(deadlineLabel("limit", new Date(now.getTime() - 1000), "absolute", now, "12h", "en")).toBe("Limit soon");
    expect(deadlineLabel("resetExpires", inTwoHours, "relative", now, "12h", "en")).toBe("Reset expires in 2h 6m");
    expect(deadlineLabel("limit", inTwoHours, "relative", now, "24h", "vi")).toBe("Hết hạn mức sau 2 giờ 6 phút");
    expect(deadlineLabel("limit", new Date(now.getTime() + 60_000), "relative", now, "24h", "vi")).toBe("Sắp hết hạn mức");
    expect(deadlineLabel("resetExpires", inTwoHours, "relative", now, "24h", "vi")).toBe("Hết hạn sau 2 giờ 6 phút");
  });

  it("says which clock shortTime draws, so the island and the widgets draw the same one", () => {
    const at = new Date(2024, 5, 1, 18, 38);
    const twelveHour = (text: string) => /CH|PM/.test(text);
    try {
      for (const preference of [null, true, false]) {
        setSystemClockPreference(preference);
        for (const format of ["auto", "12h", "24h"] as const) {
          for (const language of ["vi", "en"] as const) {
            expect(usesTwentyFourHour(format, language), `${preference} ${format} ${language}`).toBe(!twelveHour(shortTime(at, format, language)));
          }
        }
      }
      setSystemClockPreference(null);
      expect(usesTwentyFourHour("auto", "vi")).toBe(true);
      expect(usesTwentyFourHour("auto", "en")).toBe(false);
      setSystemClockPreference(true);
      expect(usesTwentyFourHour("auto", "en")).toBe(true);
      expect(usesTwentyFourHour("12h", "en")).toBe(false);
    } finally {
      setSystemClockPreference(null);
    }
  });

  it("honors the 12-hour and 24-hour time formats", () => {
    const now = new Date(2024, 5, 1, 12);
    const at = new Date(2024, 5, 1, 18, 38);
    expect(plainSpaces(resetAbsoluteLabel(at, now, "12h", "en"))).toBe("Resets today at 6:38 PM");
    expect(resetAbsoluteLabel(at, now, "24h", "en")).toBe("Resets today at 18:38");
    expect(plainSpaces(shortTime(at, "12h", "vi"))).toBe("6:38 CH");
    expect(shortTime(at, "24h", "vi")).toBe("18:38");
  });
});

describe("money on the Vietnamese UI", () => {
  afterEach(() => setDongRate(null));

  it("shows đồng at the known rate and keeps dollars in English or without a rate", () => {
    expect(plainSpaces(formatNumber(12.5, "dollars", "row", "vi"))).toBe("12,50 $");
    setDongRate(26_170);
    expect(plainSpaces(formatNumber(12.5, "dollars", "row", "vi"))).toBe("327,1 N ₫");
    expect(plainSpaces(formatNumber(12.5, "dollars", "full", "vi"))).toBe("327.125 ₫");
    expect(plainSpaces(formatNumber(0.01, "dollars", "row", "vi"))).toBe("262 ₫");
    expect(formatNumber(12.5, "dollars", "row", "en")).toBe("$12.50");
    setDongRate(Number.NaN);
    expect(plainSpaces(formatNumber(12.5, "dollars", "row", "vi"))).toBe("12,50 $");
  });
});
