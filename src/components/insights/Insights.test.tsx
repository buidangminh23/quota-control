import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { resetInsights, useInsights } from "@/state/insights";
import { updateSettings, useApp } from "@/state/store";
import { App } from "@/App";
import { closeMenu } from "../ui/menu";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));
const notify = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("@/platform/system", async (original) => ({ ...(await original<typeof import("@/platform/system")>()), notify }));

async function renderApp(settings?: Record<string, unknown>) {
  const api = new MockBackend();
  if (settings) await api.saveDocument("settings", settings);
  setBackend(api);
  render(<App />);
  await screen.findByText("Claude · Công ty");
  return api;
}

function openTab(name: string) {
  fireEvent.click(screen.getByRole("tab", { name }));
}

beforeEach(() => {
  window.localStorage.clear();
  notify.mockClear();
});

afterEach(async () => {
  closeMenu();
  cleanup();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() => {
    resetInsights();
    useApp.setState({ screen: "dashboard", previousScreen: "dashboard", tabMotion: null });
  });
});

describe("dashboard tabs", () => {
  it("adds Benchmark and Reset after Bảng giá and cycles through the tabs that are shown", async () => {
    await renderApp();
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["Hạn mức", "Token", "Bảng giá", "Benchmark", "Reset"]);
    const limits = screen.getByRole("tab", { name: "Hạn mức" });
    act(() => limits.focus());
    fireEvent.keyDown(limits, { key: "ArrowLeft" });
    expect(screen.getByRole("tab", { name: "Reset" })).toHaveAttribute("aria-selected", "true");
    act(() => updateSettings({ showTotalSpend: false }));
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["Hạn mức", "Bảng giá", "Benchmark", "Reset"]);
    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true, shiftKey: true });
    expect(screen.getByRole("tab", { name: "Reset" })).toHaveAttribute("aria-selected", "true");
    act(() => updateSettings({ showResetsTab: false }));
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
    act(() => updateSettings({ showBenchmarkTab: false }));
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["Hạn mức", "Bảng giá"]);
  });
});

describe("Benchmark tab", () => {
  it("scores models on the user's projects with intervals and lists the ones short of data", async () => {
    await renderApp();
    openTab("Benchmark");
    const card = await screen.findByRole("article", { name: "Claude Opus 5" });
    expect(within(card).getByText("Claude Code · ", { exact: false })).toBeInTheDocument();
    expect(within(card).getByText("Sửa file thành công")).toBeInTheDocument();
    expect(within(card).getByText("Kết thúc với kiểm tra đạt")).toBeInTheDocument();
    expect(within(card).getByText("Không bị dừng giữa chừng")).toBeInTheDocument();
    expect(within(card).getByText(/^khoảng \d/)).toBeInTheDocument();
    expect(screen.getByText(/^Chưa đủ dữ liệu \(\d+\)$/)).toBeInTheDocument();
    expect(screen.getByText("Claude Sonnet 5")).toBeInTheDocument();
    expect(screen.getByText(/lượt có kiểm tra 0\/10/)).toBeInTheDocument();
    expect(screen.getByText(/^Đã quét 1\.284 tệp nhật ký/)).toBeInTheDocument();
  });

  it("rescans the local logs on request", async () => {
    const api = await renderApp();
    const rescan = vi.spyOn(api, "rescanModelQuality");
    openTab("Benchmark");
    await screen.findByRole("article", { name: "Claude Opus 5" });
    fireEvent.click(screen.getByRole("button", { name: "Quét lại nhật ký" }));
    expect(rescan).toHaveBeenCalledOnce();
    expect(screen.getByText("Đang quét nhật ký…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Quét lại nhật ký" })).toBeDisabled();
  });

  it("narrows to a project and a period", async () => {
    await renderApp();
    openTab("Benchmark");
    await screen.findByRole("article", { name: "Claude Opus 5" });
    fireEvent.click(screen.getByRole("button", { name: "Dự án: Mọi dự án" }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: /^demo-app · / }));
    expect(useInsights.getState().project).toBe("demo-app");
    expect(screen.getByRole("button", { name: "Dự án: demo-app" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "Tất cả" }));
    expect(await screen.findByRole("article", { name: "Claude Opus 5" })).toBeInTheDocument();
    expect(useInsights.getState().qualityKey).toBe("|");
  });

  it("shows public boards for every kind of work, marking the models in use", async () => {
    await renderApp();
    openTab("Benchmark");
    await screen.findByRole("article", { name: "Claude Opus 5" });
    fireEvent.click(screen.getByRole("radio", { name: "Công khai" }));
    const eci = await screen.findByText("GPT-6 Astra");
    expect(eci.closest(".uc-insight-row")).toHaveTextContent("Đang dùng");
    expect(screen.getByText(/Nguồn: Epoch AI, giấy phép CC BY 4\.0/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^Bảng xếp hạng:/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Epoch AI" }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Hình ảnh, không gian và 3D" }));
    const cad = await screen.findByRole("article", { name: "CadEval" });
    expect(within(cad).getByText(/thiết kế 3D tham số/)).toBeInTheDocument();
    expect(within(cad).getByText("Không có model nào ra mắt trong 12 tháng qua được chấm")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^Bảng xếp hạng:/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Arena" }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Sửa video" }));
    expect(await screen.findByText("wan3.0")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^Bảng xếp hạng:/ }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "3D Arena · Tạo mô hình 3D" }));
    expect(await screen.findByText("PicGen3D")).toBeInTheDocument();
  });

  it("fetches the shown public board again on request", async () => {
    const api = await renderApp();
    const refresh = vi.spyOn(api, "refreshPublicFeed");
    openTab("Benchmark");
    await screen.findByRole("article", { name: "Claude Opus 5" });
    fireEvent.click(screen.getByRole("radio", { name: "Công khai" }));
    await screen.findByText("GPT-6 Astra");
    expect(screen.getByText(/^Tải 2 giờ/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Làm mới" }));
    expect(refresh).toHaveBeenCalledExactlyOnceWith("epochScores");
    expect(screen.getByRole("button", { name: "Đang làm mới…" })).toBeDisabled();
    expect(await screen.findByText("Vừa tải")).toBeInTheDocument();
  });

  it("dates a board by its last good download after a failed fetch", async () => {
    await renderApp();
    openTab("Benchmark");
    await screen.findByRole("article", { name: "Claude Opus 5" });
    fireEvent.click(screen.getByRole("radio", { name: "Công khai" }));
    await screen.findByText("GPT-6 Astra");
    const snapshot = useInsights.getState().feeds.epochScores!;
    const downloaded = new Date(Date.now() - 3 * 3_600_000).toISOString();
    act(() =>
      useInsights.setState({
        feeds: { ...useInsights.getState().feeds, epochScores: { ...snapshot, fetchedAt: downloaded, checkedAt: new Date().toISOString(), error: "timed out", stale: true } },
      }),
    );
    expect(screen.getByText("Lần tải gần nhất bị lỗi, đang hiện bản đã lưu.")).toBeInTheDocument();
    expect(screen.getByText(/^Tải 3 giờ/)).toBeInTheDocument();
  });

  it("compares the user's models across sources and adds one by search", async () => {
    await renderApp();
    openTab("Benchmark");
    await screen.findByRole("article", { name: "Claude Opus 5" });
    fireEvent.click(screen.getByRole("radio", { name: "So sánh" }));
    expect(await screen.findByText("Điểm chất lượng")).toBeInTheDocument();
    expect(screen.getByText("Trên dự án của bạn · 30 ngày")).toBeInTheDocument();
    const selected = useInsights.getState().compared;
    expect(selected).toBeNull();
    expect(screen.getAllByRole("button", { name: /^Bỏ / })).toHaveLength(3);
    expect(screen.queryByRole("button", { name: "Thêm model" })).not.toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: /^Bỏ / })[2]!);
    fireEvent.click(screen.getByRole("button", { name: "Thêm model" }));
    fireEvent.change(screen.getByRole("searchbox", { name: "Tìm model…" }), { target: { value: "kimi" } });
    fireEvent.click(screen.getByRole("button", { name: "Kimi K3" }));
    expect(useInsights.getState().compared).toContain("kimi-k3");
    expect(screen.getByText("Chỉ số ECI")).toBeInTheDocument();
  });
});

describe("Reset tab", () => {
  it("shows the announced reset, the estimate, the statistics and the history with its source", async () => {
    await renderApp();
    openTab("Reset");
    expect(await screen.findByText("Đã hẹn reset")).toBeInTheDocument();
    const latest = screen.getByText("Lần reset gần nhất").closest("article")!;
    expect(within(latest).getByText(/^\d+ (phút|giờ|ngày) trước$/)).toBeInTheDocument();
    expect(within(latest).getByText(/^\d{1,2}:\d{2} · (T[2-7]|CN) \d{2}\/\d{2} · Lượt để dành$/)).toBeInTheDocument();
    const message = within(latest).getByText(/^GPT-6 Sol and Luna are out\./).closest<HTMLElement>(".uc-reset-message")!;
    expect(within(message).getByText("@thsottiaux")).toBeInTheDocument();
    expect(within(message).getByRole("button", { name: "Mở bài trên X" })).toBeInTheDocument();
    expect(within(latest).getByText(/^\d+ (phút|giờ|ngày) trước$/).compareDocumentPosition(message) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(message).queryByText(/We are loading a banked reset/)).not.toBeInTheDocument();
    fireEvent.click(within(message).getByRole("button", { name: "Đọc tiếp" }));
    expect(within(message).getByText(/We are loading a banked reset into all accounts of our Plus, Pro and Business users\. Let's go!$/)).toBeInTheDocument();
    fireEvent.click(within(message).getByRole("button", { name: "Thu gọn" }));
    expect(within(message).queryByText(/We are loading a banked reset/)).not.toBeInTheDocument();
    expect(latest.compareDocumentPosition(screen.getByText("Đã hẹn reset")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText(/we’ll reset usage limits for all paid users/)).toBeInTheDocument();
    expect(screen.getByText("Khả năng sắp có reset (ứng dụng tự ước tính)")).toBeInTheDocument();
    for (const horizon of ["24 giờ tới", "3 ngày tới", "7 ngày tới"]) expect(screen.getByText(horizon)).toBeInTheDocument();
    expect(screen.getByText(/^Đã .* ngày chưa có reset\. Trước đây, \d+% số lần chờ ngắn hơn thế này\.$/)).toBeInTheDocument();
    expect(screen.getByText(/^Bình thường cứ .* ngày lại reset một lần, nên (khoảng .* là tới lượt|đã quá lượt từ .*)\.$/)).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Lịch reset 20 tuần qua" })).toBeInTheDocument();
    expect(screen.getByText("Hôm nay")).toBeInTheDocument();
    expect(screen.getByText("Thói quen thông báo")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /^Theo thứ: T2 \d+, T3 \d+/ })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /^Theo giờ \(giờ máy này\): 0h \d+, 4h \d+/ })).toBeInTheDocument();
    expect(screen.getByText("Khoảng lặng dài nhất")).toBeInTheDocument();
    expect(screen.getAllByText("Lượt để dành").length).toBeGreaterThan(0);
    expect(screen.getAllByRole("button", { name: "Mở bài trên X" }).length).toBeGreaterThan(0);
    expect(screen.getByText(/Dữ liệu từ Codex Resets/)).toBeInTheDocument();
  });
});

describe("reset notifications", () => {
  it("records what is there on the first run, then announces only new resets", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(notify).not.toHaveBeenCalled();
    const seen = JSON.parse(window.localStorage.getItem("quota-control.codex-resets-notified") ?? "{}") as { latest: string; scheduled: string };
    expect(seen.scheduled).toBe("2103637477760311522");

    const status = await api.publicFeed("codexResetStatus");
    const body = JSON.parse(status.body!) as { data: { scheduled_reset: { id: string } } };
    body.data.scheduled_reset.id = "999";
    act(() => useInsights.setState({ feeds: { ...useInsights.getState().feeds, codexResetStatus: { ...status, body: JSON.stringify(body) } } }));
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Codex hẹn reset", expect.stringContaining("reset usage limits"), { id: "resets.scheduled", group: "resets" });
  });

  it("announces a watch once while it is active", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    const status = await api.publicFeed("codexResetStatus");
    const body = JSON.parse(status.body!) as { data: { active_watch: unknown } };
    const observed = new Date(Date.now() - 60_000).toISOString();
    body.data.active_watch = {
      level: "strong",
      reset_chance_percent: 70,
      forecast_window: "24h",
      observed_at: observed,
      expires_at: new Date(Date.now() + 3_600_000).toISOString(),
      text: "Codex incident is mitigated, you know what comes next",
      source: { type: "x_post", author: "thsottiaux", url: "https://x.com/thsottiaux/status/1" },
    };
    const push = () => act(() => useInsights.setState({ feeds: { ...useInsights.getState().feeds, codexResetStatus: { ...status, body: JSON.stringify(body) } } }));
    push();
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Codex Resets: dấu hiệu mạnh sắp reset", expect.stringContaining("you know what comes next"), { id: "resets.watch", group: "resets" });
    body.data.active_watch = { ...(body.data.active_watch as object), text: "same watch, new wording" };
    push();
    expect(notify).toHaveBeenCalledOnce();
    expect(JSON.parse(window.localStorage.getItem("quota-control.codex-resets-notified") ?? "{}")).toMatchObject({ watch: `strong@${observed}` });
  });
});

describe("free reset on the Codex card", () => {
  async function pushScheduled(api: MockBackend, scheduled: Record<string, unknown>) {
    const status = await api.publicFeed("codexResetStatus");
    const body = JSON.parse(status.body!) as { data: { scheduled_reset: Record<string, unknown> } };
    body.data.scheduled_reset = { ...body.data.scheduled_reset, ...scheduled };
    act(() => useInsights.setState({ feeds: { ...useInsights.getState().feeds, codexResetStatus: { ...status, body: JSON.stringify(body) } } }));
  }

  it("counts down to an announced reset on Codex cards only and opens the Reset tab", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    await pushScheduled(api, { id: "42", scheduled_for: new Date(Date.now() + 3 * 3_600_000 - 30_000).toISOString(), text: "Resetting everyone soon" });

    const codex = screen.getByRole("region", { name: "Codex" });
    const row = within(codex).getByRole("button", { name: /^Reset free: sau 3 giờ\. Lúc .+ · GMT([+-]\d+(:\d{2})?)?$/ });
    expect(within(row).getByText("sau 3 giờ")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Claude · Công ty" })).queryByText("Reset free")).not.toBeInTheDocument();

    fireEvent.click(row);
    expect(screen.getByRole("tab", { name: "Reset" })).toHaveAttribute("aria-selected", "true");
  });

  it("counts 'tomorrow' down from 24 hours when the post gives no time", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    await pushScheduled(api, { id: "43", announced_at: new Date(Date.now() - 5 * 60_000).toISOString(), scheduled_for: null, text: "More resets coming tomorrow" });
    const row = within(screen.getByRole("region", { name: "Codex" })).getByRole("button", {
      name: /^Reset free: sau ~23 giờ 5\d phút\. Khoảng .+ · GMT([+-]\d+(:\d{2})?)?\. “Ngày mai” theo giờ Mỹ$/,
    });
    expect(within(row).getByText("“Ngày mai” theo giờ Mỹ")).toBeInTheDocument();
  });

  it("stays away while reset tracking is off", async () => {
    await renderApp({ showResetsTab: false, notifyCodexResets: false });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByText("Reset free")).not.toBeInTheDocument();
  });
});

describe("Claude resets", () => {
  const HOUR_MS = 3_600_000;
  const DAY_MS = 24 * HOUR_MS;

  function catalog(events: Record<string, unknown>[], overrides: Record<string, unknown> = {}): string {
    return JSON.stringify({ account: "ClaudeDevs", product: "Claude Code", events, live: true, detector: "fresh", provisionalEventIds: [], provisionalPolicyIds: [], ...overrides });
  }

  function reset(id: string, agoMs: number, overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
      id,
      date: new Date(Date.now() - agoMs).toISOString(),
      kind: "reset",
      scope: "all",
      note: `Reset number ${id} for all users.`,
      url: `https://x.com/ClaudeDevs/status/${id}`,
      verification: "curated",
      ...overrides,
    };
  }

  function banked(untilMs: number, overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return reset("77", 2 * DAY_MS, {
      resetType: "banked",
      usableUntil: new Date(Date.now() + untilMs).toISOString(),
      scope: "Pro, Max + Team",
      note: "Gave Pro, Max and Team users a banked reset.",
      ...overrides,
    });
  }

  async function push(api: MockBackend, body: string) {
    const snapshot = await api.publicFeed("claudeResets");
    act(() => useInsights.setState({ feeds: { ...useInsights.getState().feeds, claudeResets: { ...snapshot, body } } }));
  }

  const history = () => [reset("5", 9 * DAY_MS), reset("4", 20 * DAY_MS, { scope: "Max", account: "lydiahallie" }), reset("3", 40 * DAY_MS), reset("2", 70 * DAY_MS), reset("1", 120 * DAY_MS)];
  const change = () => reset("50", 30 * DAY_MS, { kind: "policy", scope: "paid plans", note: "Raised weekly limits 50%." });

  it("shows the Claude history in the Reset tab with the same cards, its banked reset, limit changes and the comparison", async () => {
    const api = await renderApp();
    await push(api, catalog([banked(10 * DAY_MS + HOUR_MS), ...history(), change()]));
    openTab("Reset");
    expect(screen.getByRole("radio", { name: "Codex" })).toHaveAttribute("aria-checked", "true");
    expect(await screen.findByText("Đã hẹn reset")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("radio", { name: "Claude" }));
    expect(useApp.getState().settings.resetsProvider).toBe("claude");
    expect(screen.queryByText("Đã hẹn reset")).not.toBeInTheDocument();
    const latest = screen.getByText("Lần reset gần nhất").closest("article")!;
    expect(within(latest).getByText("2 ngày trước")).toBeInTheDocument();
    expect(within(latest).getByText("@ClaudeDevs")).toBeInTheDocument();
    expect(within(latest).getByText(/· Lượt để dành · Gói Pro, Max, Team$/)).toBeInTheDocument();
    const message = within(latest).getByText("Gave Pro, Max and Team users a banked reset.").closest<HTMLElement>(".uc-reset-message")!;
    expect(within(message).getByText("@ClaudeDevs")).toBeInTheDocument();
    expect(within(message).getByRole("button", { name: "Mở bài trên X" })).toBeInTheDocument();
    expect(within(message).queryByRole("button", { name: "Đọc tiếp" })).not.toBeInTheDocument();

    const card = screen.getByText("Có lượt reset để dành").closest("article")!;
    expect(within(card).queryByText("Gave Pro, Max and Team users a banked reset.")).not.toBeInTheDocument();
    expect(within(card).queryByText("@ClaudeDevs")).not.toBeInTheDocument();
    expect(within(card).getByRole("button", { name: "Mở bài trên X" })).toBeInTheDocument();
    expect(within(card).getByText(/^Còn 10 ngày/)).toBeInTheDocument();
    expect(within(card).getByText(/^Dùng được đến \d{1,2}:\d{2} \d{2}\/\d{2}\/\d{4}$/)).toBeInTheDocument();
    expect(within(card).getByText(/^Gói .+ của bạn: có áp dụng$/)).toBeInTheDocument();
    expect(within(card).getByText(/Settings → Usage/)).toBeInTheDocument();
    expect(screen.queryByText("Chưa có thông báo reset mới")).not.toBeInTheDocument();

    expect(screen.getByText("Khả năng sắp có reset (ứng dụng tự ước tính)")).toBeInTheDocument();
    expect(screen.getByText(/không phải thông tin chính thức từ Anthropic/)).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Lịch reset 20 tuần qua" })).toBeInTheDocument();
    expect(screen.getByText("Reset cho mọi người")).toBeInTheDocument();
    expect(screen.getByText("Gói Max")).toBeInTheDocument();
    const other = screen.getByText("Reset number 4 for all users.").closest("article")!;
    expect(within(other).getByText("@lydiahallie")).toBeInTheDocument();
    expect(within(screen.getByText("Reset number 5 for all users.").closest("article")!).queryByText("@ClaudeDevs")).not.toBeInTheDocument();
    expect(screen.getByText("Thay đổi hạn mức")).toBeInTheDocument();
    expect(screen.getByText("Raised weekly limits 50%.")).toBeInTheDocument();
    expect(screen.getByText("Các gói trả phí")).toBeInTheDocument();
    const compare = screen.getByText("Claude so với Codex").closest("section")!;
    expect(within(compare).getByRole("row", { name: /^Số lần reset \d+ \d+$/ })).toBeInTheDocument();
    expect(within(compare).getByRole("img", { name: /^Số lần reset mỗi tháng: / })).toBeInTheDocument();
    expect(screen.getByText(/Dữ liệu từ claude-resets\.com/)).toBeInTheDocument();
    expect(screen.queryByText(/Dữ liệu từ Codex Resets/)).not.toBeInTheDocument();

    fireEvent.click(within(card).getByRole("button", { name: "Tôi đã dùng rồi" }));
    expect(useApp.getState().settings.usedBankedResets).toEqual(["77"]);
    expect(screen.queryByText("Có lượt reset để dành")).not.toBeInTheDocument();
    expect(screen.getByText("Bạn đã đánh dấu lượt này là đã dùng.")).toBeInTheDocument();
    expect(screen.getByText("Chưa có thông báo reset mới")).toBeInTheDocument();
    expect(within(screen.getByText("Lần reset gần nhất").closest("article")!).getByText("Gave Pro, Max and Team users a banked reset.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hoàn tác" }));
    expect(useApp.getState().settings.usedBankedResets).toEqual([]);
    expect(screen.getByText("Có lượt reset để dành")).toBeInTheDocument();
  });

  it("says when the site has not reviewed an entry, is behind, or only its published copy could be read", async () => {
    const api = await renderApp({ resetsProvider: "claude" });
    await push(api, catalog([reset("9", HOUR_MS, { verification: "provisional" }), ...history()], { detector: "stale" }));
    openTab("Reset");
    const latest = (await screen.findByText("Lần reset gần nhất")).closest("article")!;
    expect(within(latest).getByText(/· Mọi người dùng · Chưa kiểm chứng$/)).toBeInTheDocument();
    expect(within(latest).getByText(/chưa duyệt lại; tin có thể bị rút/)).toBeInTheDocument();
    expect(screen.getAllByText("Chưa kiểm chứng")).toHaveLength(1);
    expect(screen.getByText(/claude-resets\.com đang chậm cập nhật/)).toBeInTheDocument();
    await push(api, catalog(history(), { live: false, detector: null }));
    expect(screen.queryByText(/đang chậm cập nhật/)).not.toBeInTheDocument();
    expect(screen.getByText(/đang hiện bản đã công bố/)).toBeInTheDocument();
  });

  it("reminds Claude cards of a banked reset their plan has, and opens the Claude view", async () => {
    const api = await renderApp();
    await push(api, catalog([banked(3 * DAY_MS + 2 * HOUR_MS - 30_000), ...history()]));
    const claude = screen.getByRole("region", { name: "Claude · Công ty" });
    const row = within(claude).getByRole("button", { name: /^Lượt reset để dành: còn 3 ngày 2 giờ\. Dùng trước .+ · GMT([+-]\d+(:\d{2})?)?$/ });
    expect(within(screen.getByRole("region", { name: "Codex" })).queryByText("Lượt reset để dành")).not.toBeInTheDocument();
    fireEvent.click(row);
    expect(screen.getByRole("tab", { name: "Reset" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("radio", { name: "Claude" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByText("Có lượt reset để dành")).toBeInTheDocument();
  });

  it("leaves out cards whose plan the banked reset did not cover, and every card once it is marked as applied", async () => {
    const api = await renderApp();
    await push(api, catalog([banked(5 * DAY_MS, { scope: "Team" }), ...history()]));
    expect(screen.queryByText("Lượt reset để dành")).not.toBeInTheDocument();
    await push(api, catalog([banked(5 * DAY_MS), ...history()]));
    expect(screen.getAllByText("Lượt reset để dành").length).toBeGreaterThan(0);
    act(() => updateSettings({ usedBankedResets: ["77"] }));
    expect(screen.queryByText("Lượt reset để dành")).not.toBeInTheDocument();
  });

  it("stays away while Claude reset tracking is off", async () => {
    const api = await renderApp({ showResetsTab: false, notifyClaudeResets: false });
    await push(api, catalog([banked(2 * DAY_MS), ...history()]));
    expect(screen.queryByText("Lượt reset để dành")).not.toBeInTheDocument();
    expect(notify).not.toHaveBeenCalled();
  });

  it("keeps the card row while only the notification is on, without a tab to open", async () => {
    const api = await renderApp({ showResetsTab: false, notifyClaudeResets: true });
    await push(api, catalog([banked(5 * DAY_MS), ...history()]));
    const claude = screen.getByRole("region", { name: "Claude · Công ty" });
    expect(within(claude).getByRole("group", { name: /^Lượt reset để dành: còn / })).toBeInTheDocument();
    expect(within(claude).queryByRole("button", { name: /^Lượt reset để dành: / })).not.toBeInTheDocument();
  });

  it("keeps the card row while only the Reset tab is on, and sends nothing", async () => {
    const api = await renderApp({ showResetsTab: true, notifyClaudeResets: false });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    notify.mockClear();
    await push(api, catalog([banked(2 * DAY_MS), reset("6", 5 * 60_000), ...history()]));
    const claude = screen.getByRole("region", { name: "Claude · Công ty" });
    expect(within(claude).getByRole("button", { name: /^Lượt reset để dành: còn / })).toBeInTheDocument();
    expect(notify).not.toHaveBeenCalled();
    expect(window.localStorage.getItem("quota-control.claude-resets-notified") ?? "").not.toContain('"6"');
  });

  it("records what is there on the first run, then announces new resets and limit changes once", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    notify.mockClear();
    window.localStorage.removeItem("quota-control.claude-resets-notified");
    await push(api, catalog([reset("0", 10 * 60_000, { note: "Announced just before the app first looked." }), ...history(), change(), reset("49", 20 * 60_000, { kind: "policy", note: "Changed minutes ago." })]));
    expect(notify).not.toHaveBeenCalled();
    expect(JSON.parse(window.localStorage.getItem("quota-control.claude-resets-notified") ?? "{}")).toMatchObject({
      changes: expect.arrayContaining(["49", "50"]),
      resets: expect.arrayContaining(["0", "1", "5"]),
    });

    await push(api, catalog([reset("6", 5 * 60_000, { verification: "provisional", note: "Limits are reset for everyone." }), reset("old", 5 * DAY_MS), ...history(), change()]));
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Claude vừa reset (chưa kiểm chứng)", "Limits are reset for everyone.", { id: "claude-resets.reset.6", group: "resets" });

    await push(api, catalog([reset("6", 5 * 60_000, { note: "Limits are reset for everyone!" }), ...history(), change(), reset("51", 60_000, { kind: "policy", note: "Doubled the 5-hour limits." })]));
    expect(notify).toHaveBeenCalledTimes(2);
    expect(notify).toHaveBeenLastCalledWith("Claude đổi hạn mức", "Doubled the 5-hour limits.", { id: "claude-resets.change.51", group: "resets" });

    await push(api, catalog([reset("6", 5 * 60_000, { note: "Limits are reset for everyone!!" }), ...history(), change(), reset("51", 60_000, { kind: "policy", note: "Doubled the 5-hour limits, as said." })]));
    expect(notify).toHaveBeenCalledTimes(2);
  });

  it("forgets the oldest ids first, so a recent reset is never announced twice", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    notify.mockClear();
    window.localStorage.removeItem("quota-control.claude-resets-notified");
    const long = [reset("r0", 2 * HOUR_MS), ...Array.from({ length: 499 }, (_, index) => reset(`r${index + 1}`, 3 * DAY_MS + index * HOUR_MS))];
    await push(api, catalog(long));
    expect(notify).not.toHaveBeenCalled();

    const next = [reset("n1", 5 * 60_000, { note: "A new reset." }), ...long];
    await push(api, catalog(next));
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Claude vừa reset", "A new reset.", { id: "claude-resets.reset.n1", group: "resets" });
    const seen = JSON.parse(window.localStorage.getItem("quota-control.claude-resets-notified") ?? "{}") as { resets: string[] };
    expect(seen.resets).toHaveLength(500);
    expect(seen.resets).toEqual(expect.arrayContaining(["n1", "r0", "r1"]));
    expect(seen.resets).not.toContain("r499");

    await push(api, catalog([reset("n1", 5 * 60_000, { note: "A new reset, reworded." }), ...long]));
    expect(notify).toHaveBeenCalledOnce();
  });

  it("announces the deadline of a banked reset once when three days are left, unless it was marked as applied", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    notify.mockClear();
    window.localStorage.removeItem("quota-control.claude-resets-notified");
    await push(api, catalog([banked(4 * DAY_MS), ...history()]));
    expect(notify).not.toHaveBeenCalled();
    await push(api, catalog([banked(2 * DAY_MS + 3 * HOUR_MS - 30_000), ...history()]));
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Lượt reset để dành của Claude sắp hết hạn", expect.stringMatching(/^Còn 2 ngày 3 giờ, dùng trước .+ trong Settings → Usage\.$/), {
      id: "claude-resets.expiring.77",
      group: "resets",
    });
    await push(api, catalog([banked(2 * DAY_MS, { note: "Same reset, new wording." }), ...history()]));
    expect(notify).toHaveBeenCalledOnce();

    notify.mockClear();
    act(() => updateSettings({ usedBankedResets: ["78"] }));
    await push(api, catalog([banked(DAY_MS, { id: "78" }), ...history()]));
    expect(notify).not.toHaveBeenCalled();
    act(() => updateSettings({ usedBankedResets: [] }));
    await push(api, catalog([banked(DAY_MS, { id: "78", note: "Same reset, after the mark was taken back." }), ...history()]));
    expect(notify).not.toHaveBeenCalled();
  });

  it("announces a near deadline on the first run too", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    notify.mockClear();
    window.localStorage.removeItem("quota-control.claude-resets-notified");
    await push(api, catalog([banked(2 * DAY_MS), ...history()]));
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Lượt reset để dành của Claude sắp hết hạn", expect.any(String), { id: "claude-resets.expiring.77", group: "resets" });
  });

  it("keeps the deadline reminder to resets that can concern an account connected here", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    notify.mockClear();
    window.localStorage.removeItem("quota-control.claude-resets-notified");
    await push(api, catalog([banked(2 * DAY_MS, { scope: "Team" }), ...history()]));
    expect(notify).not.toHaveBeenCalled();
    openTab("Reset");
    fireEvent.click(screen.getByRole("radio", { name: "Claude" }));
    expect(screen.queryByText("Có lượt reset để dành")).not.toBeInTheDocument();
    expect(screen.getByText(/^Gói .+ của bạn: không áp dụng$/)).toBeInTheDocument();

    await push(api, catalog([banked(2 * DAY_MS, { id: "78", scope: "affected users" }), ...history()]));
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Lượt reset để dành của Claude sắp hết hạn", expect.any(String), { id: "claude-resets.expiring.78", group: "resets" });
    expect(screen.getByText("Có lượt reset để dành")).toBeInTheDocument();
  });

  it("sends one notification, not two, for a banked reset announced with under three days to use it", async () => {
    const api = await renderApp();
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    notify.mockClear();
    window.localStorage.removeItem("quota-control.claude-resets-notified");
    await push(api, catalog(history()));
    const quick = (note: string) => reset("90", 60_000, { resetType: "banked", usableUntil: new Date(Date.now() + 2 * DAY_MS).toISOString(), note });
    await push(api, catalog([quick("A banked reset, good for two days."), ...history()]));
    expect(notify).toHaveBeenCalledOnce();
    expect(notify).toHaveBeenCalledWith("Claude tặng lượt reset để dành", "A banked reset, good for two days.", { id: "claude-resets.reset.90", group: "resets" });
    await push(api, catalog([quick("A banked reset, good for two days!"), ...history()]));
    expect(notify).toHaveBeenCalledOnce();
  });

  it("scores the Codex estimate on its own history once that is long enough", async () => {
    const api = await renderApp();
    const ago = (days: number) => new Date(Date.now() - days * DAY_MS).toISOString();
    const post = (id: string, days: number) => ({ id, reset_type: "regular", announced_at: ago(days), text: `Codex reset ${id}.`, source: { type: "x_post", author: "thsottiaux", url: `https://x.com/thsottiaux/status/${id}` } });
    const slow = Array.from({ length: 8 }, (_, index) => post(`1${index}`, 300 - index * 25));
    const fast = Array.from({ length: 33 }, (_, index) => post(`2${index + 10}`, 100 - index * 3));
    const snapshot = await api.publicFeed("codexResets");
    const body = JSON.stringify({ data: [...slow, ...fast].reverse(), pagination: { has_more: false, next_cursor: null }, meta: { api_version: "v1" } });
    openTab("Reset");
    expect(await screen.findByText(/^Tính từ 24 lần reset đã ghi nhận/)).toBeInTheDocument();
    expect(screen.queryByText(/^Thử lại trên/)).not.toBeInTheDocument();
    act(() => useInsights.setState({ feeds: { ...useInsights.getState().feeds, codexResets: { ...snapshot, body } } }));
    expect(await screen.findByText(/^Thử lại trên \d+ ngày đã qua \(\d+ lần reset\): cách ước tính này đoán sát hơn mức trung bình \d+%\.$/)).toBeInTheDocument();
  });
});
