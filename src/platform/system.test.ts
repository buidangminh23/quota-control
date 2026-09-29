import { setBackend, type Backend } from "@/lib/backend";
import type { SystemNotification } from "@/lib/types";
import { autostartEnabled, notificationAccess, notify, requestNotificationAccess, setAutostart } from "./system";

const plugin = vi.hoisted(() => ({
  isEnabled: vi.fn(async () => true),
  enable: vi.fn(async () => undefined),
  disable: vi.fn(async () => undefined),
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted" as const),
  sendNotification: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-autostart", () => ({ isEnabled: plugin.isEnabled, enable: plugin.enable, disable: plugin.disable }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: plugin.isPermissionGranted,
  requestPermission: plugin.requestPermission,
  sendNotification: plugin.sendNotification,
}));

function use(backend: Partial<Backend>): void {
  setBackend(backend as Backend);
}

beforeEach(() => {
  vi.clearAllMocks();
  Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
});

afterEach(() => {
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

describe("where the system has its own way (macOS)", () => {
  it("sends a notification through the system with what it is about, and not through the plugin too", async () => {
    const sent: SystemNotification[] = [];
    use({
      sendSystemNotification: async (notification) => {
        sent.push(notification);
        return true;
      },
    });
    await notify("Claude", "Almost out", { id: "usage.claude.session", group: "usage.claude" });
    expect(sent).toEqual([{ title: "Claude", body: "Almost out", id: "usage.claude.session", group: "usage.claude" }]);
    expect(plugin.sendNotification).not.toHaveBeenCalled();
  });

  it("offers to ask a user who was never asked, and knows a refusal", async () => {
    use({ systemNotificationAccess: async () => "undetermined", requestSystemNotificationAccess: async () => "granted" });
    expect(await notificationAccess()).toBe("denied");
    expect(await requestNotificationAccess()).toBe("granted");
    use({ systemNotificationAccess: async () => "denied" });
    expect(await notificationAccess()).toBe("denied");
    expect(plugin.isPermissionGranted).not.toHaveBeenCalled();
    expect(plugin.requestPermission).not.toHaveBeenCalled();
  });

  it("switches launch at login through the login item alone", async () => {
    const switched: boolean[] = [];
    use({
      systemLaunchAtLogin: async () => false,
      setSystemLaunchAtLogin: async (enabled) => {
        switched.push(enabled);
        return enabled;
      },
    });
    expect(await autostartEnabled()).toBe(false);
    await setAutostart(true);
    expect(switched).toEqual([true]);
    expect(plugin.enable).not.toHaveBeenCalled();
    expect(plugin.isEnabled).not.toHaveBeenCalled();
  });
});

describe("where it has none (Windows, Linux, a core without it)", () => {
  it("falls back to the plugins when the core says not here", async () => {
    use({
      sendSystemNotification: async () => false,
      systemNotificationAccess: async () => null,
      requestSystemNotificationAccess: async () => null,
      systemLaunchAtLogin: async () => null,
      setSystemLaunchAtLogin: async () => null,
    });
    await notify("Codex", "Reset", { id: "resets.latest", group: "resets" });
    expect(plugin.sendNotification).toHaveBeenCalledWith({ title: "Codex", body: "Reset" });
    expect(await notificationAccess()).toBe("granted");
    expect(await requestNotificationAccess()).toBe("granted");
    expect(await autostartEnabled()).toBe(true);
    await setAutostart(false);
    expect(plugin.disable).toHaveBeenCalledTimes(1);
  });

  it("uses the plugins with a core that has no such commands", async () => {
    use({});
    await notify("Codex", "Reset");
    expect(plugin.sendNotification).toHaveBeenCalledWith({ title: "Codex", body: "Reset" });
    await setAutostart(true);
    expect(plugin.enable).toHaveBeenCalledTimes(1);
  });
});
