import { setBackend } from "@/lib/backend";
import { fixtureAccounts } from "@/lib/fixtures";
import { MockBackend } from "@/lib/mockBackend";
import type { AccountLoginResult, ConnectedAccount } from "@/lib/types";
import { clearNotice, reloadAccounts, startApp, useApp } from "./store";

const fresh = useApp.getState();
let stop: (() => void) | undefined;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  useApp.setState(fresh, true);
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(async () => {
  stop?.();
  stop = undefined;
  clearNotice();
  await new Promise((resolve) => setTimeout(resolve, 5));
  vi.restoreAllMocks();
  useApp.setState(fresh, true);
});

it.each([
  ["vi", "second@example.com", "Đã kết nối Codex · second@example.com"],
  ["en", "second@example.com", "Codex · second@example.com connected"],
  ["vi", undefined, "Đã kết nối Codex"],
  ["vi", "codex", "Đã kết nối Codex"],
] as const)("identifies the completed account in %s without relying on the current login", async (language, accountLabel, expected) => {
  const api = new MockBackend();
  setBackend(api);
  let receive: ((result: AccountLoginResult) => void) | undefined;
  vi.spyOn(api, "onAccountLogin").mockImplementation((listener) => {
    receive = listener;
    return () => {};
  });
  stop = startApp();
  await vi.waitFor(() => expect(receive).toBeDefined());
  useApp.setState({
    settings: { ...useApp.getState().settings, language },
    accountLogin: { phase: "waiting", provider: "codex", method: "google", flowId: "newer-flow", browser: "chrome" },
  });
  receive!({ flowId: "finished-flow", provider: "codex", status: "connected", accountId: "codex@second", accountLabel });
  expect(useApp.getState().notice?.text).toBe(expected);
  expect(useApp.getState().accountLogin).toMatchObject({ flowId: "newer-flow" });
});

it("keeps newly connected accounts when an older reload responds last", async () => {
  const api = new MockBackend();
  setBackend(api);
  const saved = fixtureAccounts().filter((account) => account.credentialMode === "managed_oauth");
  const hidden = fixtureAccounts().filter((account) => account.credentialMode === "cli");
  const earlierAccounts = deferred<ConnectedAccount[]>();
  const earlierHidden = deferred<ConnectedAccount[]>();
  vi.spyOn(api, "listAccounts").mockReturnValueOnce(earlierAccounts.promise).mockResolvedValueOnce(saved);
  vi.spyOn(api, "listRemovedLogins").mockReturnValueOnce(earlierHidden.promise).mockResolvedValueOnce(hidden);

  const earlier = reloadAccounts();
  await reloadAccounts();
  expect(useApp.getState().accounts).toEqual(saved);
  expect(useApp.getState().removedLogins).toEqual(hidden);

  earlierAccounts.resolve([]);
  earlierHidden.resolve([]);
  await earlier;
  expect(useApp.getState().accounts).toEqual(saved);
  expect(useApp.getState().removedLogins).toEqual(hidden);
});

it("preserves current accounts when a newer reload fails and an older reply arrives later", async () => {
  const api = new MockBackend();
  setBackend(api);
  const saved = fixtureAccounts();
  useApp.setState({ accounts: saved, removedLogins: saved.slice(-1) });
  const earlierAccounts = deferred<ConnectedAccount[]>();
  vi.spyOn(api, "listAccounts").mockReturnValueOnce(earlierAccounts.promise).mockRejectedValueOnce(new Error("registry unavailable"));
  vi.spyOn(api, "listRemovedLogins").mockResolvedValue([]);

  const earlier = reloadAccounts();
  await reloadAccounts();
  earlierAccounts.resolve([]);
  await earlier;
  expect(useApp.getState().accounts).toEqual(saved);
  expect(useApp.getState().removedLogins).toEqual(saved.slice(-1));
});

it("keeps a successful newer reload when an older request fails", async () => {
  const api = new MockBackend();
  setBackend(api);
  const saved = fixtureAccounts();
  const earlierAccounts = deferred<ConnectedAccount[]>();
  vi.spyOn(api, "listAccounts").mockReturnValueOnce(earlierAccounts.promise).mockResolvedValueOnce(saved);
  vi.spyOn(api, "listRemovedLogins").mockResolvedValue([]);

  const earlier = reloadAccounts();
  await reloadAccounts();
  earlierAccounts.reject(new Error("older request failed"));
  await earlier;
  expect(useApp.getState().accounts).toEqual(saved);
});

it("keeps the entire account snapshot if removed-login listing fails", async () => {
  const api = new MockBackend();
  setBackend(api);
  const saved = fixtureAccounts();
  const hidden = saved.slice(-1);
  useApp.setState({ accounts: saved, removedLogins: hidden });
  vi.spyOn(api, "listAccounts").mockResolvedValue([]);
  vi.spyOn(api, "listRemovedLogins").mockRejectedValue(new Error("removed logins unavailable"));

  await reloadAccounts();
  expect(useApp.getState().accounts).toEqual(saved);
  expect(useApp.getState().removedLogins).toEqual(hidden);
});
