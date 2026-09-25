/**
 * Pop-up menus drawn inside the popup: the Options menu, context menus and picker lists. They stay
 * in the webview on purpose, so opening one never takes focus from the popup (the core hides the
 * popup when it loses focus) and so they follow the popup's own theme. A submenu drills down in place
 * because the 320px window has no room for a side flyout.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { CheckIcon, ChevronLeft, ChevronRight } from "./icons";
import { hideTooltip } from "./tooltip";

export type MenuEntry =
  | {
      kind: "item";
      label: string;
      onSelect: () => void;
      checked?: boolean;
      disabled?: boolean;
      destructive?: boolean;
      shortcut?: string;
    }
  | { kind: "separator" }
  | { kind: "submenu"; label: string; entries: MenuEntry[]; disabled?: boolean };

export interface MenuRequest {
  entries: MenuEntry[];
  /** The element or point the menu hangs from. */
  anchor: DOMRect | { x: number; y: number };
  /** Preferred side; the menu flips when that side has no room. */
  placement?: "below" | "above";
  align?: "start" | "end";
  /** Shows checkmark gutters even when no entry is checked (picker lists). */
  checkable?: boolean;
  onClose?: () => void;
}

const EDGE = 6;
const GAP = 4;

let request: MenuRequest | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function snapshot(): MenuRequest | null {
  return request;
}

export function openMenu(next: MenuRequest): void {
  hideTooltip();
  request?.onClose?.();
  request = next;
  emit();
}

export function closeMenu(): void {
  if (!request) return;
  const closing = request;
  request = null;
  emit();
  closing.onClose?.();
}

export function isMenuOpen(): boolean {
  return request !== null;
}

/** Open a menu under (or over) `element`, right-aligned by default like upstream's footer menus. */
export function openMenuAt(element: HTMLElement, entries: MenuEntry[], options: Partial<MenuRequest> = {}): void {
  openMenu({ entries, anchor: element.getBoundingClientRect(), ...options });
}

function anchorRect(anchor: MenuRequest["anchor"]): { left: number; right: number; top: number; bottom: number } {
  if ("x" in anchor && !("width" in anchor)) return { left: anchor.x, right: anchor.x, top: anchor.y, bottom: anchor.y };
  const rect = anchor as DOMRect;
  return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
}

interface Position {
  left: number;
  top: number;
  maxHeight: number;
}

function position(req: MenuRequest, width: number, height: number): Position {
  const rect = anchorRect(req.anchor);
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const preferredLeft = req.align === "end" ? rect.right - width : rect.left;
  const left = Math.min(Math.max(preferredLeft, EDGE), Math.max(EDGE, viewportWidth - width - EDGE));
  const spaceBelow = viewportHeight - rect.bottom - GAP - EDGE;
  const spaceAbove = rect.top - GAP - EDGE;
  const wantsAbove = req.placement === "above";
  const goAbove = wantsAbove ? spaceAbove >= height || spaceAbove > spaceBelow : spaceBelow < height && spaceAbove > spaceBelow;
  const available = Math.max(80, goAbove ? spaceAbove : spaceBelow);
  const shown = Math.min(height, available);
  const top = goAbove ? rect.top - GAP - shown : rect.bottom + GAP;
  return { left, top: Math.max(EDGE, top), maxHeight: available };
}

function selectable(entry: MenuEntry): boolean {
  return entry.kind !== "separator" && !entry.disabled;
}

export function MenuLayer() {
  const req = useSyncExternalStore(subscribe, snapshot, snapshot);
  if (!req) return null;
  return <MenuPanel request={req} />;
}

function MenuPanel({ request: req }: { request: MenuRequest }) {
  const [stack, setStack] = useState<{ label: string; entries: MenuEntry[] }[]>([]);
  const [active, setActive] = useState(-1);
  const [pos, setPos] = useState<Position | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const level = stack.at(-1);
  const entries = level?.entries ?? req.entries;
  const checkable = req.checkable || entries.some((entry) => entry.kind === "item" && entry.checked !== undefined);

  useLayoutEffect(() => {
    const element = ref.current;
    if (element) setPos(position(req, element.offsetWidth, element.scrollHeight));
  }, [req, stack]);

  useEffect(() => {
    ref.current?.focus();
    setActive(-1);
  }, [stack]);

  const activate = useCallback(
    (entry: MenuEntry) => {
      if (entry.kind === "separator" || entry.disabled) return;
      if (entry.kind === "submenu") {
        setStack((levels) => [...levels, { label: entry.label, entries: entry.entries }]);
        return;
      }
      closeMenu();
      entry.onSelect();
    },
    [],
  );

  const back = useCallback(() => {
    if (stack.length > 0) setStack((levels) => levels.slice(0, -1));
    else closeMenu();
  }, [stack.length]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const move = (step: number) => {
        const count = entries.length;
        for (let offset = 1; offset <= count; offset += 1) {
          const index = (active + step * offset + count * 2) % count;
          if (selectable(entries[index]!)) {
            setActive(index);
            return;
          }
        }
      };
      switch (event.key) {
        case "Escape":
          back();
          break;
        case "ArrowDown":
          move(1);
          break;
        case "ArrowUp":
          move(-1);
          break;
        case "ArrowLeft":
          if (stack.length > 0) back();
          break;
        case "ArrowRight": {
          const entry = entries[active];
          if (entry?.kind === "submenu") activate(entry);
          break;
        }
        case "Enter":
        case " ": {
          const entry = entries[active];
          if (entry) activate(entry);
          break;
        }
        case "Tab":
          closeMenu();
          return;
        default:
          return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    window.addEventListener("keydown", onKey, true);
    const onBlur = () => closeMenu();
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onBlur);
    };
  }, [active, activate, back, entries, stack.length]);

  return (
    <div className="uc-menu-backdrop" onPointerDown={(event) => event.target === event.currentTarget && closeMenu()} onContextMenu={(event) => event.preventDefault()}>
      <div
        ref={ref}
        role="menu"
        tabIndex={-1}
        className="uc-menu"
        style={pos ? { left: pos.left, top: pos.top, maxHeight: pos.maxHeight } : { left: -9999, top: -9999 }}
      >
        {level ? (
          <button type="button" role="menuitem" className="uc-menu-item uc-menu-back" onClick={back}>
            <span className="uc-menu-check">
              <ChevronLeft size={10} />
            </span>
            <span className="uc-menu-label">{level.label}</span>
          </button>
        ) : null}
        {level ? <div className="uc-menu-separator" role="separator" /> : null}
        {entries.map((entry, index) =>
          entry.kind === "separator" ? (
            <div key={`separator-${index}`} className="uc-menu-separator" role="separator" />
          ) : (
            <MenuRow
              key={`${entry.kind}-${entry.label}-${index}`}
              entry={entry}
              active={index === active}
              checkable={checkable}
              onHover={() => setActive(selectable(entry) ? index : -1)}
              onActivate={() => activate(entry)}
            />
          ),
        )}
      </div>
    </div>
  );
}

function MenuRow({
  entry,
  active,
  checkable,
  onHover,
  onActivate,
}: {
  entry: Exclude<MenuEntry, { kind: "separator" }>;
  active: boolean;
  checkable: boolean;
  onHover: () => void;
  onActivate: () => void;
}) {
  const isItem = entry.kind === "item";
  const checked = isItem && entry.checked === true;
  const className = [
    "uc-menu-item",
    active ? "is-active" : "",
    isItem && entry.destructive ? "is-destructive" : "",
  ].join(" ");
  let trailing: ReactNode = null;
  if (entry.kind === "submenu") trailing = <ChevronRight size={10} />;
  else if (entry.shortcut) trailing = <span className="uc-menu-shortcut">{entry.shortcut}</span>;
  return (
    <button
      type="button"
      role={isItem && entry.checked !== undefined ? "menuitemcheckbox" : "menuitem"}
      aria-checked={isItem && entry.checked !== undefined ? checked : undefined}
      aria-haspopup={entry.kind === "submenu" ? "menu" : undefined}
      disabled={entry.disabled}
      className={className}
      onPointerMove={onHover}
      onClick={onActivate}
    >
      {checkable ? <span className="uc-menu-check">{checked ? <CheckIcon size={11} /> : null}</span> : null}
      <span className="uc-menu-label">{entry.label}</span>
      {trailing ? <span className="uc-menu-trailing">{trailing}</span> : null}
    </button>
  );
}
