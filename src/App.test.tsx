import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { updateSettings, useApp } from "@/state/store";
import { App } from "./App";
import { closeDialog } from "./components/ui/dialog";
import { closeMenu } from "./components/ui/menu";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

async function renderApp(strict = false) {
  const api = new MockBackend();
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
  act(() => useApp.setState({ screen: "dashboard", previousScreen: "dashboard", customizeProviderId: null, notice: null }));
});

describe("popup", () => {
  it("keeps receiving live engine updates under StrictMode's double mount", async () => {
    const api = await renderApp(true);
    await act(async () => {
      void api.refresh("claude@7c1e");
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    expect(within(work).getByLabelText("Đang làm mới")).toBeInTheDocument();
  });

  it("shows every connected account and this computer's usage in Vietnamese by default", async () => {
    await renderApp();
    expect(screen.getByText("Claude · Cá nhân")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Codex" })).toBeInTheDocument();
    expect(screen.getByText("Claude · Trên máy này")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Chỉ số tổng chi tiêu/ })).toBeInTheDocument();
    const work = screen.getByRole("region", { name: "Claude · Công ty" });
    expect(within(work).getByText("Còn 88%")).toBeInTheDocument();
    expect(within(work).getByText(/Đặt lại sau 3 giờ/)).toBeInTheDocument();
    expect(screen.getByText(/Cập nhật sau/)).toBeInTheDocument();
  });

  it("switches every screen to English", async () => {
    await renderApp();
    act(() => updateSettings({ language: "en" }));
    expect(await screen.findByText("Claude · This Computer")).toBeInTheDocument();
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

  it("lists connected accounts with sign-in and in-app chat actions", async () => {
    await renderApp();
    act(() => useApp.setState({ screen: "accounts" }));
    expect(await screen.findByRole("heading", { name: "Tài khoản" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Xóa / })).toHaveLength(3);
    expect(screen.getByRole("button", { name: "Đăng nhập Claude…" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Phiên ChatGPT mới/ })).toBeInTheDocument();
  });
});
