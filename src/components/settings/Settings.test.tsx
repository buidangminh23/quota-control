import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
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

describe("bug reports", () => {
  it("opens an encoded draft with only allowlisted app information", async () => {
    const api = await openSettings();
    const open = vi.spyOn(api, "openUrl").mockResolvedValue();
    act(() => useApp.setState({ info: { name: "Quota Control", version: "0.3.9", platform: "windows", logFile: "C:/private/log.txt" } }));
    fireEvent.click(screen.getByRole("button", { name: "Viết báo cáo lỗi" }));
    const submit = screen.getByRole("button", { name: "Mở bản nháp trên GitHub" });
    expect(submit).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Tiêu đề lỗi"), { target: { value: "Lỗi & # dấu" } });
    fireEvent.change(screen.getByLabelText("Lỗi gặp phải và kết quả mong đợi"), { target: { value: "Không hiển thị" } });
    await act(async () => fireEvent.click(submit));
    const url = new URL(open.mock.calls[0]![0]);
    expect(url.origin + url.pathname).toBe("https://github.com/buidangminh23/quota-control/issues/new");
    expect(url.searchParams.get("title")).toBe("Lỗi & # dấu");
    expect(url.searchParams.get("body")).toContain("Version: 0.3.9");
    expect(url.searchParams.get("body")).not.toMatch(/private|log.txt|Công ty/);
    expect(screen.getByRole("status")).toHaveTextContent("Kiểm tra nội dung và bấm gửi trên GitHub");
  });

  it("retains the report on browser failure and permits copying", async () => {
    const api = await openSettings();
    vi.spyOn(api, "openUrl").mockRejectedValue(new Error("private backend details"));
    const copy = vi.spyOn(api, "copyText").mockResolvedValue();
    fireEvent.click(screen.getByRole("button", { name: "Viết báo cáo lỗi" }));
    fireEvent.change(screen.getByLabelText("Tiêu đề lỗi"), { target: { value: "Bug" } });
    fireEvent.change(screen.getByLabelText("Lỗi gặp phải và kết quả mong đợi"), { target: { value: "Details" } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Mở bản nháp trên GitHub" })));
    expect(screen.getByRole("status")).toHaveTextContent("Không thực hiện được");
    expect(screen.queryByText("private backend details")).not.toBeInTheDocument();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Sao chép báo cáo" })));
    expect(copy).toHaveBeenCalledWith(expect.stringContaining("Details"));
  });

  it("keeps long reports intact for copying instead of truncating them in the URL", async () => {
    const api = await openSettings();
    const open = vi.spyOn(api, "openUrl").mockResolvedValue();
    const copy = vi.spyOn(api, "copyText").mockResolvedValue();
    fireEvent.click(screen.getByRole("button", { name: "Viết báo cáo lỗi" }));
    fireEvent.change(screen.getByLabelText("Tiêu đề lỗi"), { target: { value: "Long" } });
    const details = "ỗ".repeat(3000);
    fireEvent.change(screen.getByLabelText("Lỗi gặp phải và kết quả mong đợi"), { target: { value: details } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Sao chép báo cáo" })));
    expect(copy).toHaveBeenCalledWith(expect.stringContaining(details));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Mở bản nháp trên GitHub" })));
    expect(new URL(open.mock.calls[0]![0]).searchParams.has("body")).toBe(false);
    expect(screen.getByRole("status")).toHaveTextContent("dán vào bản nháp");
  });
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

describe("app updates", () => {
  it("checks on request and turns automatic checks off", async () => {
    const api = await openSettings();
    api.nextRelease = null;
    expect(screen.getByRole("heading", { name: "Cập nhật ứng dụng" })).toBeInTheDocument();
    expect(screen.getByText("Phiên bản 0.1.0")).toBeInTheDocument();
    const automatic = screen.getByRole("switch", { name: "Tự động kiểm tra phiên bản mới" });
    expect(automatic).toHaveAttribute("aria-checked", "true");
    act(() => {
      fireEvent.click(automatic);
    });
    expect(useApp.getState().settings.automaticUpdateChecks).toBe(false);
    expect(automatic).toHaveAttribute("aria-checked", "false");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Kiểm tra ngay" }));
    });
    expect(await screen.findByText(/^Đang dùng bản mới nhất · kiểm tra lúc .+\.$/)).toBeInTheDocument();
  });

  it("offers the release a check found, in the section and in the update dialog", async () => {
    await openSettings();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Kiểm tra ngay" }));
    });
    expect(await screen.findByText("Đã có bản 0.2.0.")).toBeInTheDocument();
    const dialog = screen.getByRole("alertdialog", { name: "Có phiên bản mới" });
    expect(within(dialog).getByRole("button", { name: "Cài bản mới" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Cài bản mới" })).toHaveLength(2);
  });

  it("points a build that cannot update itself to the releases page", async () => {
    const api = await openSettings();
    act(() => api.setUpdateStatus({ supported: false, currentVersion: "0.1.0", phase: "idle", manual: false, downloaded: 0 }));
    expect(await screen.findByText(/không tự cập nhật được/)).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Tự động kiểm tra phiên bản mới" })).toBeNull();
    const open = vi.spyOn(api, "openUrl").mockResolvedValue();
    fireEvent.click(screen.getByRole("button", { name: "Mở trang phát hành" }));
    expect(open).toHaveBeenCalledWith("https://github.com/buidangminh23/quota-control/releases");
  });
});

describe("taskbar", () => {
  it("picks the tray icon's bars or the app icon alone", async () => {
    await openSettings();
    fireEvent.click(screen.getByRole("button", { name: "Hiển thị trên thanh tác vụ: Thanh" }));
    expect(screen.queryByRole("menuitemcheckbox", { name: "Số liệu" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Chỉ icon app" }));
    expect(useApp.getState().settings.showTaskbarStrip).toBe(false);
    expect(screen.getByText("Thanh tác vụ chỉ hiện biểu tượng Quota Control; bấm vào để mở bảng hạn mức.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hiển thị trên thanh tác vụ: Chỉ icon app" }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Thanh" }));
    expect(useApp.getState().settings).toMatchObject({ showTaskbarStrip: true, iconStyle: "bars" });
  });
});

describe("command line", () => {
  it("has no section, because usagectl installs itself", async () => {
    await openSettings();
    expect(screen.queryByRole("heading", { name: "Dòng lệnh" })).not.toBeInTheDocument();
    expect(screen.queryByText(/usagectl/)).not.toBeInTheDocument();
  });
});
