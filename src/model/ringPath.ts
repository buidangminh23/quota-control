/**
 * SVG path for one Total Spend ring slice: an annular wedge with a hairline gap between slices and
 * softly rounded corners. Port of upstream `Views/RingSectorShape.swift` (same angles, gap and corner
 * clamping), expressed as SVG arc commands. Angles run clockwise from 12 o'clock in y-down space.
 */

export interface RingGeometry {
  size: number;
  innerRadiusRatio?: number;
  gapWidth?: number;
  cornerRadius?: number;
}

const INNER_RADIUS_RATIO = 0.618;
const GAP_WIDTH = 1.6;
const CORNER_RADIUS = 3;

function point(cx: number, cy: number, radius: number, angle: number): [number, number] {
  return [cx + radius * Math.cos(angle), cy + radius * Math.sin(angle)];
}

function fmt([x, y]: [number, number]): string {
  return `${x.toFixed(3)} ${y.toFixed(3)}`;
}

function arc(radius: number, sweep: number, clockwise: boolean, to: [number, number]): string {
  const large = Math.abs(sweep) > Math.PI ? 1 : 0;
  return `A ${radius.toFixed(3)} ${radius.toFixed(3)} 0 ${large} ${clockwise ? 1 : 0} ${fmt(to)}`;
}

/** The slice from `startFraction` to `endFraction` (0...1 around the ring), or `""` when too thin. */
export function ringSectorPath(startFraction: number, endFraction: number, geometry: RingGeometry): string {
  const outer = geometry.size / 2;
  const inner = outer * (geometry.innerRadiusRatio ?? INNER_RADIUS_RATIO);
  const cx = outer;
  const cy = outer;
  const top = -Math.PI / 2;
  const halfGap = (geometry.gapWidth ?? GAP_WIDTH) / outer / 2;
  const a0 = top + startFraction * 2 * Math.PI + halfGap;
  const a1 = top + endFraction * 2 * Math.PI - halfGap;
  const width = a1 - a0;
  if (!(width > 0.001)) return "";

  const s = Math.sin(Math.min(width / 2, Math.PI / 2));
  let corner = Math.min(geometry.cornerRadius ?? CORNER_RADIUS, (outer - inner) / 2);
  corner = Math.min(corner, (outer * s) / (1 + s));
  if (s < 1) corner = Math.min(corner, (inner * s) / (1 - s));

  if (corner < 0.25) {
    return [
      `M ${fmt(point(cx, cy, outer, a0))}`,
      arc(outer, width, true, point(cx, cy, outer, a1)),
      `L ${fmt(point(cx, cy, inner, a1))}`,
      arc(inner, width, false, point(cx, cy, inner, a0)),
      "Z",
    ].join(" ");
  }

  const betaOuter = Math.asin(Math.min(1, corner / (outer - corner)));
  const betaInner = Math.asin(Math.min(1, corner / (inner + corner)));
  const around = (center: [number, number], angle: number): [number, number] => [
    center[0] + corner * Math.cos(angle),
    center[1] + corner * Math.sin(angle),
  ];
  const trailingOuter = point(cx, cy, outer - corner, a1 - betaOuter);
  const trailingInner = point(cx, cy, inner + corner, a1 - betaInner);
  const leadingInner = point(cx, cy, inner + corner, a0 + betaInner);
  const leadingOuter = point(cx, cy, outer - corner, a0 + betaOuter);
  const outerSweep = a1 - betaOuter - (a0 + betaOuter);
  const innerSweep = a1 - betaInner - (a0 + betaInner);

  return [
    `M ${fmt(point(cx, cy, outer, a0 + betaOuter))}`,
    arc(outer, outerSweep, true, point(cx, cy, outer, a1 - betaOuter)),
    arc(corner, Math.PI / 2, true, around(trailingOuter, a1 + Math.PI / 2)),
    `L ${fmt(around(trailingInner, a1 + Math.PI / 2))}`,
    arc(corner, Math.PI / 2, true, around(trailingInner, a1 - betaInner + Math.PI)),
    arc(inner, innerSweep, false, point(cx, cy, inner, a0 + betaInner)),
    arc(corner, Math.PI / 2, true, around(leadingInner, a0 + (3 * Math.PI) / 2)),
    `L ${fmt(around(leadingOuter, a0 - Math.PI / 2))}`,
    arc(corner, Math.PI / 2, true, point(cx, cy, outer, a0 + betaOuter)),
    "Z",
  ].join(" ");
}
