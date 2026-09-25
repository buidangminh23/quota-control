/**
 * Browser-only backend for building the popup without the Rust core. Serves static fixtures and
 * simulates refreshes. Never bundled into the Tauri app's startup path (loaded lazily by `main.tsx`).
 */
import type { Backend, DocumentName, Unsubscribe } from "./backend";
import type { AppInfo, EngineState, PopoverScreen, ProviderEntry } from "./types";
import { fixtureCatalog, fixtureEngineState } from "./fixtures";

export class MockBackend implements Backend {
  private state: EngineState = fixtureEngineState();
  private readonly engineListeners = new Set<(state: EngineState) => void>();
  private readonly visibilityListeners = new Set<(shown: boolean) => void>();
  private readonly navigateListeners = new Set<(screen: PopoverScreen) => void>();
  private readonly documents = new Map<DocumentName, unknown>();

  async appInfo(): Promise<AppInfo> {
    return { name: "Usage Control", version: "0.1.0", platform: "web" };
  }

  async catalog(): Promise<ProviderEntry[]> {
    return fixtureCatalog();
  }

  async engineState(): Promise<EngineState> {
    return this.state;
  }

  onEngineState(listener: (state: EngineState) => void): Unsubscribe {
    this.engineListeners.add(listener);
    return () => this.engineListeners.delete(listener);
  }

  async refresh(providerId?: string): Promise<void> {
    const ids = providerId ? [providerId] : Object.keys(this.state.providers);
    this.update((state) => {
      for (const id of ids) {
        const entry = state.providers[id];
        if (entry) entry.refreshing = true;
      }
    });
    await new Promise((resolve) => setTimeout(resolve, 600));
    this.update((state) => {
      for (const id of ids) {
        const entry = state.providers[id];
        if (entry) entry.refreshing = false;
      }
      state.lastRefreshAt = new Date().toISOString();
    });
  }

  async setEnabledProviders(): Promise<void> {}

  async loadDocument<T>(name: DocumentName): Promise<T | null> {
    return (this.documents.get(name) as T | undefined) ?? null;
  }

  async saveDocument(name: DocumentName, value: unknown): Promise<void> {
    this.documents.set(name, structuredClone(value));
  }

  async resizePopup(): Promise<void> {}

  async hidePopup(): Promise<void> {
    for (const listener of this.visibilityListeners) listener(false);
  }

  onPopupVisibility(listener: (shown: boolean) => void): Unsubscribe {
    this.visibilityListeners.add(listener);
    return () => this.visibilityListeners.delete(listener);
  }

  onNavigate(listener: (screen: PopoverScreen) => void): Unsubscribe {
    this.navigateListeners.add(listener);
    return () => this.navigateListeners.delete(listener);
  }

  async openUrl(url: string): Promise<void> {
    window.open(url, "_blank", "noopener");
  }

  async copyImagePng(): Promise<void> {}

  async copyText(text: string): Promise<void> {
    await navigator.clipboard?.writeText(text);
  }

  async setTrayIcon(): Promise<void> {}

  async quit(): Promise<void> {}

  private update(mutate: (state: EngineState) => void): void {
    const next = structuredClone(this.state);
    mutate(next);
    this.state = next;
    for (const listener of this.engineListeners) listener(next);
  }
}
