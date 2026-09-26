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
    expect(screen.getByText(/we’ll reset usage limits for all paid users/)).toBeInTheDocument();
    expect(screen.getByText("Khả năng có reset (ước tính của ứng dụng)")).toBeInTheDocument();
    for (const horizon of ["24 giờ", "3 ngày", "7 ngày"]) expect(screen.getByText(horizon)).toBeInTheDocument();
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
    expect(notify).toHaveBeenCalledWith("Codex hẹn reset", expect.stringContaining("reset usage limits"));
  });
});
