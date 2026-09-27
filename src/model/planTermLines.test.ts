import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setSystemTimeZone } from "@/model/timeZone";
import { planTermLines } from "./planTermLines";

/** Sunday 27/09/2026 09:45 in Vietnam. */
const NOW = new Date("2026-09-27T02:45:00Z");
const CODEX = { basis: "stated", endsAt: "2026-10-17T01:56:39+00:00", checkedAt: "2026-09-25T13:56:24Z" } as const;
const CLAUDE = { basis: "monthlyFrom", startedAt: "2026-07-31T03:40:09Z" } as const;

beforeEach(() => setSystemTimeZone("Asia/Saigon"));
afterEach(() => setSystemTimeZone(null));

describe("planTermLines", () => {
  it("counts down to the date ChatGPT states and names its source", () => {
    const lines = planTermLines(CODEX, NOW, "auto", "vi");
    expect(lines).toMatchObject({ left: "còn 19 ngày", day: "tới T7 17/10", soon: false });
    expect(lines?.note).toBe(
      "Gói hết kỳ lúc 8:56 · T7 17/10 · GMT+7. Ngày do ChatGPT ghi trong phiên đăng nhập, kiểm tra lần cuối 25/09/2026. Gói tự gia hạn thì đây là ngày gia hạn.",
    );
  });

  it("marks Claude's monthly renewal as an estimate and warns three days out", () => {
    const lines = planTermLines(CLAUDE, NOW, "auto", "vi");
    expect(lines).toMatchObject({ left: "còn ~3 ngày", day: "tới ~T4 30/09", soon: true });
    expect(lines?.note).toBe(
      "Khoảng 10:40 · T4 30/09 · GMT+7, ước tính. Anthropic chỉ cho biết ngày đăng ký gói (31/07/2026), không cho biết ngày gia hạn, nên ngày này tính theo chu kỳ tháng từ ngày đăng ký.",
    );
  });

  it("says when a stated period has ended and ChatGPT has not sent the next date yet", () => {
    const lines = planTermLines(CODEX, new Date("2026-10-18T00:00:00Z"), "auto", "vi");
    expect(lines).toMatchObject({ left: "đã tới hạn", day: "T7 17/10", soon: true });
    expect(lines?.note).toMatch(/^Kỳ gói đã hết lúc 8:56 · T7 17\/10 · GMT\+7\. ChatGPT gửi ngày của kỳ mới/);
  });

  it("uses relative days near the end and speaks English with a 12-hour clock", () => {
    expect(planTermLines(CODEX, new Date("2026-10-16T03:00:00Z"), "auto", "vi")).toMatchObject({ left: "còn 22 giờ", day: "tới ngày mai", soon: true });
    const english = planTermLines(CLAUDE, NOW, "12h", "en");
    expect(english).toMatchObject({ left: "~3 days left", day: "~Wed, Sep 30", soon: true });
    expect(english?.note.replace(/\s/g, " ")).toBe(
      "Around 10:40 AM · Wed, Sep 30 · GMT+7, estimated. Anthropic only states when the subscription started (Jul 31, 2026), not when it renews, so this counts monthly from that day.",
    );
    expect(planTermLines(CODEX, new Date("2026-10-16T02:00:00Z"), "12h", "en")?.left).toBe("23 hours left");
  });

  it("returns nothing for a term it cannot read", () => {
    expect(planTermLines({ basis: "stated", endsAt: "later" }, NOW, "auto", "vi")).toBeNull();
  });
});
