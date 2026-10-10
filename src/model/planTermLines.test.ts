import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setSystemTimeZone } from "@/model/timeZone";
import { planTermLines } from "./planTermLines";

/** Sunday 27/09/2026 09:45 in Vietnam. */
const NOW = new Date("2026-09-27T02:45:00Z");
const CODEX = { basis: "stated", endsAt: "2026-10-17T01:56:39+00:00", checkedAt: "2026-09-25T13:56:24Z" } as const;
const MONTHLY = { basis: "monthlyFrom", startedAt: "2026-07-31T03:40:09Z", checkedAt: "2026-09-27T02:45:00Z" } as const;

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("planTermLines", () => {
  it("counts down to a stated date with a provider-neutral confirmation note", () => {
    const lines = planTermLines(CODEX, NOW, "auto", "vi");
    expect(lines).toMatchObject({ left: "còn 19 ngày", day: "tới T7 17/10", soon: false });
    expect(lines?.note).toBe(
      "Gói hết kỳ lúc 8:56 · T7 17/10 · GMT+7. Ngày được nhà cung cấp xác nhận, kiểm tra lần cuối 25/09/2026. Gói tự gia hạn thì đây là ngày gia hạn.",
    );
  });

  it("marks a legacy monthly renewal as an estimate and warns three days out", () => {
    const lines = planTermLines(MONTHLY, NOW, "auto", "vi");
    expect(lines).toMatchObject({ left: "còn ~3 ngày", day: "tới ~T4 30/09", soon: true });
    expect(lines?.note).toBe(
      "Khoảng 10:40 · T4 30/09 · GMT+7, ước tính. Chu kỳ tháng được tính từ ngày đăng ký gói (31/07/2026); nhà cung cấp chưa xác nhận ngày gia hạn.",
    );
  });

  it("says when a stated period has ended until the service confirms a new date", () => {
    const lines = planTermLines(CODEX, new Date("2026-10-18T00:00:00Z"), "auto", "vi");
    expect(lines).toMatchObject({ left: "đã tới hạn", day: "T7 17/10", soon: true });
    expect(lines?.note).toMatch(/^Kỳ gói đã hết lúc 8:56 · T7 17\/10 · GMT\+7\. Kỳ mới chỉ hiện khi nhà cung cấp xác nhận ngày mới\./);
  });

  it("uses the actual Claude billing date without a monthly estimate", () => {
    const now = new Date("2026-10-10T01:00:00Z");
    const term = { basis: "stated", endsAt: "2026-11-03T07:03:48Z", checkedAt: now.toISOString() } as const;
    const lines = planTermLines(term, now, "auto", "vi");
    expect(lines).toMatchObject({ left: "còn 24 ngày", day: "tới T3 03/11", soon: false });
    expect(lines?.note).toBe("Gói hết kỳ lúc 14:03 · T3 03/11 · GMT+7. Ngày được nhà cung cấp xác nhận, kiểm tra lần cuối 10/10/2026. Gói tự gia hạn thì đây là ngày gia hạn.");
    const english = planTermLines(term, now, "12h", "en");
    expect(english?.note).toMatch(/The service confirmed this date, last checked Oct 10, 2026/);
    expect(english?.note).not.toMatch(/ChatGPT|estimated/);
    expect(planTermLines(term, new Date("2026-11-04T01:00:00Z"), "auto", "vi")).toMatchObject({ left: "đã tới hạn", day: "T3 03/11" });
  });

  it("uses relative days near the end and speaks English with a 12-hour clock", () => {
    expect(planTermLines(CODEX, new Date("2026-10-16T03:00:00Z"), "auto", "vi")).toMatchObject({ left: "còn 22 giờ", day: "tới ngày mai", soon: true });
    const english = planTermLines(MONTHLY, NOW, "12h", "en");
    expect(english).toMatchObject({ left: "~3 days left", day: "~Wed, Sep 30", soon: true });
    expect(english?.note.replace(/\s/g, " ")).toBe(
      "Around 10:40 AM · Wed, Sep 30 · GMT+7, estimated. This monthly estimate counts from the subscription start (Jul 31, 2026); the service has not confirmed a renewal date.",
    );
    expect(planTermLines(CODEX, new Date("2026-10-16T02:00:00Z"), "12h", "en")?.left).toBe("23 hours left");
  });

  it("returns nothing for a term it cannot read", () => {
    expect(planTermLines({ basis: "stated", endsAt: "later" }, NOW, "auto", "vi")).toBeNull();
  });

  it("hides an expired monthly estimate until the subscription is confirmed again", () => {
    const now = new Date("2026-10-01T00:00:00Z");
    expect(planTermLines(MONTHLY, now, "auto", "vi")).toBeNull();
    expect(planTermLines({ ...MONTHLY, checkedAt: now.toISOString() }, now, "auto", "vi")).toMatchObject({ day: "tới ~T7 31/10" });
  });
});
