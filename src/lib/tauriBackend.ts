import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend, DocumentName, Unsubscribe } from "./backend";
import type { AccountLogin, AccountProvider, AppInfo, ChatSession, ConnectedAccount, EngineState, PopoverScreen, ProviderEntry } from "./types";

/** Subscribe to a Tauri event synchronously; the returned function tears the listener down. */
function subscribe<T>(event: string, listener: (payload: T) => void, replay?: () => Promise<T>): Unsubscribe {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  let received = 0;
  listen<T>(event, (message) => {
    received += 1;
    if (!disposed) listener(message.payload);
  })
    .then((stop) => {
      if (disposed) stop();
      else {
        unlisten = stop;
        const revision = received;
        void replay?.().then((payload) => {
          if (!disposed && revision === received) listener(payload);
        }).catch((error: unknown) => console.error(`replay(${event}) failed`, error));
      }
    })
    .catch((error: unknown) => console.error(`listen(${event}) failed`, error));
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export class TauriBackend implements Backend {
  appInfo(): Promise<AppInfo> {
    return invoke<AppInfo>("app_info");
  }

  catalog(): Promise<ProviderEntry[]> {
    return invoke<ProviderEntry[]>("catalog");
  }

  onCatalogChanged(listener: (catalog: ProviderEntry[]) => void): Unsubscribe {
    return subscribe<ProviderEntry[]>("catalog-changed", listener, () => this.catalog());
  }

  listAccounts(): Promise<ConnectedAccount[]> {
    return invoke("list_accounts");
  }

  importCurrentAccount(provider: AccountProvider, label?: string): Promise<ConnectedAccount> {
    return invoke("import_current_account", { provider, label: label ?? null });
  }

  beginAccountLogin(provider: AccountProvider, label?: string): Promise<AccountLogin> {
    return invoke("begin_account_login", { provider, label: label ?? null });
  }

  completeAccountLogin(flowId: string, callback?: string): Promise<ConnectedAccount> {
    return invoke("complete_account_login", { flowId, callback: callback ?? null });
  }

  cancelAccountLogin(flowId: string): Promise<void> {
    return invoke("cancel_account_login", { flowId });
  }

  removeAccount(accountId: string): Promise<void> {
    return invoke("remove_account", { accountId });
  }

  listChatSessions(): Promise<ChatSession[]> {
    return invoke("list_chat_sessions");
  }

  createChatSession(provider: AccountProvider, label?: string): Promise<ChatSession> {
    return invoke("create_chat_session", { provider, label: label ?? null });
  }

  openChatSession(sessionId: string): Promise<void> {
    return invoke("open_chat_session", { sessionId });
  }

  engineState(): Promise<EngineState> {
    return invoke<EngineState>("engine_state");
  }

  onEngineState(listener: (state: EngineState) => void): Unsubscribe {
    return subscribe<EngineState>("engine-state", listener, () => this.engineState());
  }

  refresh(providerId?: string): Promise<void> {
    return invoke("refresh", { providerId: providerId ?? null });
  }

  setEnabledProviders(providerIds: string[]): Promise<void> {
    return invoke("set_enabled_providers", { providerIds });
  }

  loadDocument<T>(name: DocumentName): Promise<T | null> {
    return invoke<T | null>("load_document", { name });
  }

  saveDocument(name: DocumentName, value: unknown): Promise<void> {
    return invoke("save_document", { name, value });
  }

  resizePopup(height: number): Promise<void> {
    return invoke("resize_popup", { height });
  }

  hidePopup(): Promise<void> {
    return invoke("hide_popup");
  }

  onPopupVisibility(listener: (shown: boolean) => void): Unsubscribe {
    return subscribe<boolean>("popup-visibility", listener);
  }

  onNavigate(listener: (screen: PopoverScreen) => void): Unsubscribe {
    return subscribe<PopoverScreen>("navigate", listener);
  }

  openUrl(url: string): Promise<void> {
    return invoke("open_url", { url });
  }

  copyImagePng(png: Uint8Array): Promise<void> {
    return invoke("copy_image_png", { png: Array.from(png) });
  }

  copyText(text: string): Promise<void> {
    return invoke("copy_text", { text });
  }

  setTrayIcon(png: Uint8Array | null, tooltip: string): Promise<void> {
    return invoke("set_tray_icon", { png: png ? Array.from(png) : null, tooltip });
  }

  quit(): Promise<void> {
    return invoke("quit_app");
  }
}
