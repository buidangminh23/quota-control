/**
 * The global shortcut recorder's rules (upstream `KeyboardShortcuts.Recorder`): which key presses
 * make a combo and how a saved accelerator reads. Accelerators use the browser's
 * `KeyboardEvent.code` names (`Ctrl+Alt+KeyU`), which the core's shortcut parser also accepts.
 */

export interface KeyPress {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

export type RecordedKey =
  | { kind: "waiting" }
  | { kind: "cancel" }
  | { kind: "needsModifier" }
  | { kind: "unsupported" }
  | { kind: "combo"; accelerator: string };

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

const NAMED_KEYS: Readonly<Record<string, string>> = {
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "Page Up",
  PageDown: "Page Down",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

function isFunctionKey(code: string): boolean {
  return /^F([1-9]|1\d|2[0-4])$/.test(code);
}

function isRecordable(code: string): boolean {
  return /^Key[A-Z]$/.test(code) || /^(Digit|Numpad)\d$/.test(code) || isFunctionKey(code) || code in NAMED_KEYS;
}

/** What one key press means while the recorder listens. Modifiers alone keep it waiting. */
export function recordKey(press: KeyPress): RecordedKey {
  const modifiers = [press.ctrlKey && "Ctrl", press.altKey && "Alt", press.shiftKey && "Shift", press.metaKey && "Super"].filter(
    (modifier): modifier is string => typeof modifier === "string",
  );
  if (press.code === "Escape" && modifiers.length === 0) return { kind: "cancel" };
  if (!press.code || MODIFIER_CODES.has(press.code)) return { kind: "waiting" };
  if (!isRecordable(press.code)) return { kind: "unsupported" };
  if (modifiers.length === 0 && !isFunctionKey(press.code)) return { kind: "needsModifier" };
  return { kind: "combo", accelerator: [...modifiers, press.code].join("+") };
}

/** The keys of a saved accelerator as the user reads them: `Ctrl+Alt+KeyU` → Ctrl, Alt, U. */
export function shortcutKeys(accelerator: string, platform: "windows" | "linux" | "other"): string[] {
  return accelerator
    .split("+")
    .map((token) => token.trim())
    .filter(Boolean)
    .map((token) => keyLabel(token, platform));
}

function keyLabel(token: string, platform: "windows" | "linux" | "other"): string {
  switch (token.toLowerCase()) {
    case "ctrl":
    case "control":
    case "commandorcontrol":
    case "cmdorctrl":
      return "Ctrl";
    case "alt":
    case "option":
      return "Alt";
    case "shift":
      return "Shift";
    case "super":
    case "meta":
    case "cmd":
    case "command":
      return platform === "windows" ? "Win" : "Super";
  }
  if (/^key[a-z]$/i.test(token)) return token.slice(3).toUpperCase();
  if (/^digit\d$/i.test(token)) return token.slice(5);
  if (/^numpad\d$/i.test(token)) return `Num ${token.slice(6)}`;
  return NAMED_KEYS[token] ?? token;
}
