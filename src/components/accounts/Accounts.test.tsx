import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { setBackend } from "@/lib/backend";
import { fixtureAccounts } from "@/lib/fixtures";
import { MockBackend } from "@/lib/mockBackend";
import type { ConnectedAccount } from "@/lib/types";
import { startAccountLogin, useApp } from "@/state/store";
import { Accounts } from "./Accounts";

vi.mock("@/state/store", async (original) => ({
  ...await original<typeof import("@/state/store")>(),
  startAccountLogin: vi.fn(),
}));

const initial = useApp.getState();
const pending: ConnectedAccount = {
  id: "codex@synced",
  provider: "codex",
  label: "Office",
  requiresLogin: true,
};

beforeEach(() => {
  useApp.setState(initial, true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  useApp.setState(initial, true);
});

it("lists synced accounts as needing local login and starts the existing provider flow", async () => {
  const api = new MockBackend();
  setBackend(api);
  const connected = fixtureAccounts()[0]!;
  vi.spyOn(api, "listAccounts").mockResolvedValue([connected, pending]);
  vi.spyOn(api, "listServices").mockResolvedValue([]);
  const engine = await api.engineState();
  const active = engine.providers[connected.id]!;
  engine.providers[pending.id] = active;
  useApp.setState({ accounts: [connected, pending], engine, services: [] });
  render(<Accounts />);
  const heading = await screen.findByRole("heading", { name: "Tài khoản đã đồng bộ" });
  const syncedSection = heading.closest("section")!;
  expect(within(syncedSection).getByText("Cần kết nối trên máy này")).toBeInTheDocument();
  expect(within(syncedSection).queryByText("Đang hoạt động")).not.toBeInTheDocument();
  expect(syncedSection.querySelector(".uc-account-status")).toBeNull();
  expect(syncedSection.querySelector(".uc-status-dot")).toBeNull();
  expect(within(syncedSection).queryByText("Đăng nhập qua trình duyệt")).not.toBeInTheDocument();
  fireEvent.click(within(syncedSection).getByRole("button", { name: "Kết nối Codex · Office trên máy này" }));
  expect(startAccountLogin).toHaveBeenCalledWith("codex");
  const connectedSection = screen.getByRole("heading", { name: "Tài khoản đã kết nối" }).closest("section")!;
  expect(connectedSection.querySelector(".uc-account-status")).not.toBeNull();
  expect(within(connectedSection).queryByText("Codex · Office")).not.toBeInTheDocument();
});

it("moves an account into connected accounts only after local credentials are available", async () => {
  const api = new MockBackend();
  setBackend(api);
  const list = vi.spyOn(api, "listAccounts").mockResolvedValue([pending]);
  vi.spyOn(api, "listServices").mockResolvedValue([]);
  useApp.setState({ accounts: [pending], services: [] });
  render(<Accounts />);
  await screen.findByText("Cần kết nối trên máy này");
  const connected: ConnectedAccount = {
    ...pending,
    requiresLogin: undefined,
    credentialMode: "managed_oauth",
    connectedAt: "2026-10-16T00:00:00Z",
    updatedAt: "2026-10-16T00:00:00Z",
  };
  list.mockResolvedValue([connected]);
  await act(async () => useApp.setState({ accounts: [connected] }));
  expect(screen.queryByRole("heading", { name: "Tài khoản đã đồng bộ" })).not.toBeInTheDocument();
  expect(screen.queryByText("Cần kết nối trên máy này")).not.toBeInTheDocument();
  expect(screen.getByText("Đăng nhập qua trình duyệt")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Kết nối Codex · Office trên máy này" })).not.toBeInTheDocument();
});
