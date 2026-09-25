/**
 * The hover detail panel behind a value (model breakdown, usage trend, reset credits): upstream
 * `HoverPopoverState` + `motionAwareHoverPopover`. It opens after a short dwell over the trigger, stays
 * open while the pointer is over the trigger or the panel, and closes after a brief grace period so
 * the pointer can travel between the two. Only one panel is open at a time.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { hideTooltip } from "./tooltip";

const OPEN_DELAY_MS = 280;
const CLOSE_GRACE_MS = 140;
const EDGE = 6;
const GAP = 6;

let closeOpenPanel: (() => void) | null = null;

/** Close whichever hover panel is open (the popup hid, the screen changed, a menu opened). */
export function dismissHoverPopovers(): void {
  closeOpenPanel?.();
}

interface HoverPopoverProps {
  enabled: boolean;
  width: number;
  /** Rendered only while open. */
  panel: () => ReactNode;
  children: (state: { highlighted: boolean }) => ReactNode;
  className?: string;
  label?: string;
}

interface Position {
  left: number;
  top: number;
  maxHeight: number;
}

function place(anchor: DOMRect, width: number, height: number): Position {
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const centered = anchor.left + anchor.width / 2 - width / 2;
  const left = Math.min(Math.max(centered, EDGE), Math.max(EDGE, viewportWidth - width - EDGE));
  const below = viewportHeight - anchor.bottom - GAP - EDGE;
  const above = anchor.top - GAP - EDGE;
  const goBelow = below >= height || below >= above;
  const available = Math.max(60, goBelow ? below : above);
  const shown = Math.min(height, available);
  const top = goBelow ? anchor.bottom + GAP : anchor.top - GAP - shown;
  return { left, top: Math.max(EDGE, top), maxHeight: available };
}

export function HoverPopover({ enabled, width, panel, children, className, label }: HoverPopoverProps) {
  const triggerRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [pos, setPos] = useState<Position | null>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const closeTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const close = useCallback(() => {
    clearTimeout(openTimer.current);
    clearTimeout(closeTimer.current);
    setOpen(false);
    setHovered(false);
    setPos(null);
  }, []);

  useEffect(() => {
    if (!open) return;
    if (closeOpenPanel && closeOpenPanel !== close) closeOpenPanel();
    closeOpenPanel = close;
    return () => {
      if (closeOpenPanel === close) closeOpenPanel = null;
    };
  }, [open, close]);

  useEffect(() => () => close(), [close]);

  useEffect(() => {
    if (!enabled) close();
  }, [enabled, close]);

  useLayoutEffect(() => {
    if (!open || !triggerRef.current || !panelRef.current) return;
    setPos(place(triggerRef.current.getBoundingClientRect(), width, panelRef.current.scrollHeight));
  }, [open, width]);

  const enter = () => {
    if (!enabled) return;
    setHovered(true);
    clearTimeout(closeTimer.current);
    if (!open) {
      clearTimeout(openTimer.current);
      openTimer.current = setTimeout(() => {
        hideTooltip();
        setOpen(true);
      }, OPEN_DELAY_MS);
    }
  };

  const leave = () => {
    clearTimeout(openTimer.current);
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(close, CLOSE_GRACE_MS);
    if (!open) setHovered(false);
  };

  return (
    <>
      <div
        ref={triggerRef}
        className={className}
        aria-label={label}
        onPointerEnter={enter}
        onPointerLeave={leave}
        onFocus={() => enabled && setOpen(true)}
        onBlur={leave}
        tabIndex={enabled ? 0 : undefined}
      >
        {children({ highlighted: enabled && (hovered || open) })}
      </div>
      {open
        ? createPortal(
            <div
              ref={panelRef}
              className="uc-popover"
              role="dialog"
              style={{
                width,
                left: pos?.left ?? -9999,
                top: pos?.top ?? -9999,
                maxHeight: pos?.maxHeight,
              }}
              onPointerEnter={() => clearTimeout(closeTimer.current)}
              onPointerLeave={leave}
            >
              {panel()}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}
