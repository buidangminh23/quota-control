import { describe, expect, it } from "vitest";
import { recordKey, shortcutKeys, type KeyPress } from "./shortcut";

function press(code: string, modifiers: Partial<Omit<KeyPress, "code">> = {}): KeyPress {
  return { code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...modifiers };
}

describe("recordKey", () => {
  it("records modifiers in a fixed order with the physical key code", () => {
    expect(recordKey(press("KeyU", { shiftKey: true, ctrlKey: true, altKey: true }))).toEqual({ kind: "combo", accelerator: "Ctrl+Alt+Shift+KeyU" });
    expect(recordKey(press("Digit5", { metaKey: true }))).toEqual({ kind: "combo", accelerator: "Super+Digit5" });
    expect(recordKey(press("Space", { altKey: true }))).toEqual({ kind: "combo", accelerator: "Alt+Space" });
  });

  it("allows a bare function key but asks for a modifier on anything else", () => {
    expect(recordKey(press("F9"))).toEqual({ kind: "combo", accelerator: "F9" });
    expect(recordKey(press("KeyU"))).toEqual({ kind: "needsModifier" });
  });

  it("keeps waiting on modifiers, cancels on a bare Escape and rejects unknown keys", () => {
    expect(recordKey(press("ControlLeft", { ctrlKey: true }))).toEqual({ kind: "waiting" });
    expect(recordKey(press(""))).toEqual({ kind: "waiting" });
    expect(recordKey(press("Escape"))).toEqual({ kind: "cancel" });
    expect(recordKey(press("Escape", { ctrlKey: true }))).toEqual({ kind: "unsupported" });
    expect(recordKey(press("MediaPlayPause", { ctrlKey: true }))).toEqual({ kind: "unsupported" });
  });
});

describe("shortcutKeys", () => {
  it("reads accelerators the way each platform names its keys", () => {
    expect(shortcutKeys("Ctrl+Alt+KeyU", "windows")).toEqual(["Ctrl", "Alt", "U"]);
    expect(shortcutKeys("Super+Shift+Digit5", "windows")).toEqual(["Win", "Shift", "5"]);
    expect(shortcutKeys("Super+Shift+Digit5", "linux")).toEqual(["Super", "Shift", "5"]);
    expect(shortcutKeys("Ctrl+Numpad3", "linux")).toEqual(["Ctrl", "Num 3"]);
    expect(shortcutKeys("Alt+Backquote", "other")).toEqual(["Alt", "`"]);
    expect(shortcutKeys("F12", "windows")).toEqual(["F12"]);
  });
});
