import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { localDay } from "@/lib/days";
import type { EngineState } from "@/lib/types";
import { MockBackend } from "@/lib/mockBackend";
import { updateSettings, useApp } from "@/state/store";
import { App } from "./App";
import { closeDialog } from "./components/ui/dialog";
import { closeMenu } from "./components/ui/menu";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

/** `settings` builds the stored settings document from the catalog's provider ids before the popup boots. */
async function renderApp({
  strict = false,
  settings,
  engine,
}: { strict?: boolean; settings?: (providerIds: string[]) => Record<string, unknown>; engine?: (state: EngineState) => void } = {}) {
  const api = new MockBackend();
  if (settings) await api.saveDocument("settings", settings((await api.catalog()).map((entry) => entry.provider.id)));
  if (engine) api.editEngineState(engine);
  setBackend(api);
  render(strict ? <StrictMode><App /></StrictMode> : <App />);
  await screen.findByText("Claude · Công ty");
  return api;
}

/** Use up the Công ty account's five-hour session, resetting `resetInMs` from now (negative: already past). */
function spendWorkSession(state: EngineState, resetInMs: number): void {
  const session = state.providers["claude@7c1e"]?.snapshot?.lines.find((line) => line.label === "Session");
  if (session?.type !== "progress") throw new Error("the fixture has no Session meter");
  session.used = 100;
  session.resetsAt = new Date(Date.now() + resetInMs).toISOString();
}

afterEach(async () => {
  closeMenu();
  closeDialog();
  cleanup();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() =>
    useApp.setState({ screen: "dashboard", previousScreen: "dashboard", tabMotion: null, customizeProviderId: null, notice: null, accountLogin: null, accountLoginError: null }),
  );
});

describe("popup", () => {
  it("keeps receiving live engine updates under StrictMode's double mount", async () => {
    const api = await renderApp({ strict: true });
    await act(async () => {
      void api.refresh("claude@7c1e");
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    expect(within(work).getByLabelText("Đang làm mới")).toBeInTheDocument();
  });

  it("opens on the Hạn mức tab with every connected account in Vietnamese by default", async () => {
    await renderApp();
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Token" })).toHaveAttribute("aria-selected", "false");
    expect(screen.getByRole("tabpanel", { name: "Hạn mức" })).toBeInTheDocument();
    expect(screen.getByText("Claude · Cá nhân")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Codex" })).toBeInTheDocument();
    expect(screen.queryByText("Claude · Trên máy này")).not.toBeInTheDocument();
    expect(screen.queryByText("Codex · Trên máy này")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Chỉ số tổng chi tiêu/ })).not.toBeInTheDocument();
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    expect(within(work).getByText("Còn 88%")).toBeInTheDocument();
    expect(within(work).getByText(/Đặt lại sau 3 giờ/)).toBeInTheDocument();
    expect(screen.getByText(/Cập nhật sau/)).toBeInTheDocument();
  });

  it("puts the exact restore time under each reset countdown, but not beside Exact Time", async () => {
    await renderApp();
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    const countdowns = within(work).getAllByText(/^Đặt lại sau /);
    expect(within(work).getAllByText(/^Hồi lại lúc \d{1,2}:\d{2} · /)).toHaveLength(countdowns.length);
    fireEvent.click(countdowns[0]!);
    expect(within(work).getAllByText(/^Đặt lại lúc /)).toHaveLength(countdowns.length);
    expect(within(work).queryByText(/^Hồi lại lúc /)).not.toBeInTheDocument();
  });

  it("shows a limit window whose reset has passed as reset before the next reading comes in", async () => {
    const api = await renderApp();
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    act(() => api.editEngineState((state) => spendWorkSession(state, -1_000)));
    expect(within(work).getByText("Còn 100%")).toBeInTheDocument();
    expect(within(work).getByText("Chưa bắt đầu")).toBeInTheDocument();
    expect(within(work).queryByText("Còn 0%")).not.toBeInTheDocument();
  });

  it("opens with a window that reset while the app was closed already shown as reset", async () => {
    await renderApp({ engine: (state) => spendWorkSession(state, -60_000) });
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    expect(within(work).getByText("Còn 100%")).toBeInTheDocument();
    expect(within(work).getByText("Chưa bắt đầu")).toBeInTheDocument();
  });

  it("rolls a spent window over the moment its reset arrives, with no new reading", async () => {
    const api = await renderApp();
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    act(() => api.editEngineState((state) => spendWorkSession(state, 120)));
    expect(within(work).getByText("Còn 0%")).toBeInTheDocument();
    await act(() => new Promise((resolve) => setTimeout(resolve, 250)));
    expect(within(work).getByText("Còn 100%")).toBeInTheDocument();
    expect(within(work).queryByText("Còn 0%")).not.toBeInTheDocument();
  });

  it("shows Codex's free limit resets as the last row of its card with the caret closed", async () => {
    await renderApp();
    const codex = screen.getByRole("region", { name: "Codex" });
    expect(within(codex).getByRole("button", { name: "Xem thêm" })).toHaveAttribute("aria-expanded", "false");
    const titles = within(codex).getAllByText(/^(Phiên|Tuần|Spark|Tín dụng|Lượt đặt lại hạn mức)$/).map((element) => element.textContent);
    expect(titles).toEqual(["Phiên", "Tuần", "Lượt đặt lại hạn mức"]);
  });

  it("uses a Codex limit reset only after the user confirms, one per press", async () => {
    await renderApp();
    const codex = screen.getByRole("region", { name: "Codex" });
    expect(within(codex).getByText("2 khả dụng")).toBeInTheDocument();
    fireEvent.click(within(codex).getByRole("button", { name: "Dùng 1 lượt" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/Dùng rồi không lấy lại được/)).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Hủy" }));
    expect(within(codex).getByText("2 khả dụng")).toBeInTheDocument();
    for (const left of ["1 khả dụng", "0 khả dụng"]) {
      fireEvent.click(within(codex).getByRole("button", { name: "Dùng 1 lượt" }));
      fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Dùng 1 lượt" }));
      expect(await within(codex).findByText(left)).toBeInTheDocument();
    }
    expect(screen.getByText("Đã dùng 1 lượt đặt lại. Hạn mức Codex đã hồi.")).toBeInTheDocument();
    expect(within(codex).queryByRole("button", { name: "Dùng 1 lượt" })).not.toBeInTheDocument();
  });

  it("shows the token total, then each source's trend and periods, on the Token tab and remembers the choice", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: "Token" }));
    expect(screen.getByRole("tab", { name: "Token" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel", { name: "Token" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Chỉ số tổng chi tiêu/ })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Hôm nay" })).toBeInTheDocument();
    for (const name of ["Claude", "Codex"]) {
      const source = screen.getByRole("region", { name });
      for (const row of ["Xu hướng sử dụng", "Hôm nay", "30 ngày qua"]) expect(within(source).getByText(row)).toBeInTheDocument();
      expect(within(source).queryByText("Hôm qua")).not.toBeInTheDocument();
      expect(within(source).queryByText("Token đầu vào")).not.toBeInTheDocument();
    }
    expect(screen.queryByRole("region", { name: "Claude · Công ty" })).not.toBeInTheDocument();
    expect(screen.queryByText(/Trên máy này/)).not.toBeInTheDocument();
    expect(useApp.getState().settings.dashboardTab).toBe("tokens");
    expect(useApp.getState().tabMotion).toBe("forward");
  });

  it("offers only a refresh on a token source, since hiding it would empty the Token tab", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: "Token" }));
    fireEvent.contextMenu(screen.getByRole("region", { name: "Claude" }).querySelector("header")!);
    expect(screen.getByRole("menuitem", { name: "Làm mới Claude" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Ẩn Claude" })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Tùy chỉnh…" })).not.toBeInTheDocument();
  });

  it("moves between tabs with the arrow keys and Ctrl+Tab", async () => {
    await renderApp({ settings: () => ({ showBenchmarkTab: false, showResetsTab: false }) });
    const limits = screen.getByRole("tab", { name: "Hạn mức" });
    act(() => limits.focus());
    fireEvent.keyDown(limits, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "Token" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Token" })).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("tab", { name: "Token" }), { key: "Home" });
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
    expect(useApp.getState().tabMotion).toBe("back");
    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
    expect(screen.getByRole("tab", { name: "Token" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Token" })).toHaveFocus();
    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true, shiftKey: true });
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(screen.getByRole("tab", { name: "Hạn mức" }), { key: "End" });
    expect(screen.getByRole("tab", { name: "Bảng giá" })).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
  });

  it("walks the Token views: context windows, a past day, the charts and the projects", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: "Token" }));
    expect(screen.getByRole("radio", { name: "Tổng quan" })).toHaveAttribute("aria-checked", "true");
    expect(await screen.findByText("478,3 N / 1 Tr")).toBeInTheDocument();
    expect(screen.getByText("(48%)")).toBeInTheDocument();
    expect(screen.getByText("(85%)")).toHaveClass("is-warning");
    expect(screen.getByText("Theo năm")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("radio", { name: "Lịch sử" }));
    const today = localDay();
    const title = `Tháng ${Number(today.slice(5, 7))}, ${today.slice(0, 4)}`;
    expect(await screen.findByRole("heading", { name: title })).toBeInTheDocument();
    expect(await screen.findByText("Cả tháng")).toBeInTheDocument();
    fireEvent.click(document.querySelector<HTMLButtonElement>(".uc-history-day")!);
    for (const heading of ["Theo nguồn", "Theo model", "Theo project"]) expect(await screen.findByText(heading)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: `Quay lại ${title}` }));
    expect(await screen.findByText("Cả tháng")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("radio", { name: "Biểu đồ" }));
    expect(await screen.findByRole("heading", { name: "30 ngày gần nhất" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "Model" }));
    expect(await screen.findByRole("heading", { name: "Xếp hạng model" })).toBeInTheDocument();
    expect((await screen.findAllByText("gpt-5.6-sol")).length).toBeGreaterThan(0);

    fireEvent.click(screen.getAllByRole("radio", { name: "Project" })[0]!);
    expect(await screen.findByText("PCC4SH")).toBeInTheDocument();
    expect(screen.getByText("Không rõ project")).toBeInTheDocument();
    expect(useApp.getState().settings.tokenView).toBe("projects");
  });

  it("shows the official price tables in đồng, then OpenAI's and in dollars", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: "Bảng giá" }));
    const [base, caching] = screen.getAllByRole("table");
    expect(within(base!).getByRole("columnheader", { name: "Đầu vào" })).toBeInTheDocument();
    expect(within(caching!).getByRole("columnheader", { name: "Ghi 1 giờ" })).toBeInTheDocument();
    const opus = within(base!).getByRole("rowheader", { name: "Claude Opus 5.5" }).closest("tr")!;
    expect(within(opus).getByText("104.680")).toBeInTheDocument();
    expect(screen.getByText(/Vietcombank: 26\.170/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "$" }));
    const dollars = within(screen.getAllByRole("table")[0]!).getByRole("rowheader", { name: "Claude Opus 5.5" }).closest("tr")!;
    expect(within(dollars).getByText("20")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "OpenAI" }));
    expect(screen.getAllByText("Ngữ cảnh dài").length).toBeGreaterThan(0);
    expect(screen.getByRole("radio", { name: "Flex" })).toBeInTheDocument();
    expect(useApp.getState().settings).toMatchObject({ priceProvider: "openai", priceCurrency: "usd" });
  });

  it("drops only the Token tab and shows the limits while it is turned off", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: "Token" }));
    act(() => updateSettings({ showTotalSpend: false }));
    expect(screen.queryByRole("tab", { name: "Token" })).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Bảng giá" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Claude · Công ty" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Chỉ số tổng chi tiêu/ })).not.toBeInTheDocument();
    act(() => updateSettings({ showTotalSpend: true }));
    expect(screen.getByRole("tab", { name: "Token" })).toHaveAttribute("aria-selected", "true");
  });

  it("brings back the Token tab and its sources when an older version hid the token cards", async () => {
    const api = await renderApp({ settings: (ids) => ({ enabledProviders: ids.filter((id) => !id.endsWith("-local")) }) });
    expect(screen.getByRole("tab", { name: "Hạn mức" })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("tab", { name: "Token" }));
    expect(screen.getByRole("button", { name: /Chỉ số tổng chi tiêu/ })).toBeInTheDocument();
    expect(useApp.getState().enabledProviders).toEqual(expect.arrayContaining(["claude-local", "codex-local"]));
    await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
    const stored = await api.loadDocument<{ enabledProviders: string[] }>("settings");
    expect(stored?.enabledProviders).toEqual(expect.arrayContaining(["claude-local", "codex-local", "claude@7c1e"]));
    expect(screen.queryByText("Claude · Trên máy này")).not.toBeInTheDocument();
  });

  it("turns the token sources back on together with the Token tab setting", async () => {
    const api = await renderApp({ settings: (ids) => ({ showTotalSpend: false, enabledProviders: ids.filter((id) => !id.endsWith("-local")) }) });
    expect(screen.queryByRole("tab", { name: "Token" })).not.toBeInTheDocument();
    expect(useApp.getState().enabledProviders).not.toContain("claude-local");
    act(() => updateSettings({ showTotalSpend: true }));
    expect(screen.getByRole("tab", { name: "Token" })).toBeInTheDocument();
    await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
    const stored = await api.loadDocument<{ enabledProviders: string[]; showTotalSpend: boolean }>("settings");
    expect(stored?.showTotalSpend).toBe(true);
    expect(stored?.enabledProviders).toEqual(expect.arrayContaining(["claude-local", "codex-local"]));
  });

  it("checks for a new version from the Options menu, where screenshot sharing used to be", async () => {
    const api = await renderApp();
    expect(screen.queryByRole("button", { name: /ảnh chụp/ })).not.toBeInTheDocument();
    const check = vi.spyOn(api, "checkForUpdate");
    fireEvent.click(screen.getByRole("button", { name: "Tùy chọn" }));
    expect(screen.queryByRole("menuitem", { name: /ảnh chụp/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitem", { name: "Kiểm tra phiên bản mới…" }));
    expect(check).toHaveBeenCalledOnce();
    expect(await screen.findByText("Đang kiểm tra phiên bản mới…")).toBeInTheDocument();
    expect(await screen.findByText("Có phiên bản mới")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Tùy chọn" }));
    expect(screen.getByRole("menuitem", { name: "Cài bản mới 0.2.0…" })).toBeEnabled();
  });

  it("draws the Claude and Codex marks in their brand colors", async () => {
    await renderApp();
    const mark = (name: string) => screen.getByRole("region", { name }).querySelector("svg.uc-mark");
    expect(mark("Claude · Công ty")).toHaveAttribute("fill", "#DE7356");
    expect(mark("Codex")).toHaveAttribute("fill", "#10A37F");
  });

  it("switches every screen to English", async () => {
    await renderApp();
    act(() => updateSettings({ language: "en" }));
    expect(await screen.findByRole("tab", { name: "Limits" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Tokens" })).toBeInTheDocument();
    expect(screen.getAllByText("88% left").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Options" }));
    expect(screen.getByRole("menuitem", { name: /Settings/ })).toBeInTheDocument();
    act(() => updateSettings({ language: "vi" }));
  });

  it("opens Customize with Enter and Settings with Ctrl+comma, and goes back with Escape", async () => {
    await renderApp();
    fireEvent.keyDown(window, { key: "Enter" });
    expect(await screen.findByRole("heading", { name: "Tùy chỉnh" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(await screen.findByRole("heading", { name: "Cài đặt" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hiển thị trên thanh tác vụ: Thanh" })).toBeInTheDocument();
  });

  it("shows an account's email on its own line under the name and plan", async () => {
    await renderApp();
    const catalog = structuredClone(useApp.getState().catalog);
    const work = catalog.find((entry) => entry.provider.id === "claude@7c1e");
    if (!work) throw new Error("fixture card missing");
    work.provider.displayName = "Claude · someone@example.com";
    act(() => useApp.setState({ catalog }));
    const card = await screen.findByRole("region", { name: "Claude · someone@example.com" });
    const email = within(card).getByText("someone@example.com");
    expect(email).toHaveClass("uc-section-account");
    expect(within(card).getByText("Claude")).toHaveClass("uc-section-name");
    expect(within(card).queryByText("Claude · someone@example.com")).not.toBeInTheDocument();
    expect(screen.getByText("Claude · Cá nhân")).toHaveClass("uc-section-name");
  });

  it("shows how long each plan's paid period has left in the card header's right corner", async () => {
    await renderApp();
    const codex = screen.getByRole("region", { name: "Codex" });
    const stated = within(codex).getByRole("group", { name: /^còn 20 ngày\. tới .+\. Gói hết kỳ lúc .+ · GMT/ });
    expect(stated).not.toHaveClass("is-soon");
    expect(stated.closest("header")).toHaveClass("has-term");
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    const estimate = within(work).getByRole("group", { name: /^còn ~\d+ ngày\. tới ~.+\. Khoảng .+, ước tính\./ });
    expect(within(estimate).getByText(/^tới ~/)).toHaveClass("uc-plan-term-day");
    const personal = screen.getByRole("region", { name: "Claude · Cá nhân" });
    expect(within(personal).queryByRole("group", { name: /^còn / })).not.toBeInTheDocument();
    expect(personal.querySelector("header")).not.toHaveClass("has-term");
  });

  it("lists connected accounts with a Google sign-in and in-app chat actions", async () => {
    await renderApp();
    act(() => useApp.setState({ screen: "accounts" }));
    expect(await screen.findByRole("heading", { name: "Tài khoản" })).toBeInTheDocument();
    expect(screen.getByText("Tự động từ Codex CLI trên máy này")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Xóa (?!OpenRouter)/ })).toHaveLength(2);
    expect(screen.getByRole("button", { name: /^Xóa OpenRouter/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Xóa Codex" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Đăng nhập bằng Google" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Phiên ChatGPT mới/ })).toBeInTheDocument();
  });

  it("tells how to copy a session cookie or a whole Cookie header for web-session providers", async () => {
    await renderApp();
    act(() => useApp.setState({ screen: "accounts" }));
    await screen.findByRole("heading", { name: "Tài khoản" });
    const pick = (name: string) => {
      fireEvent.change(screen.getByRole("searchbox", { name: "Tìm nhà cung cấp…" }), { target: { value: name } });
      fireEvent.click(screen.getByRole("option", { name }));
    };
    pick("LongCat");
    expect(screen.getByLabelText("Dán Cookie header")).toBeInTheDocument();
    expect(screen.getByText(/chép nguyên giá trị Cookie trong Request Headers/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Mở trang LongCat" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Lấy API key" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Đổi nhà cung cấp" }));
    pick("Perplexity");
    expect(screen.getByLabelText("Dán cookie phiên (__Secure-authjs.session-token)")).toBeInTheDocument();
    expect(screen.getByText(/Application → Cookies/)).toBeInTheDocument();
    expect(screen.queryByText(/Request Headers/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Đổi nhà cung cấp" }));
    pick("DeepSeek");
    expect(screen.getByLabelText("Dán API key")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Lấy API key" })).toBeInTheDocument();
    expect(screen.queryByText(/F12/)).not.toBeInTheDocument();
  });

  it("keeps waiting for a browser sign-in while the popup hides and announces the new account", async () => {
    const api = await renderApp();
    api.loginDelayMs = null;
    act(() => useApp.setState({ screen: "accounts" }));
    fireEvent.click(await screen.findByRole("button", { name: "Đăng nhập bằng Google" }));
    expect(await screen.findByText("Đang chờ bạn đăng nhập Claude trong Google Chrome…")).toBeInTheDocument();
    await act(() => api.hidePopup());
    expect(useApp.getState().screen).toBe("dashboard");
    expect(screen.getByText("Đang chờ bạn đăng nhập Claude trong Google Chrome…")).toBeInTheDocument();
    const login = useApp.getState().accountLogin;
    if (login?.phase !== "waiting") throw new Error("the sign-in should be waiting");
    await act(async () => {
      api.finishLogin(login.flowId);
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    expect(useApp.getState().accountLogin).toBeNull();
    expect(screen.getByText("Đã kết nối Claude")).toBeInTheDocument();
    expect(useApp.getState().accounts.filter((account) => account.credentialMode === "managed_oauth")).toHaveLength(2);
  });

  it("explains a failed sign-in and lets a waiting one be cancelled", async () => {
    const api = await renderApp();
    api.loginDelayMs = null;
    act(() => useApp.setState({ screen: "accounts" }));
    fireEvent.click(await screen.findByRole("radio", { name: "Codex" }));
    fireEvent.click(screen.getByRole("button", { name: "Đăng nhập bằng Google" }));
    await screen.findByText("Đang chờ bạn đăng nhập Codex trong Google Chrome…");
    const login = useApp.getState().accountLogin;
    if (login?.phase !== "waiting") throw new Error("the sign-in should be waiting");
    act(() => api.finishLogin(login.flowId, "The browser login was not authorized."));
    expect(await screen.findByText("Chưa kết nối được Codex: Đăng nhập trong trình duyệt chưa được cho phép.")).toBeInTheDocument();
    expect(screen.getByText("Chưa kết nối được Codex")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Đăng nhập bằng Google" }));
    await screen.findByText("Đang chờ bạn đăng nhập Codex trong Google Chrome…");
    fireEvent.click(screen.getByRole("button", { name: "Hủy" }));
    expect(await screen.findByRole("button", { name: "Đăng nhập bằng Google" })).toBeInTheDocument();
    expect(useApp.getState().accountLogin).toBeNull();
  });

  it("keeps a way to add an account after the onboarding hint is closed", async () => {
    const api = await renderApp();
    for (const account of await api.listAccounts()) await act(() => api.removeAccount(account.id));
    await act(async () => {
      updateSettings({ accountsHintDismissed: true });
      useApp.setState({ accounts: [] });
    });
    fireEvent.click(await screen.findByRole("button", { name: "Thêm tài khoản" }));
    expect(await screen.findByRole("heading", { name: "Tài khoản" })).toBeInTheDocument();
    act(() => updateSettings({ accountsHintDismissed: false }));
  });
});
