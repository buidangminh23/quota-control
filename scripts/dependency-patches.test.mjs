import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const repoFile = (file) => join(root, file);
const read = (file) => readFileSync(repoFile(file), "utf8").replace(/\r\n/g, "\n");

function lockedPackages() {
  const [packages] = read("Cargo.lock").split("\n[[patch.unused]]");
  return packages
    .split("\n[[package]]\n")
    .slice(1)
    .map((block) => ({
      name: block.match(/^name = "(.+)"$/m)?.[1],
      version: block.match(/^version = "(.+)"$/m)?.[1],
      source: block.match(/^source = "(.+)"$/m)?.[1],
    }));
}

function atLeast(version, minimum) {
  const [have, want] = [version, minimum].map((text) => text.split(/[.+-]/).slice(0, 3).map(Number));
  const index = have.findIndex((part, position) => part !== want[position]);
  return index === -1 || have[index] > want[index];
}

describe("glib (GHSA-wrw7-89jp-8q8g)", () => {
  const glib = lockedPackages().filter((pkg) => pkg.name === "glib");

  it("never builds a crates.io glib that still has the unsound VariantStrIter", () => {
    const unsound = glib.filter((pkg) => pkg.source?.startsWith("registry+") && !atLeast(pkg.version, "0.20.0"));
    expect(unsound, "keep [patch.crates-io] glib pointing at third_party/glib while anything needs glib 0.18").toEqual([]);
  });

  it("builds the vendored glib only with the upstream fix and only while something needs it", () => {
    const vendored = glib.filter((pkg) => pkg.source === undefined);
    if (vendored.length === 0) {
      expect(existsSync(repoFile("third_party/glib")), "nothing needs glib 0.18 any more: delete third_party/glib and the [patch.crates-io] entry").toBe(false);
      return;
    }
    const source = read("third_party/glib/src/variant_iter.rs");
    const start = source.indexOf("fn impl_get");
    const body = source.slice(start, source.indexOf("CStr::from_ptr", start));
    expect(start).toBeGreaterThan(-1);
    expect(body).toContain("let mut p: *mut libc::c_char");
    expect(body).toContain("&mut p,");
    expect(body).not.toContain("&p,");
  });
});
