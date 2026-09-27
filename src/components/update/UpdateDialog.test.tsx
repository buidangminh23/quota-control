import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import type { UpdateStatus } from "@/lib/types";
import { useApp } from "@/state/store";
import { App } from "../../App";
import { closeDialog, confirmAction, isDialogOpen } from "../ui/dialog";
import { closeMenu } from "../ui/menu";

vi.mock("@/strip/useTaskbarStrip", () => ({ useTaskbarStrip: () => {} }));

const FIRST_CHECK = "2026-09-26T04:00:00.000Z";
const LATER_CHECK = "2026-09-26T10:00:00.000Z";

function status(patch: Partial<UpdateStatus>): UpdateStatus {
  return { supported: true, currentVersion: "0.1.0", phase: "idle", manual: false, downloaded: 0, ...patch };
}

function offer(checkedAt: string): UpdateStatus {
  return status({ phase: "available", available: { version: "0.2.0" }, checkedAt });
}

async function renderApp() {
  const api = new MockBackend();
  setBackend(api);
  render(<App />);
  await screen.findByText("Claude · Công ty");
  return api;
}

function dialog(title: string): HTMLElement {
  const found = screen.getByRole("alertdialog", { name: title });
  expect(found).toHaveAttribute("data-update-dialog");
  return found;
}

afterEach(async () => {
  closeMenu();
  closeDialog();
  cleanup();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() => useApp.setState({ screen: "dashboard", previousScreen: "dashboard", customizeProviderId: null, notice: null, dismissedUpdate: null }));
});

describe("update dialog", () => {
  it("stays quiet about background checks until one finds a release, then installs it and names the replaced version", async () => {
    const api = await renderApp();
    act(() => api.setUpdateStatus(status({ phase: "upToDate", checkedAt: FIRST_CHECK })));
    expect(screen.queryByRole("alertdialog")).toBeNull();

    act(() => api.setUpdateStatus(offer(FIRST_CHECK)));
    const found = dialog("Có phiên bản mới");
    expect(within(found).getByText("Quota Control 0.2.0 đã sẵn sàng. Cài xong, ứng dụng tự mở lại.")).toBeInTheDocument();
    expect(within(found).getByRole("button", { name: "Để sau" })).toHaveFocus();
    expect(isDialogOpen()).toBe(true);

    const install = vi.spyOn(api, "installUpdate");
    await act(async () => {
      fireEvent.click(within(found).getByRole("button", { name: "Cài bản mới" }));
    });
    expect(install).toHaveBeenCalledOnce();
    expect(await screen.findByRole("progressbar", { name: "Đang tải bản 0.2.0…" })).toBeInTheDocument();
    expect(await screen.findByRole("alertdialog", { name: "Đang cài bản 0.2.0…" })).toBeInTheDocument();

    const updated = await screen.findByRole("alertdialog", { name: "Đã cập nhật lên bản 0.2.0" });
    expect(within(updated).getByText("Quota Control đã được cập nhật từ bản 0.1.0 lên 0.2.0.")).toBeInTheDocument();
    const acknowledge = vi.spyOn(api, "acknowledgeUpdate");
    fireEvent.click(within(updated).getByRole("button", { name: "Đóng" }));
    expect(acknowledge).toHaveBeenCalledOnce();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(isDialogOpen()).toBe(false);
  });

  it("links the release notes and snoozes the offer until a later check finds it again", async () => {
    const api = await renderApp();
    const open = vi.spyOn(api, "openUrl").mockResolvedValue();
    act(() => api.setUpdateStatus(offer(FIRST_CHECK)));
    fireEvent.click(within(dialog("Có phiên bản mới")).getByRole("button", { name: "Xem thay đổi" }));
    expect(open).toHaveBeenCalledWith("https://github.com/buidangminh23/quota-control/releases/tag/v0.2.0");
    expect(dialog("Có phiên bản mới")).toBeInTheDocument();

    fireEvent.click(within(dialog("Có phiên bản mới")).getByRole("button", { name: "Để sau" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    act(() => api.setUpdateStatus(offer(FIRST_CHECK)));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    act(() => api.setUpdateStatus(offer(LATER_CHECK)));
    expect(dialog("Có phiên bản mới")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(useApp.getState().screen).toBe("dashboard");
  });

  it("reports a failed download and retries the install", async () => {
    const api = await renderApp();
    act(() =>
      api.setUpdateStatus(status({ phase: "failed", manual: true, available: { version: "0.2.0" }, failure: { stage: "download", reason: "signature" } })),
    );
    const failed = dialog("Không tải được bản mới");
    expect(within(failed).getByText("Tệp tải về không khớp chữ ký của Quota Control nên đã bị loại bỏ.")).toBeInTheDocument();
    const install = vi.spyOn(api, "installUpdate").mockResolvedValue();
    fireEvent.click(within(failed).getByRole("button", { name: "Thử lại" }));
    expect(install).toHaveBeenCalledOnce();
  });

  it("shows the result of a check the user started, and not again after the popup closes", async () => {
    const api = await renderApp();
    api.nextRelease = null;
    await act(async () => {
      await api.checkForUpdate();
    });
    expect(within(dialog("Đang dùng bản mới nhất")).getByText("Quota Control 0.1.0 là phiên bản mới nhất.")).toBeInTheDocument();
    await act(async () => {
      await api.hidePopup();
    });
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("hides a download until the install step, and waits for a confirmation dialog to close", async () => {
    const api = await renderApp();
    act(() => api.setUpdateStatus(status({ phase: "downloading", manual: true, available: { version: "0.2.0" }, downloaded: 1, total: 4 })));
    fireEvent.click(within(dialog("Đang tải bản 0.2.0…")).getByRole("button", { name: "Ẩn" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    act(() => api.setUpdateStatus(status({ phase: "downloading", manual: true, available: { version: "0.2.0" }, downloaded: 3, total: 4 })));
    expect(screen.queryByRole("alertdialog")).toBeNull();

    let confirmed: Promise<boolean> | undefined;
    act(() => {
      confirmed = confirmAction({ title: "Xoá?", message: "Thử", confirmLabel: "Xoá", cancelLabel: "Huỷ" });
    });
    act(() => api.setUpdateStatus(status({ phase: "installing", manual: true, available: { version: "0.2.0" } })));
    expect(screen.queryByRole("alertdialog", { name: "Đang cài bản 0.2.0…" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Huỷ" }));
    await expect(confirmed).resolves.toBe(false);
    expect(dialog("Đang cài bản 0.2.0…")).toBeInTheDocument();
  });
});
