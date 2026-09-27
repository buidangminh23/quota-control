import { sinceText } from "./parts";

describe("time since a reset", () => {
  const now = new Date("2026-09-27T09:00:00Z");
  const before = (minutes: number) => new Date(now.getTime() - minutes * 60_000);

  it("says it in the largest whole unit, the way a feed does", () => {
    expect(sinceText(before(0.2), now, "vi")).toBe("1 phút trước");
    expect(sinceText(before(59), now, "vi")).toBe("59 phút trước");
    expect(sinceText(before(60), now, "vi")).toBe("1 giờ trước");
    expect(sinceText(before(15 * 60 + 42), now, "vi")).toBe("15 giờ trước");
    expect(sinceText(before(24 * 60), now, "vi")).toBe("1 ngày trước");
    expect(sinceText(before(3 * 24 * 60 + 600), now, "vi")).toBe("3 ngày trước");
  });

  it("reads in English too", () => {
    expect(sinceText(before(7 * 60 + 5), now, "en")).toBe("7 hours ago");
    expect(sinceText(before(2 * 24 * 60), now, "en")).toBe("2 days ago");
  });
});
