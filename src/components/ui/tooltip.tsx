/**
 * Hover tooltips drawn inside the popup (upstream `HoverTooltip`): a short delay on first hover,
 * instant while another tooltip was just visible, placed above the element when it fits and clamped
 * to the window so nothing is cut off at the popup's narrow edges.
 */
import { useLayoutEffect, useRef, useState, useSyncExternalStore, type FocusEvent, type PointerEvent } from "react";

const SHOW_DELAY_MS = 450;
const WARM_WINDOW_MS = 400;
const EDGE = 6;
const GAP = 6;

interface TooltipState {
  text: string;
  anchor: DOMRect;
}

let current: TooltipState | null = null;
let pending: ReturnType<typeof setTimeout> | undefined;
let lastHiddenAt = 0;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function snapshot(): TooltipState | null {
  return current;
}

export function showTooltip(text: string, anchor: DOMRect, immediate = false): void {
  clearTimeout(pending);
  const warm = current !== null || performance.now() - lastHiddenAt < WARM_WINDOW_MS;
  if (immediate || warm) {
    current = { text, anchor };
    emit();
    return;
  }
  pending = setTimeout(() => {
    current = { text, anchor };
    emit();
  }, SHOW_DELAY_MS);
}

export function hideTooltip(): void {
  clearTimeout(pending);
  if (current) {
    current = null;
    lastHiddenAt = performance.now();
    emit();
  }
}

export interface TooltipProps {
  onPointerEnter: (event: PointerEvent<HTMLElement>) => void;
  onPointerLeave: () => void;
  onPointerDown: () => void;
  onFocus: (event: FocusEvent<HTMLElement>) => void;
  onBlur: () => void;
}

const NO_TOOLTIP: Partial<TooltipProps> = {};

/** Event props that show `text` over the element; nothing when `text` is empty. */
export function tooltipProps(text: string | null | undefined): Partial<TooltipProps> {
  if (!text) return NO_TOOLTIP;
  return {
    onPointerEnter: (event) => {
      if (event.pointerType === "mouse" || event.pointerType === "pen") showTooltip(text, event.currentTarget.getBoundingClientRect());
    },
    onPointerLeave: hideTooltip,
    onPointerDown: hideTooltip,
    onFocus: (event) => {
      if (event.currentTarget.matches(":focus-visible")) showTooltip(text, event.currentTarget.getBoundingClientRect(), true);
    },
    onBlur: hideTooltip,
  };
}

interface Placement {
  left: number;
  top: number;
}

function place(anchor: DOMRect, width: number, height: number): Placement {
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const centered = anchor.left + anchor.width / 2 - width / 2;
  const left = Math.min(Math.max(centered, EDGE), Math.max(EDGE, viewportWidth - width - EDGE));
  const above = anchor.top - GAP - height;
  const below = anchor.bottom + GAP;
  const top = above >= EDGE || below + height > viewportHeight - EDGE ? Math.max(EDGE, above) : below;
  return { left, top };
}

export function TooltipLayer() {
  const state = useSyncExternalStore(subscribe, snapshot, snapshot);
  const ref = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<Placement | null>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!state || !element) {
      setPlacement(null);
      return;
    }
    setPlacement(place(state.anchor, element.offsetWidth, element.offsetHeight));
  }, [state]);

  useLayoutEffect(() => {
    if (!state) return;
    const dismiss = () => hideTooltip();
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("blur", dismiss);
    return () => {
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("blur", dismiss);
    };
  }, [state]);

  if (!state) return null;
  return (
    <div
      ref={ref}
      role="tooltip"
      className="uc-tooltip"
      style={placement ? { left: placement.left, top: placement.top } : { left: -9999, top: -9999 }}
    >
      {state.text}
    </div>
  );
}
