import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { FONT_FILES, copyFonts } from "./macos-widget.mjs";

let dir;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "qc-widget-"));
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

describe("the typeface the island and the widgets draw with", () => {
  it("copies the popup's Inter subsets and their license from the package the popup bundles", () => {
    copyFonts(join(dir, "Fonts"));
    expect(readdirSync(join(dir, "Fonts")).sort()).toEqual(["Inter-Latin.woff2", "Inter-LatinExt.woff2", "Inter-Vietnamese.woff2", "OFL.txt"]);
    const css = readFileSync(join(process.cwd(), "node_modules/@fontsource-variable/inter/index.css"), "utf8");
    for (const [source] of FONT_FILES) expect(css).toContain(`files/${source}`);
    expect(readFileSync(join(dir, "Fonts", "OFL.txt"), "utf8")).toContain("SIL Open Font License");
  });

  it("names every file the Swift side loads, Latin first", () => {
    const swift = readFileSync(join(process.cwd(), "src-tauri/macos/Shared/GlanceFont.swift"), "utf8");
    const subsets = /subsets = \[([^\]]*)\]/.exec(swift)?.[1].match(/"([^"]+)"/g)?.map((name) => `${name.slice(1, -1)}.woff2`);
    expect(subsets).toEqual(FONT_FILES.map(([, name]) => name));
  });
});
