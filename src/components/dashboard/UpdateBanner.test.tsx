import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { MockBackend } from "@/lib/mockBackend";
import type { UpdateStatus } from "@/lib/types";
import { useApp } from "@/state/store";
import { App } from "../../App";
import { closeDialog } from "../ui/dialog";
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

function updateCard(title: string): HTMLElement {
  const card = screen.getByText(title).closest<HTMLElement>("[data-update-card]");
  if (!card) throw new Error(`No update card titled ${title}`);
  return card;
}

afterEach(async () => {
  closeMenu();
  closeDialog();
  cleanup();
  await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  act(() => useApp.setState({ screen: "dashboard", previousScreen: "dashboard", customizeProviderId: null, notice: null, dismissedUpdate: null }));
});

describe("update card", () => {
  it("stays quiet about background checks until one finds a release, then installs it", async () => {
    const api = await renderApp();
    act(() => api.setUpdateStatus(status({ phase: "upToDate", checkedAt: FIRST_CHECK })));
    expect(screen.queryByText("Đang dùng bản mới nhất")).toBeNull();

    act(() => api.setUpdateStatus(offer(FIRST_CHECK)));
    const card = updateCard("Có phiên bản mới");
    expect(within(card).getByText("Quota Control 0.2.0 đã sẵn sàng. Cài xong, ứng dụng tự mở lại.")).toBeInTheDocument();

    const install = vi.spyOn(api, "installUpdate");
    await act(async () => {
      fireEvent.click(within(card).getByRole("button", { name: "Cài bản mới" }));
    });
    expect(install).toHaveBeenCalledOnce();
    expect(await screen.findByRole("progressbar", { name: "Đang tải bản 0.2.0…" })).toBeInTheDocument();
    expect(await screen.findByText("Đang cài bản 0.2.0…")).toBeInTheDocument();
  });

  it("links the release notes and snoozes the offer until a later check finds it again", async () => {
    const api = await renderApp();
    const open = vi.spyOn(api, "openUrl").mockResolvedValue();
    act(() => api.setUpdateStatus(offer(FIRST_CHECK)));
    const card = updateCard("Có phiên bản mới");
    fireEvent.click(within(card).getByRole("button", { name: "Xem thay đổi" }));
    expect(open).toHaveBeenCalledWith("https://github.com/buidangminh23/quota-control/releases/tag/v0.2.0");

    fireEvent.click(within(card).getByRole("button", { name: "Đóng" }));
    expect(screen.queryByText("Có phiên bản mới")).toBeNull();
    act(() => api.setUpdateStatus(offer(FIRST_CHECK)));
    expect(screen.queryByText("Có phiên bản mới")).toBeNull();
    act(() => api.setUpdateStatus(offer(LATER_CHECK)));
    expect(screen.getByText("Có phiên bản mới")).toBeInTheDocument();
  });

  it("reports a failed download and retries the install", async () => {
    const api = await renderApp();
    act(() =>
      api.setUpdateStatus(status({ phase: "failed", manual: true, available: { version: "0.2.0" }, failure: { stage: "download", reason: "signature" } })),
    );
    const card = updateCard("Không tải được bản mới");
    expect(within(card).getByText("Tệp tải về không khớp chữ ký của Quota Control nên đã bị loại bỏ.")).toBeInTheDocument();
    const install = vi.spyOn(api, "installUpdate").mockResolvedValue();
    fireEvent.click(within(card).getByRole("button", { name: "Thử lại" }));
    expect(install).toHaveBeenCalledOnce();
  });

  it("shows the result of a check the user started, and not again after the popup closes", async () => {
    const api = await renderApp();
    api.nextRelease = null;
    await act(async () => {
      await api.checkForUpdate();
    });
    expect(within(updateCard("Đang dùng bản mới nhất")).getByText("Quota Control 0.1.0 là phiên bản mới nhất.")).toBeInTheDocument();
    await act(async () => {
      await api.hidePopup();
    });
    expect(screen.queryByText("Đang dùng bản mới nhất")).toBeNull();
  });
});
