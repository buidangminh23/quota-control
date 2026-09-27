import { act, cleanup, render } from "@testing-library/react";
import { useApp } from "@/state/store";
import { PROVIDER_MARKS } from "@/assets/providerMarks";
import { ProviderMark } from "./ProviderMark";

function setTheme(theme: "light" | "dark" | "system"): void {
  act(() => useApp.setState({ settings: { ...useApp.getState().settings, theme } }));
}

function fill(brand: string): string | null {
  const { container } = render(<ProviderMark brand={brand} size={16} />);
  const value = container.querySelector("svg")?.getAttribute("fill") ?? null;
  cleanup();
  return value;
}

afterEach(() => {
  cleanup();
  setTheme("system");
});

describe("ProviderMark", () => {
  it("draws Claude and Codex in their own colors in both appearances", () => {
    setTheme("light");
    expect(fill("claude")).toBe("#DE7356");
    expect(fill("codex")).toBe("#10A37F");
    setTheme("dark");
    expect(fill("claude")).toBe("#DE7356");
    expect(fill("codex")).toBe("#10A37F");
  });

  it("switches a black-and-white brand with the appearance", () => {
    setTheme("light");
    expect(fill("cursor")).toBe("#13120A");
    setTheme("dark");
    expect(fill("cursor")).toBe("#F5F5F7");
  });

  it("draws every brand that has a mark in its own color", () => {
    setTheme("light");
    const uncolored = Object.keys(PROVIDER_MARKS).filter((brand) => fill(brand) === "currentColor");
    expect(uncolored).toEqual([]);
  });
});
