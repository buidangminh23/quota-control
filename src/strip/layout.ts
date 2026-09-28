/**
 * Pure geometry for the text strip's stacked readings, kept apart from the canvas so it can be
 * tested without one. All lengths are in the same unit (device pixels in the renderer); the glyph
 * measurements come from the canvas and are passed in.
 */

/** The font measurements the vertical placement works from, all positive distances. */
export interface RowFontMetrics {
  /** How far digits rise above the baseline: the ink a reading actually shows. */
  capAscent: number;
  /** How far the tallest glyph a row may carry (`l`, `h`, `k`) rises above the baseline. */
  ascent: number;
  /** How far the deepest glyph a row may carry (`g`, `y`, `ợ`) falls below the baseline. */
  descent: number;
}

/** How the rows are stacked: the space between rows and the clear edge kept inside the band. */
export interface RowSpacing {
  /** Space from one row's baseline to the top of the next row's tallest glyph. */
  rowGap: number;
  /** Space kept clear at the top and bottom edges of the band. */
  edge: number;
}

/**
 * Alphabetic baselines for `rows` readings stacked in a band `height` tall, top row first.
 *
 * The digits' ink (from the top row's cap height to the bottom row's baseline) is centered on the
 * band's middle, where the brand mark is centered too. The rows then move, as one block, just enough
 * that the tallest glyph and the deepest descender stay inside the band; when the band is too short
 * for both, the rows move closer together first, so nothing is cut off at either edge.
 */
export function stackBaselines(rows: number, height: number, font: RowFontMetrics, spacing: RowSpacing): number[] {
  const count = Math.max(1, Math.floor(rows));
  const room = height - 2 * spacing.edge - font.ascent - font.descent;
  const pitch = count > 1 ? Math.max(0, Math.min(font.ascent + spacing.rowGap, room / (count - 1))) : 0;
  const span = (count - 1) * pitch;
  let first = (height - font.capAscent - span) / 2 + font.capAscent;
  const overflowBottom = first + span + font.descent - (height - spacing.edge);
  if (overflowBottom > 0) first -= overflowBottom;
  const overflowTop = spacing.edge - (first - font.ascent);
  if (overflowTop > 0) first += overflowTop;
  return Array.from({ length: count }, (_, index) => first + index * pitch);
}

/** One reading's measured widths; `labelWidth` is `null` for a reading without a window name. */
export interface ColumnRow {
  labelWidth: number | null;
  valueWidth: number;
}

/**
 * Width of one provider's readings beside its mark: the window names left-aligned in one column,
 * the values right-aligned in the next, the same `labelGap` between the columns on every row. The
 * value column is never narrower than `minValueWidth`, so a reading growing from `6%` to `89%` keeps
 * the strip (and every menu bar item beside it) still. A reading without a window name may reach
 * back into the name column instead of leaving a hole there.
 */
export function groupTextWidth(rows: readonly ColumnRow[], labelGap: number, minValueWidth: number): number {
  const named = rows.filter((row) => row.labelWidth !== null);
  const unnamed = rows.filter((row) => row.labelWidth === null);
  const valueSource = named.length > 0 ? named : rows;
  const valueColumn = Math.max(minValueWidth, ...valueSource.map((row) => row.valueWidth));
  const nameColumn = named.length > 0 ? Math.max(...named.map((row) => row.labelWidth ?? 0)) + labelGap : 0;
  return Math.max(nameColumn + valueColumn, ...unnamed.map((row) => row.valueWidth));
}
