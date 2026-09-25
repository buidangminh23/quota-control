import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter";
import "./index.css";
import { App } from "./App";
import { isTauri, setBackend } from "./lib/backend";
import { TauriBackend } from "./lib/tauriBackend";

async function bootstrap(): Promise<void> {
  if (isTauri()) {
    setBackend(new TauriBackend());
  } else {
    const { MockBackend } = await import("./lib/mockBackend");
    setBackend(new MockBackend());
  }
  const root = document.getElementById("root");
  if (!root) throw new Error("#root element missing from index.html");
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void bootstrap();
