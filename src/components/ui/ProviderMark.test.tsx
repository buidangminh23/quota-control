import { render } from "@testing-library/react";
import { colorArtUrl, PROVIDER_COLOR_ART } from "@/assets/providerColorArt";
import { ProviderMark } from "./ProviderMark";

describe("provider marks", () => {
  it("shows the official color logo for a brand whose logo is several colors", () => {
    const { container } = render(<ProviderMark brand="antigravity" size={18} />);
    const image = container.querySelector("img.uc-mark");
    expect(image).not.toBeNull();
    expect(image!.getAttribute("src")).toBe(colorArtUrl(PROVIDER_COLOR_ART.antigravity!, 0.04));
    expect(decodeURIComponent(image!.getAttribute("src")!)).toContain("#FC413D");
    expect(container.querySelector("svg")).toBeNull();
  });

  it("draws a single-color brand's mark in its tint", () => {
    const { container } = render(<ProviderMark brand="claude" size={18} />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("svg.uc-mark")?.getAttribute("fill")).toBe("#DE7356");
  });

  it("centers color art in a square with the same inset as the marks", () => {
    const url = decodeURIComponent(colorArtUrl({ box: [0, 7, 24, 11], body: "<path/>" }, 0.04));
    expect(url).toContain('viewBox="-0.96 -0.46 25.92 25.92"');
    expect(url.endsWith("<path/></svg>")).toBe(true);
  });
});
