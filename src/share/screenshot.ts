/**
 * "Share Screenshot": render a card of the popup to a PNG and put it on the clipboard through the
 * core (upstream `ShareCardRenderer`). The capture uses the live DOM node at 2x on the popup's own
 * background so it reads the same as on screen.
 */
import { toBlob } from "html-to-image";
import { backend } from "@/lib/backend";

const PIXEL_RATIO = 2;
const PADDING = 12;

function trayColor(): string {
  return getComputedStyle(document.documentElement).getPropertyValue("--uc-tray").trim() || "#ffffff";
}

export async function captureElementPng(element: HTMLElement): Promise<Uint8Array> {
  const blob = await toBlob(element, {
    pixelRatio: PIXEL_RATIO,
    backgroundColor: trayColor(),
    cacheBust: true,
    skipFonts: true,
    style: { margin: "0", padding: `${PADDING}px`, boxSizing: "content-box" },
    width: element.offsetWidth + PADDING * 2,
    height: element.offsetHeight + PADDING * 2,
    filter: (node) => !(node instanceof HTMLElement && node.dataset.shareExclude === "true"),
  });
  if (!blob) throw new Error("Screenshot rendering produced no image");
  return new Uint8Array(await blob.arrayBuffer());
}

/** Copy a screenshot of `element`; resolves `true` when the image reached the clipboard. */
export async function copyScreenshot(element: HTMLElement | null): Promise<boolean> {
  if (!element) return false;
  try {
    await backend().copyImagePng(await captureElementPng(element));
    return true;
  } catch (error) {
    console.error("Copying screenshot failed", error);
    return false;
  }
}
