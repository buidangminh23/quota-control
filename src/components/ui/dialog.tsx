/**
 * In-popup alerts (upstream `.alert`): a scrim over the popup and a small card with a title, a
 * message and its actions. Escape and the scrim cancel; Cancel starts focused so Enter never
 * confirms a destructive action by accident.
 */
import { useEffect, useRef, useSyncExternalStore, type ReactNode } from "react";
import { closeMenu } from "./menu";
import { hideTooltip } from "./tooltip";

export interface DialogAction {
  label: string;
  role?: "cancel" | "destructive" | "default";
  onSelect?: () => void;
}

export interface DialogRequest {
  title: string;
  message?: ReactNode;
  actions: DialogAction[];
}

let request: DialogRequest | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function snapshot(): DialogRequest | null {
  return request;
}

export function openDialog(next: DialogRequest): void {
  hideTooltip();
  closeMenu();
  request = next;
  emit();
}

export function closeDialog(): void {
  if (!request) return;
  request = null;
  emit();
}

export function isDialogOpen(): boolean {
  return request !== null;
}

/** Ask to confirm a destructive action; resolves `true` only when the user confirms. */
export function confirmAction(options: { title: string; message: string; confirmLabel: string; cancelLabel: string }): Promise<boolean> {
  return new Promise((resolve) => {
    openDialog({
      title: options.title,
      message: options.message,
      actions: [
        { label: options.cancelLabel, role: "cancel", onSelect: () => resolve(false) },
        { label: options.confirmLabel, role: "destructive", onSelect: () => resolve(true) },
      ],
    });
  });
}

export function DialogLayer() {
  const req = useSyncExternalStore(subscribe, snapshot, snapshot);
  if (!req) return null;
  return <DialogCard request={req} />;
}

function DialogCard({ request: req }: { request: DialogRequest }) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const cancel = req.actions.find((action) => action.role === "cancel");

  const choose = (action: DialogAction | undefined) => {
    closeDialog();
    action?.onSelect?.();
  };

  useEffect(() => {
    cancelRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      choose(cancel);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return (
    <div className="uc-dialog-scrim" onPointerDown={(event) => event.target === event.currentTarget && choose(cancel)}>
      <div className="uc-dialog" role="alertdialog" aria-modal="true" aria-labelledby="uc-dialog-title">
        <h2 id="uc-dialog-title" className="uc-dialog-title">
          {req.title}
        </h2>
        {req.message ? <div className="uc-dialog-message">{req.message}</div> : null}
        <div className={`uc-dialog-actions${req.actions.length > 2 ? " is-stacked" : ""}`}>
          {req.actions.map((action) => (
            <button
              key={action.label}
              ref={action.role === "cancel" ? cancelRef : undefined}
              type="button"
              className={`uc-button ${action.role === "destructive" ? "is-destructive" : action.role === "default" ? "is-prominent" : "is-bordered"}`}
              onClick={() => choose(action)}
            >
              {action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
