/**
 * A provider's mark (upstream `ProviderIcon`): the artwork is centered in a square box with a small
 * inset so every brand fills its glyph box the same way. It is drawn in the brand's own color, the one
 * the Token ring gives it, so Claude's mark is orange and Codex's green; a brand without one takes
 * `currentColor`.
 */
import { PROVIDER_MARKS } from "@/assets/providerMarks";
import { knownBrandColor } from "@/model/totalSpend";
import { useIsDark } from "@/state/hooks";

const DEFAULT_INSET = 0.04;

export function ProviderMark({ brand, size, inset = DEFAULT_INSET }: { brand: string; size: number; inset?: number }) {
  const dark = useIsDark();
  const mark = PROVIDER_MARKS[brand];
  if (!mark) {
    return (
      <span className="uc-mark-fallback" style={{ width: size, height: size, fontSize: Math.round(size * 0.62) }} aria-hidden="true">
        {brand.charAt(0).toUpperCase()}
      </span>
    );
  }
  const [minX, minY, width, height] = mark.box;
  const side = Math.max(width, height);
  const pad = side * inset;
  const viewBox = [minX - (side - width) / 2 - pad, minY - (side - height) / 2 - pad, side + pad * 2, side + pad * 2].join(" ");
  return (
    <svg className="uc-mark" width={size} height={size} viewBox={viewBox} fill={knownBrandColor(brand, dark) ?? "currentColor"} aria-hidden="true" focusable="false">
      {mark.paths.map((path, index) => (
        <path key={index} d={path.d} fillRule={path.evenOdd ? "evenodd" : undefined} clipRule={path.evenOdd ? "evenodd" : undefined} />
      ))}
    </svg>
  );
}
