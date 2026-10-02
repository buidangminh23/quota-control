/**
 * Keeps the taskbar in sync with the starred metrics, live: every engine refresh, layout change or
 * setting change re-renders the strip (text style, when the core hosts one) or the tray-icon Bars
 * glyph, and pushes it only when the picture actually changed. The hidden popup keeps running, so the
 * taskbar updates while the popup is closed.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { messagesFor } from "@/i18n";
import { backend } from "@/lib/backend";
import { markArt } from "@/glance/markArt";
import { glanceGroups } from "@/model/layout";
import { buildStripContent, isStripEmpty, stripSummary, type StripContent } from "@/model/menuBar";
import { providerTitle } from "@/model/providerText";
import { widgetDataFor } from "@/model/widgetData";
import { useDisplay, useIsEnabled, useSystemDark } from "@/state/hooks";
import { useApp } from "@/state/store";
import { MENU_BAR_GLYPH_SIDE, GLYPH_SIDE, renderBarsGlyph, renderTextStrip, stripText, type StripStyle } from "./render";
import { nativeStrip, type NativeStrip } from "./native";
import { pushStripFrame, useTaskbarInfo, watchTaskbarInfo } from "./support";

type Output =
  | { kind: "off"; tooltip: string }
  | { kind: "bars"; content: StripContent; color: "#000000" | "#ffffff"; style: StripStyle; tooltip: string }
  | { kind: "text"; content: StripContent; color: "#000000" | "#ffffff"; style: StripStyle; height: number; scale: number; tooltip: string };

function outputKey(output: Output): string {
  if (output.kind === "off") return `off|${output.tooltip}`;
  const values = output.content.groups
    .map((group) => `${group.brand}:${group.metrics.map((metric) => `${metric.period ?? ""} ${metric.value}/${metric.fraction.toFixed(3)}`).join(",")}`)
    .join(";");
  if (output.kind === "bars") return `bars|${output.color}|${output.style}|${values}|${output.tooltip}`;
  return `text|${output.color}|${output.style}|${output.height}|${output.scale}|${values}|${output.tooltip}`;
}

async function apply(output: Output, appName: string): Promise<void> {
  const api = backend();
  if (output.kind === "off") {
    await Promise.all([api.setTrayIcon(null, output.tooltip), pushStripFrame(null)]);
    return;
  }
  if (output.kind === "bars") {
    const side = output.style === "menuBar" ? MENU_BAR_GLYPH_SIDE : GLYPH_SIDE;
    const png = await renderBarsGlyph(output.content.bars, Math.max(2, Math.ceil(window.devicePixelRatio || 1)), output.color, side);
    await Promise.all([api.setTrayIcon(output.content.bars.length > 0 ? png : null, output.tooltip), pushStripFrame(null)]);
    return;
  }
  const frame = await renderTextStrip(output.content, output.height, output.scale, output.color, output.style);
  const native = frame && output.style === "menuBar" ? await describe(output.content, output.height, output.scale) : null;
  await Promise.all([
    api.setTrayIcon(null, appName),
    pushStripFrame(frame ? { ...frame, text: stripText(output.content), tooltip: output.tooltip, ...(native ? { native } : {}) } : null),
  ]);
}

/** The strip as a description for macOS to draw, with the color logos of the brands that have one. */
async function describe(content: StripContent, height: number, scale: number): Promise<NativeStrip | null> {
  const brands = [...new Set(content.groups.map((group) => group.brand))];
  const drawn = await Promise.all(brands.map(async (brand) => [brand, await markArt(brand)] as const));
  const art = Object.fromEntries(drawn.filter((entry): entry is readonly [string, string] => entry[1] !== null));
  return nativeStrip(content, height, scale, art);
}

export function useTaskbarStrip(): void {
  const ready = useApp((state) => state.ready);
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const engine = useApp((state) => state.engine);
  const info = useApp((state) => state.info);
  const showStrip = useApp((state) => state.settings.showTaskbarStrip);
  const iconStyle = useApp((state) => state.settings.iconStyle);
  const strip = useApp((state) => state.settings.strip);
  const display = useDisplay();
  const isEnabled = useIsEnabled();
  const taskbar = useTaskbarInfo();
  const systemDark = useSystemDark();
  const lastKey = useRef<string | null>(null);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const retry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(false);
  const [retryAttempt, setRetryAttempt] = useState(0);

  useEffect(() => watchTaskbarInfo(), []);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (retry.current !== null) clearTimeout(retry.current);
    };
  }, []);

  const content = useMemo(() => {
    const groups = glanceGroups(strip.content, strip.metrics, layout, catalog, isEnabled);
    return buildStripContent(
      groups,
      (descriptor) => widgetDataFor(descriptor, engine?.providers[descriptor.providerId]?.snapshot, display),
      (provider) => providerTitle(provider, display.language),
      strip.values,
    );
  }, [layout, catalog, isEnabled, engine, display, strip]);

  useEffect(() => {
    if (!ready) return;
    const messages = messagesFor(display.language);
    const appName = info?.name ?? messages.chrome.appName;
    const dark = taskbar ? taskbar.theme === "dark" : systemDark;
    const color = dark ? "#ffffff" : "#000000";
    const style: StripStyle = info?.platform === "macos" ? "menuBar" : info?.platform === "linux" ? "panel" : "taskbar";
    const empty = isStripEmpty(content);
    const tooltip = empty ? messages.strip.tooltipEmpty : `${appName}\n${stripSummary(content)}`;
    let output: Output;
    if (!showStrip || empty) output = { kind: "off", tooltip: empty ? messages.strip.tooltipEmpty : appName };
    else if (taskbar?.supported && iconStyle === "text") output = { kind: "text", content, color, style, height: taskbar.height, scale: taskbar.scale, tooltip };
    else output = { kind: "bars", content, color, style, tooltip };
    const key = outputKey(output);
    if (key === lastKey.current) return;
    if (retry.current !== null) clearTimeout(retry.current);
    lastKey.current = key;
    queue.current = queue.current
      .then(() => apply(output, appName))
      .catch((error: unknown) => {
        lastKey.current = null;
        console.error("Updating the taskbar failed", error);
        if (mounted.current) {
          retry.current = setTimeout(() => setRetryAttempt((attempt) => attempt + 1), 5_000);
        }
      });
  }, [ready, content, showStrip, iconStyle, taskbar, systemDark, display.language, info, retryAttempt]);
}
