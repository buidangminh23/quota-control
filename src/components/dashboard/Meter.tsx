/**
 * The full-width capsule meter (upstream `WidgetRowView.meter`): a quaternary track, a flat system
 * color fill carrying the pace verdict, and an even-pace tick that pokes out above and below the bar.
 * Any non-zero fill is at least one bar-height wide so 1-2% never disappears.
 */
import { meterSeverity, meterTooltip, paceTick, type MeterState } from "@/model/meterState";
import { fraction, type WidgetData } from "@/model/widgetData";
import { tooltipProps } from "../ui/tooltip";

const SEVERITY_COLOR = {
  normal: "var(--uc-blue)",
  warning: "var(--uc-yellow)",
  critical: "var(--uc-red)",
} as const;

export function Meter({ data, state, now }: { data: WidgetData; state: MeterState; now: Date }) {
  const severity = meterSeverity(state);
  const fill = data.hasData ? fraction(data) : 0;
  const tick = paceTick(data, state, now);
  return (
    <div className="uc-meter" aria-hidden="true" {...tooltipProps(meterTooltip(state, data.language))}>
      {fill > 0 ? (
        <div
          className="uc-meter-fill"
          style={{ width: `${fill * 100}%`, background: severity ? SEVERITY_COLOR[severity] : "var(--uc-secondary)" }}
        />
      ) : null}
      {tick !== null ? <div className="uc-meter-tick" style={{ left: `clamp(0px, calc(${tick * 100}% - 1px), calc(100% - 2px))` }} /> : null}
    </div>
  );
}
