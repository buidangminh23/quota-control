/**
 * The official color logos (`providerColorArt.ts`) as small PNGs for the macOS island and widgets.
 * SwiftUI draws the single-color marks from their path data but cannot draw these SVGs (they are
 * blurred gradients under a mask), so the webview rasterizes each one once, the way the taskbar
 * strip draws them, and the glance document carries the picture. Brands without color art keep
 * their tinted mark.
 */
import { useEffect, useState } from "react";
import { colorArtUrl, PROVIDER_COLOR_ART } from "@/assets/providerColorArt";

/** Pixels per side: 32 points at 2x, crisp at every size the island and widgets draw a mark. */
export const MARK_ART_PIXELS = 64;
/** The clear margin around the logo, as the single-color marks and the strip use. */
const MARK_ART_INSET = 0.04;
const PNG_PREFIX = "data:image/png;base64,";

const rendered = new Map<string, Promise<string | null>>();

/** The brand's color logo as base64 PNG, or `null` when it has none or it cannot be drawn. */
export function markArt(brand: string): Promise<string | null> {
  const art = PROVIDER_COLOR_ART[brand];
  if (!art) return Promise.resolve(null);
  let pending = rendered.get(brand);
  if (!pending) {
    pending = new Promise<string | null>((resolve) => {
      const image = new Image();
      image.onload = () => {
        const canvas = document.createElement("canvas");
        canvas.width = MARK_ART_PIXELS;
        canvas.height = MARK_ART_PIXELS;
        const context = canvas.getContext("2d");
        if (!context) return resolve(null);
        context.drawImage(image, 0, 0, MARK_ART_PIXELS, MARK_ART_PIXELS);
        const url = canvas.toDataURL("image/png");
        resolve(url.startsWith(PNG_PREFIX) ? url.slice(PNG_PREFIX.length) : null);
      };
      image.onerror = () => resolve(null);
      image.src = colorArtUrl(art, MARK_ART_INSET);
    });
    rendered.set(brand, pending);
  }
  return pending;
}

/** The color logos of `brands` that have one, keyed by brand, filled in as each is drawn. */
export function useMarkArt(brands: readonly string[]): Readonly<Record<string, string>> {
  const [pictures, setPictures] = useState<Readonly<Record<string, string>>>({});
  const key = [...new Set(brands.filter((brand) => PROVIDER_COLOR_ART[brand]))].sort().join("|");

  useEffect(() => {
    if (!key) return;
    let live = true;
    void Promise.all(key.split("|").map(async (brand) => [brand, await markArt(brand)] as const)).then((entries) => {
      if (!live) return;
      const drawn = Object.fromEntries(entries.filter((entry): entry is readonly [string, string] => entry[1] !== null));
      setPictures((current) => (Object.keys(drawn).every((brand) => current[brand] === drawn[brand]) && Object.keys(current).length === Object.keys(drawn).length ? current : drawn));
    });
    return () => {
      live = false;
    };
  }, [key]);

  return pictures;
}
