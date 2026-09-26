import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { updateSettings, useApp } from "@/state/store";
import { App } from "./App";
import { closeDialog } from "./components/ui/dialog";
import { closeMenu } from "./components/ui/menu";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

/** `settings` builds the stored settings document from the catalog's provider ids before the popup boots. */
async function renderApp({ strict = false, settings }: { strict?: boolean; settings?: (providerIds: string[]) => Record<string, unknown> } = {}) {
  const api = new MockBackend();
  if (settings) await api.saveDocument("settings", settings((await api.catalog()).map((entry) => entry.provider.id)));
  setBackend(api);
  render(strict ? <StrictMode><App /></StrictMode> : <App />);
  await screen.findByText("Claude · Công ty");
  return api;
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

  it("shows the token total, then each source's trend and periods, on the Token tab and remembers the choice", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: "Token" }));
    expect(screen.getByRole("tab", { name: "Token" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel", { name: "Token" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Chỉ số tổng chi tiêu/ })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Hôm nay" })).toBeInTheDocument();
    for (const name of ["Claude", "Codex"]) {
      const source = screen.getByRole("region", { name });
      for (const row of ["Xu hướng sử dụng", "Hôm nay", "Hôm qua", "30 ngày qua"]) expect(within(source).getByText(row)).toBeInTheDocument();
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
    await renderApp();
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
  });

  it("drops the tab bar and shows the limits while the Token tab is turned off", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: "Token" }));
    act(() => updateSettings({ showTotalSpend: false }));
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Claude · Công ty" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Chỉ số tổng chi tiêu/ })).not.toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
    expect(screen.getByRole("region", { name: "Claude · Công ty" })).toBeInTheDocument();
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
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
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
    expect(screen.getByRole("switch", { name: "Hiện số liệu trên thanh tác vụ" })).toBeChecked();
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

  it("lists connected accounts with a Google sign-in and in-app chat actions", async () => {
    await renderApp();
    act(() => useApp.setState({ screen: "accounts" }));
    expect(await screen.findByRole("heading", { name: "Tài khoản" })).toBeInTheDocument();
    expect(screen.getByText("Tự động từ Codex CLI trên máy này")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Xóa / })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "Xóa Codex" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Đăng nhập bằng Google" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Phiên ChatGPT mới/ })).toBeInTheDocument();
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
