import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend, DocumentName, Unsubscribe } from "./backend";
import type { AppInfo, EngineState, PopoverScreen, ProviderEntry } from "./types";

/** Subscribe to a Tauri event synchronously; the returned function tears the listener down. */
function subscribe<T>(event: string, listener: (payload: T) => void): Unsubscribe {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  listen<T>(event, (message) => listener(message.payload))
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
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

  engineState(): Promise<EngineState> {
    return invoke<EngineState>("engine_state");
  }

  onEngineState(listener: (state: EngineState) => void): Unsubscribe {
    return subscribe<EngineState>("engine-state", listener);
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
