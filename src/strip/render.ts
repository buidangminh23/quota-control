/**
 * Canvas renderers for the taskbar: the Bars glyph pushed into the tray icon (upstream
 * `MenuBarBars`, with the same pad/gap/radius rules and fill geometry) and the text strip of provider
 * marks with their values stacked two high (upstream `MenuBarTextStrip`, scaled to the Windows
 * taskbar the way the system clock stacks time over date).
 */
import { PROVIDER_MARKS } from "@/assets/providerMarks";
import { barFill, type StripContent, type StripMetric } from "@/model/menuBar";

export const GLYPH_SIDE = 16;
const STRIP_FONT = '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';
const SINGLE_VALUE_SIZE = 14;
const STACKED_VALUE_SIZE = 12;
const STACKED_LINE_HEIGHT = 14;
const MARK_SIDE = 18;
const MARK_GAP = 5;
const GROUP_GAP = 14;
const SIDE_PADDING = 6;
const MARK_INSET = 0.04;

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

/** The Bars glyph at `scale` device pixels per point, drawn in `color` (black or white). */
export async function renderBarsGlyph(bars: readonly StripMetric[], scale: number, color: "#000000" | "#ffffff"): Promise<Uint8Array> {
  const side = GLYPH_SIDE * scale;
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

function drawMark(context: CanvasRenderingContext2D, brand: string, x: number, y: number, side: number, color: string): void {
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

interface MeasuredGroup {
  brand: string;
  values: string[];
  width: number;
}

/** The text strip for a band `height` device pixels tall, or `null` when there is nothing to show. */
export async function renderTextStrip(content: StripContent, height: number, scale: number, color: "#000000" | "#ffffff"): Promise<{ png: Uint8Array; width: number; height: number } | null> {
  if (content.groups.length === 0) return null;
  const [, measure] = canvas(1, 1);
  const font = (size: number, weight: number) => `${weight} ${size * scale}px ${STRIP_FONT}`;
  const groups: MeasuredGroup[] = content.groups.map((group) => {
    const values = group.metrics.slice(0, 2).map((metric) => metric.value);
    measure.font = values.length > 1 ? font(STACKED_VALUE_SIZE, 600) : font(SINGLE_VALUE_SIZE, 700);
    const textWidth = Math.max(...values.map((value) => measure.measureText(value).width));
    return { brand: group.brand, values, width: (MARK_SIDE + MARK_GAP) * scale + Math.ceil(textWidth) };
  });
  const width = Math.ceil(SIDE_PADDING * 2 * scale + groups.reduce((sum, group) => sum + group.width, 0) + GROUP_GAP * scale * (groups.length - 1));
  const [element, context] = canvas(width, height);
  context.textBaseline = "middle";
  context.textAlign = "right";
  let x = SIDE_PADDING * scale;
  const middle = height / 2;
  for (const group of groups) {
    drawMark(context, group.brand, x, middle - (MARK_SIDE * scale) / 2, MARK_SIDE * scale, color);
    const right = x + group.width;
    context.fillStyle = color;
    if (group.values.length > 1) {
      context.font = font(STACKED_VALUE_SIZE, 600);
      const offset = (STACKED_LINE_HEIGHT * scale) / 2;
      context.fillText(group.values[0]!, right, middle - offset + scale);
      context.fillText(group.values[1]!, right, middle + offset);
    } else {
      context.font = font(SINGLE_VALUE_SIZE, 700);
      context.fillText(group.values[0] ?? "", right, middle + scale);
    }
    x = right + GROUP_GAP * scale;
  }
  return { png: await toPng(element), width, height };
}

/** Plain-text strip for tray titles (Linux): `Claude 12% 58%  Codex 100% 36%`. */
export function stripText(content: StripContent): string {
  return content.groups.map((group) => [group.displayName, ...group.metrics.slice(0, 2).map((metric) => metric.value)].join(" ")).join("  ");
}
