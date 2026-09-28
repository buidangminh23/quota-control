/**
 * Canvas renderers for the taskbar: the Bars glyph pushed into the tray icon (upstream
 * `MenuBarBars`, with the same pad/gap/radius rules and fill geometry) and the text strip of provider
 * marks with their values stacked two high (upstream `MenuBarTextStrip`). The taskbar style is
 * scaled to the Windows taskbar the way the system clock stacks time over date; the menu bar style
 * keeps upstream's own macOS metrics, drawn black so macOS tints the template for the menu bar.
 */
import { colorArtUrl, PROVIDER_COLOR_ART } from "@/assets/providerColorArt";
import { PROVIDER_MARKS } from "@/assets/providerMarks";
import { barFill, type StripContent, type StripMetric } from "@/model/menuBar";
import { knownBrandColor } from "@/model/totalSpend";

export type StripStyle = "taskbar" | "menuBar" | "panel";

export const GLYPH_SIDE = 16;
/** Upstream draws the Bars glyph 18 points square in the macOS menu bar. */
export const MENU_BAR_GLYPH_SIDE = 18;
const MARK_INSET = 0.04;

interface StripMetrics {
  font: string;
  singleSize: number;
  stackedSize: number;
  stackedLineHeight: number;
  markSide: number;
  markGap: number;
  groupGap: number;
  sidePadding: number;
  /** Size of the window names (`5h`, `week`) before the readings; `null` leaves them out. */
  labelSize: number | null;
  labelGap: number;
}

const STRIP_METRICS: Readonly<Record<StripStyle, StripMetrics>> = {
  taskbar: {
    font: '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif',
    singleSize: 14,
    stackedSize: 12,
    stackedLineHeight: 14,
    markSide: 18,
    markGap: 5,
    groupGap: 14,
    sidePadding: 6,
    labelSize: 10,
    labelGap: 3,
  },
  menuBar: {
    font: '-apple-system, "SF Pro Text", system-ui, sans-serif',
    singleSize: 12,
    stackedSize: 9,
    stackedLineHeight: 9,
    markSide: 16,
    markGap: 4,
    groupGap: 11,
    sidePadding: 2,
    labelSize: null,
    labelGap: 0,
  },
  panel: {
    font: 'Ubuntu, Cantarell, "Noto Sans", system-ui, sans-serif',
    singleSize: 12,
    stackedSize: 11,
    stackedLineHeight: 12,
    markSide: 18,
    markGap: 4,
    groupGap: 10,
    sidePadding: 1,
    labelSize: 9,
    labelGap: 2,
  },
};

function canvas(width: number, height: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const element = document.createElement("canvas");
  element.width = Math.max(1, Math.round(width));
  element.height = Math.max(1, Math.round(height));
  const context = element.getContext("2d");
  if (!context) throw new Error("Canvas 2D context unavailable");
  return [element, context];
}

async function toPng(element: HTMLCanvasElement): Promise<Uint8Array> {
  const blob = await new Promise<Blob | null>((resolve) => element.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("Canvas produced no PNG");
  return new Uint8Array(await blob.arrayBuffer());
}

function roundedBar(context: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, leading: number, trailing: number): void {
  context.beginPath();
  context.roundRect(x, y, w, h, [leading, trailing, trailing, leading]);
  context.fill();
}

function withAlpha(color: string, alpha: number): string {
  return color === "#ffffff" ? `rgba(255,255,255,${alpha})` : `rgba(0,0,0,${alpha})`;
}

/** The Bars glyph `sidePoints` square at `scale` device pixels per point, drawn in `color` (black or white). */
export async function renderBarsGlyph(bars: readonly StripMetric[], scale: number, color: "#000000" | "#ffffff", sidePoints = GLYPH_SIDE): Promise<Uint8Array> {
  const side = sidePoints * scale;
  const [element, context] = canvas(side, side);
  const n = Math.max(1, Math.min(4, bars.length));
  const pad = Math.max(1, Math.round(side * 0.08));
  const gap = Math.max(1, Math.round(side * 0.03));
  const trackW = side - 2 * pad;
  const layoutN = Math.max(2, n);
  const trackH = Math.max(1, Math.floor((side - 2 * pad - (layoutN - 1) * gap) / layoutN));
  const rx = Math.max(1, Math.floor(trackH / 3));
  const total = n * trackH + (n - 1) * gap;
  const top = pad + Math.floor((side - 2 * pad - total) / 2);
  for (let index = 0; index < n; index += 1) {
    const y = top + index * (trackH + gap);
    context.fillStyle = withAlpha(color, 0.3);
    roundedBar(context, pad, y, trackW, trackH, rx, rx);
    const fill = barFill(trackW, bars[index]?.fraction ?? 0);
    if (fill.fillW > 0) {
      context.fillStyle = color;
      roundedBar(context, pad, y, fill.fillW, trackH, rx, fill.fillW >= trackW ? rx : Math.floor(rx * 0.35));
    }
    if (fill.fillW > 0 && fill.remainderW > 0 && fill.dividerX !== null) {
      context.fillStyle = withAlpha(color, 0.42);
      roundedBar(context, pad + fill.dividerX, y, fill.remainderW, trackH, Math.floor(rx * 0.2), rx);
    }
  }
  return toPng(element);
}

const colorArtImages = new Map<string, Promise<HTMLImageElement | null>>();

/** `brand`'s official color logo, decoded once, or `null` for a brand drawn in a single color. */
function colorArtImage(brand: string): Promise<HTMLImageElement | null> {
  const art = PROVIDER_COLOR_ART[brand];
  if (!art) return Promise.resolve(null);
  let pending = colorArtImages.get(brand);
  if (!pending) {
    const image = new Image();
    image.src = colorArtUrl(art, MARK_INSET);
    pending = typeof image.decode === "function" ? image.decode().then(() => image, () => null) : Promise.resolve(null);
    colorArtImages.set(brand, pending);
  }
  return pending;
}

function drawMark(context: CanvasRenderingContext2D, brand: string, x: number, y: number, side: number, color: string, art: HTMLImageElement | null): void {
  if (art) {
    context.drawImage(art, x, y, side, side);
    return;
  }
  const mark = PROVIDER_MARKS[brand];
  context.fillStyle = color;
  if (!mark) {
    context.beginPath();
    context.arc(x + side / 2, y + side / 2, side / 2 - 0.5, 0, Math.PI * 2);
    context.fill();
    return;
  }
  const [minX, minY, width, height] = mark.box;
  const box = Math.max(width, height) * (1 + MARK_INSET * 2);
  const factor = side / box;
  context.save();
  context.translate(x + side / 2, y + side / 2);
  context.scale(factor, factor);
  context.translate(-(minX + width / 2), -(minY + height / 2));
  for (const path of mark.paths) context.fill(new Path2D(path.d), path.evenOdd ? "evenodd" : "nonzero");
  context.restore();
}

interface MeasuredRow {
  label: string | null;
  value: string;
}

interface MeasuredGroup {
  brand: string;
  rows: MeasuredRow[];
  width: number;
}

/**
 * The color of `brand`'s mark on the strip: on the Windows taskbar the brand's own tint for the
 * taskbar's light or dark theme, the one the popup gives the mark, or `color` for a brand without
 * one; the macOS menu bar keeps every mark in `color`, a template the system tints. A brand with an
 * official color logo (`providerColorArt.ts`) shows that logo on the taskbar instead.
 */
export function stripMarkColor(brand: string, color: "#000000" | "#ffffff", style: StripStyle): string {
  if (style === "menuBar") return color;
  return knownBrandColor(brand, color === "#ffffff") ?? color;
}

/** The text strip for a band `height` device pixels tall, or `null` when there is nothing to show. */
export async function renderTextStrip(
  content: StripContent,
  height: number,
  scale: number,
  color: "#000000" | "#ffffff",
  style: StripStyle = "taskbar",
): Promise<{ png: Uint8Array; width: number; height: number } | null> {
  if (content.groups.length === 0) return null;
  const metrics = STRIP_METRICS[style];
  const [, measure] = canvas(1, 1);
  const font = (size: number, weight: number) => `${weight} ${size * scale}px ${metrics.font}`;
  const labelFont = metrics.labelSize === null ? null : font(metrics.labelSize, 500);
  const valueFont = (rows: number) => (rows > 1 ? font(metrics.stackedSize, 600) : font(metrics.singleSize, 700));
  const groups: MeasuredGroup[] = content.groups.map((group) => {
    const rows = group.metrics.slice(0, 2).map((metric) => ({ label: labelFont ? metric.period : null, value: metric.value }));
    const rowWidths = rows.map((row) => {
      measure.font = valueFont(rows.length);
      const value = measure.measureText(row.value).width;
      if (!row.label || !labelFont) return value;
      measure.font = labelFont;
      return measure.measureText(row.label).width + metrics.labelGap * scale + value;
    });
    return { brand: group.brand, rows, width: (metrics.markSide + metrics.markGap) * scale + Math.ceil(Math.max(...rowWidths)) };
  });
  const width = Math.ceil(
    metrics.sidePadding * 2 * scale + groups.reduce((sum, group) => sum + group.width, 0) + metrics.groupGap * scale * (groups.length - 1),
  );
  const art = new Map(await Promise.all(groups.map(async (group) => [group.brand, style !== "menuBar" ? await colorArtImage(group.brand) : null] as const)));
  const [element, context] = canvas(width, height);
  context.textBaseline = "middle";
  context.textAlign = "right";
  let x = metrics.sidePadding * scale;
  const middle = height / 2;
  const offset = (metrics.stackedLineHeight * scale) / 2;
  for (const group of groups) {
    drawMark(context, group.brand, x, middle - (metrics.markSide * scale) / 2, metrics.markSide * scale, stripMarkColor(group.brand, color, style), art.get(group.brand) ?? null);
    const left = x + (metrics.markSide + metrics.markGap) * scale;
    const right = x + group.width;
    const lines = group.rows.length > 1 ? [middle - offset + scale, middle + offset] : [middle + scale];
    group.rows.forEach((row, index) => {
      const y = lines[index]!;
      context.font = valueFont(group.rows.length);
      context.textAlign = "right";
      context.fillStyle = color;
      context.fillText(row.value, right, y);
      if (!row.label || !labelFont) return;
      const baseline = y + alphabeticDrop(context);
      context.font = labelFont;
      context.textAlign = "left";
      context.textBaseline = "alphabetic";
      context.fillStyle = withAlpha(color, 0.8);
      context.fillText(row.label, left, baseline);
      context.textBaseline = "middle";
    });
    x = right + metrics.groupGap * scale;
  }
  return { png: await toPng(element), width, height };
}

/** How far the current font's alphabetic baseline sits below a `middle` baseline, so smaller text can share it. */
function alphabeticDrop(context: CanvasRenderingContext2D): number {
  const metrics = context.measureText("0");
  return -metrics.alphabeticBaseline;
}

/** Plain-text form of the strip, `Claude 12% 58%  Codex 100% 36%`, sent along with its picture. */
export function stripText(content: StripContent): string {
  return content.groups.map((group) => [group.displayName, ...group.metrics.slice(0, 2).map((metric) => metric.value)].join(" ")).join("  ");
}
