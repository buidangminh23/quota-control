/**
 * The Usage Trend row (upstream `UsageSparkline`): the label and a strip of day bars; hovering the
 * strip opens the larger day-by-day chart (`UsageTrendDetail`).
 */
import { useState } from "react";
import { messagesFor, translate, type Language } from "@/i18n";
import type { MetricChartPoint } from "@/lib/types";
import { chartDayLabel, chartMax, detailBarHeight, peakIndex, pointReadout, sparklineBarHeight } from "@/model/chart";
import type { WidgetData } from "@/model/widgetData";
import { HoverPopover } from "../ui/hoverPopover";

const TREND_DETAIL_WIDTH = 240;
const DETAIL_CHART_HEIGHT = 76;

function trendHeight(): number {
  const value = getComputedStyle(document.documentElement).getPropertyValue("--uc-trend-height");
  return Number.parseFloat(value) || 18;
}

export function Sparkline({ data, interactive }: { data: WidgetData; interactive: boolean }) {
  const points = data.chartPoints;
  const language = data.language;
  const max = chartMax(points);
  const height = trendHeight();
  const peak = peakIndex(points);
  const first = points[0];
  const last = points.at(-1);
  const dashboard = messagesFor(language).dashboard;
  const label =
    first && last && peak !== null
      ? `${data.title}: ${dashboard.trendRange(points.length, chartDayLabel(first.label, language), chartDayLabel(last.label, language))}, ${dashboard.peak(pointReadout(points[peak]!, language))}`
      : data.title;
  return (
    <div className="uc-sparkline-row">
      <span className="uc-row-text-title">{data.title}</span>
      <HoverPopover
        enabled={interactive}
        width={TREND_DETAIL_WIDTH}
        className="uc-sparkline-slot"
        label={label}
        panel={() => <TrendDetail title={data.title} points={points} note={data.chartNote} language={language} />}
      >
        {({ highlighted }) => (
          <span className={`uc-sparkline${highlighted ? " is-highlighted" : ""}`} role="img" aria-label={label}>
            {points.map((point, index) => (
              <span key={`${point.label}-${index}`} className="uc-sparkline-bar" style={{ height: sparklineBarHeight(point.value, max, height) }} />
            ))}
          </span>
        )}
      </HoverPopover>
    </div>
  );
}

function TrendDetail({ title, points, note, language }: { title: string; points: MetricChartPoint[]; note?: string; language: Language }) {
  const [active, setActive] = useState<number | null>(null);
  const max = chartMax(points);
  const peak = peakIndex(points);
  const dashboard = messagesFor(language).dashboard;
  const activePoint = active === null ? undefined : points[active];
  const readout = activePoint
    ? `${chartDayLabel(activePoint.label, language)} · ${pointReadout(activePoint, language)}`
    : peak !== null
      ? dashboard.peak(pointReadout(points[peak]!, language))
      : "";
  return (
    <div className="uc-detail" onPointerLeave={() => setActive(null)}>
      <div className="uc-detail-header">
        <span className="uc-detail-title">{title}</span>
        <span className="uc-detail-readout uc-num">{readout}</span>
      </div>
      <div className="uc-trend-chart" style={{ height: DETAIL_CHART_HEIGHT }}>
        {points.map((point, index) => (
          <span key={`${point.label}-${index}`} className="uc-trend-column" onPointerEnter={() => setActive(index)}>
            <span
              className="uc-trend-bar"
              style={{
                height: detailBarHeight(point.value, max, DETAIL_CHART_HEIGHT),
                opacity: active === null || active === index ? 1 : 0.35,
              }}
            />
          </span>
        ))}
      </div>
      <div className="uc-trend-axis uc-num">
        <span>{points[0] ? chartDayLabel(points[0].label, language) : ""}</span>
        <span>{points.at(-1) ? chartDayLabel(points.at(-1)!.label, language) : ""}</span>
      </div>
      {note ? <p className="uc-source-note">{translate(note, language)}</p> : null}
    </div>
  );
}
