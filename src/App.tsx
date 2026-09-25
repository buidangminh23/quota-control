import { useEffect, useState } from "react";
import { backend } from "./lib/backend";
import type { AppInfo, EngineState, ProviderEntry } from "./lib/types";

/** Temporary shell until the dashboard port lands: proves the window, IPC and events work. */
export function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [catalog, setCatalog] = useState<ProviderEntry[]>([]);
  const [state, setState] = useState<EngineState | null>(null);

  useEffect(() => {
    const api = backend();
    void api.appInfo().then(setInfo);
    void api.catalog().then(setCatalog);
    void api.engineState().then(setState);
    return api.onEngineState(setState);
  }, []);

  useEffect(() => {
    const height = document.getElementById("root")?.scrollHeight ?? 0;
    if (height > 0) void backend().resizePopup(height);
  }, [catalog, state]);

  return (
    <main className="popup">
      <h1>{info?.name ?? "Usage Control"}</h1>
      <ul>
        {catalog.map((entry) => {
          const runtime = state?.providers[entry.provider.id];
          return (
            <li key={entry.provider.id}>
              {entry.provider.displayName}: {runtime?.refreshing ? "refreshing…" : runtime?.error ?? runtime?.snapshot?.plan ?? "—"}
            </li>
          );
        })}
      </ul>
      <footer>
        {info ? `${info.name} ${info.version}` : null}
        <button type="button" onClick={() => void backend().refresh()}>
          Refresh
        </button>
        <button type="button" onClick={() => void backend().quit()}>
          Quit
        </button>
      </footer>
    </main>
  );
}
