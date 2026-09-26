import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import { useApp } from "@/state/store";
import { App } from "../../App";
import { closeDialog } from "../ui/dialog";
import { closeMenu } from "../ui/menu";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

async function openSettings() {
  const api = new MockBackend();
  setBackend(api);
  render(<App />);
  await screen.findByText("Claude · Công ty");
  act(() => useApp.setState({ screen: "settings" }));
  await screen.findByRole("heading", { name: "Cài đặt" });
  return api;
}

afterEach(async () => {
  closeMenu();
  closeDialog();
  cleanup();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() => useApp.setState({ screen: "dashboard", previousScreen: "dashboard", customizeProviderId: null, notice: null }));
});

describe("global shortcut", () => {
  it("records a combo, keeps Escape inside the recorder and clears with the ✕", async () => {
    const api = await openSettings();
    const pause = vi.spyOn(api, "pauseGlobalShortcut");
    const field = await screen.findByRole("button", { name: "Phím tắt toàn cục: Ghi phím tắt" });

    fireEvent.click(field);
    expect(pause).toHaveBeenLastCalledWith(true);
    expect(screen.getByRole("button", { name: "Phím tắt toàn cục: Nhấn tổ hợp phím…" })).toBeInTheDocument();
    fireEvent.keyDown(field, { key: "u", code: "KeyU" });
    expect(screen.getByText("Hãy nhấn kèm Ctrl, Alt, Shift hoặc Super.")).toBeInTheDocument();
    fireEvent.keyDown(field, { key: "Escape", code: "Escape" });
    expect(screen.getByRole("heading", { name: "Cài đặt" })).toBeInTheDocument();
    expect(pause).toHaveBeenLastCalledWith(false);

    fireEvent.click(field);
    await act(async () => {
      fireEvent.keyDown(field, { key: "u", code: "KeyU", ctrlKey: true, altKey: true });
    });
    expect(await screen.findByRole("button", { name: "Phím tắt toàn cục: Ctrl+Alt+U" })).toBeInTheDocument();
    expect(await api.globalShortcut()).toBe("Ctrl+Alt+KeyU");
    expect(pause).toHaveBeenLastCalledWith(false);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Xóa phím tắt" }));
    });
    expect(await screen.findByRole("button", { name: "Phím tắt toàn cục: Ghi phím tắt" })).toBeInTheDocument();
    expect(await api.globalShortcut()).toBeNull();
  });

  it("shows why a combo was refused and keeps the previous one", async () => {
    const api = await openSettings();
    await api.setGlobalShortcut("Ctrl+Alt+KeyU");
    vi.spyOn(api, "setGlobalShortcut").mockRejectedValueOnce("taken");
    act(() => useApp.setState({ screen: "dashboard" }));
    act(() => useApp.setState({ screen: "settings" }));
    const field = await screen.findByRole("button", { name: "Phím tắt toàn cục: Ctrl+Alt+U" });
    fireEvent.click(field);
    await act(async () => {
      fireEvent.keyDown(field, { key: "k", code: "KeyK", ctrlKey: true });
    });
    expect(await screen.findByText("Không đặt được phím tắt này. Có thể một ứng dụng khác đang dùng nó.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Phím tắt toàn cục: Ctrl+Alt+U" })).toBeInTheDocument();
  });
});

describe("command line", () => {
  it("installs and removes the terminal helper and names the local API", async () => {
    await openSettings();
    expect(screen.getByRole("heading", { name: "Dòng lệnh" })).toBeInTheDocument();
    expect(screen.getByText(/http:\/\/127\.0\.0\.1:6736\/v1\/limits/)).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Cài đặt" }));
    });
    expect(await screen.findByText("Đã cài. Mở terminal mới để dùng lệnh.")).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Gỡ" }));
    });
    expect(await screen.findByRole("button", { name: "Cài đặt" })).toBeInTheDocument();
  });
});
