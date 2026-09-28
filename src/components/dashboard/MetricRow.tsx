/**
 * One metric inside a provider card (upstream `WidgetRowView`). A bounded metric is a label line with
 * the pace warning, a capsule meter, a headline/reset line and, under a countdown, the exact time the
 * limit comes back; an unbounded one is a single line with the value right-aligned, which reveals a
 * hover detail when there is more to show.
 */
import type { Language } from "@/i18n";
import { messagesFor } from "@/i18n";
import { meterState, meterTooltip, meterSeverity, spareText, boundedTrailingText, hasResetLabel, resetTooltip, restoreText, hasMeterStyleToggle, meterStyleTooltip, type MeterState } from "@/model/meterState";
import {
  expirySeverity,
  hasModelBreakdown,
  hasUnknownModels,
  headline,
  unboundedDetail,
  unboundedSubtitle,
  unboundedValueTooltip,
  unknownModelTooltip,
  type WidgetData,
} from "@/model/widgetData";
import { updateSettings } from "@/state/store";
import { HoverPopover } from "../ui/hoverPopover";
import { FlameIcon, WarningTriangle } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";
import { ModelBreakdownDetail, ResetsDetail } from "./details";
import { Meter } from "./Meter";
import { Sparkline } from "./Sparkline";

const DETAIL_WIDTH = 280;

interface MetricRowProps {
  data: WidgetData;
  now: Date;
  condensedTop: boolean;
  interactive?: boolean;
}

export function MetricRow({ data, now, condensedTop, interactive = true }: MetricRowProps) {
  if (data.isChart && data.hasData) {
    return (
      <div className="uc-row is-text">
        <Sparkline data={data} interactive={interactive} />
      </div>
    );
  }
  if (data.limit !== null) return <BoundedRow data={data} now={now} interactive={interactive} />;
  return <UnboundedRow data={data} now={now} condensedTop={condensedTop} interactive={interactive} />;
}

function toggleResetDisplay(data: WidgetData): void {
  updateSettings({ resetDisplayMode: data.resetDisplayMode === "relative" ? "absolute" : "relative" });
}

function toggleMeterStyle(data: WidgetData): void {
  updateSettings({ displayMode: data.displayMode === "remaining" ? "used" : "remaining" });
}

function BoundedRow({ data, now, interactive }: { data: WidgetData; now: Date; interactive: boolean }) {
  const state = meterState(data, now);
  const language = data.language;
  const trailing = boundedTrailingText(data, now);
  const restore = restoreText(data, now);
  const resetToggle = interactive && hasResetLabel(data, now);
  const styleToggle = interactive && hasMeterStyleToggle(data);
  return (
    <div className="uc-row is-bounded">
      <div className="uc-row-label">
        <span className="uc-row-title uc-truncate">{data.title}</span>
        <PaceWarning data={data} state={state} language={language} />
      </div>
      <Meter data={data} state={state} now={now} />
      <div className="uc-row-readout">
        <div className="uc-row-primary">
          {styleToggle ? (
            <button type="button" className="uc-row-headline uc-num" onClick={() => toggleMeterStyle(data)} {...tooltipProps(meterStyleTooltip(data))}>
              {headline(data)}
            </button>
          ) : (
            <span className="uc-row-headline uc-num">{headline(data)}</span>
          )}
          {trailing ? (
            resetToggle ? (
              <button type="button" className="uc-row-trailing uc-num" onClick={() => toggleResetDisplay(data)} {...tooltipProps(resetTooltip(data, now))}>
                {trailing}
              </button>
            ) : (
              <span className="uc-row-trailing uc-num" {...tooltipProps(resetTooltip(data, now))}>
                {trailing}
              </span>
            )
          ) : null}
        </div>
        {restore ? <div className="uc-row-restore uc-num">{restore}</div> : null}
      </div>
    </div>
  );
}

function PaceWarning({ data, state, language }: { data: WidgetData; state: MeterState; language: Language }) {
  const tooltip = meterTooltip(state, language);
  const severity = meterSeverity(state);
  const flameColor = severity ? `var(--uc-${severity === "critical" ? "red" : severity === "warning" ? "yellow" : "blue"})` : undefined;
  switch (state.kind) {
    case "spent":
      return (
        <span className="uc-row-warning" {...tooltipProps(tooltip)}>
          <FlameIcon size={11} style={{ color: flameColor }} />
          <span>{messagesFor(language).meter.limitReached}</span>
        </span>
      );
    case "runningOut":
      return null;
    case "closeToLimit":
      return (
        <span className="uc-row-warning uc-num" {...tooltipProps(tooltip)}>
          {spareText(state, language)}
        </span>
      );
    case "healthy":
      return data.alwaysShowPacing && tooltip ? <span className="uc-row-warning uc-num">{tooltip}</span> : null;
    default:
      return null;
  }
}

function ExpiryDot({ data, now }: { data: WidgetData; now: Date }) {
  const severity = expirySeverity(data, now);
  if (!severity) return null;
  const color = severity === "critical" ? "var(--uc-red)" : severity === "warning" ? "var(--uc-yellow)" : "var(--uc-blue)";
  return <span className="uc-status-dot" style={{ background: color }} aria-label={messagesFor(data.language).dashboard.expiryStatus(severity)} />;
}

function UnboundedRow({ data, now, condensedTop, interactive }: { data: WidgetData; now: Date; condensedTop: boolean; interactive: boolean }) {
  const breakdown = hasModelBreakdown(data);
  const resets = data.showsResetExpiries && data.hasData;
  const hasDetail = interactive && (breakdown || resets);
  const subtitle = unboundedSubtitle(data);
  return (
    <div className={`uc-row is-text${condensedTop ? " is-condensed" : ""}`}>
      <span className="uc-row-text-label">
        <span className="uc-row-text-title">{data.title}</span>
        {hasUnknownModels(data) ? (
          <span className="uc-inline-icon" style={{ color: "var(--uc-yellow)" }} aria-label={messagesFor(data.language).dashboard.unknownPricingWarning} {...tooltipProps(unknownModelTooltip(data))}>
            <WarningTriangle size={11} />
          </span>
        ) : null}
      </span>
      <HoverPopover
        enabled={hasDetail}
        width={DETAIL_WIDTH}
        className="uc-row-value-slot"
        panel={() =>
          breakdown && data.modelBreakdown ? (
            <ModelBreakdownDetail title={data.title} breakdown={data.modelBreakdown} language={data.language} />
          ) : (
            <ResetsDetail data={data} now={now} />
          )
        }
      >
        {({ highlighted }) => (
          <span className={`uc-row-value${highlighted ? " is-highlighted" : ""}`}>
            <span className="uc-row-value-line">
              <ExpiryDot data={data} now={now} />
              <span className="uc-num" {...tooltipProps(hasDetail ? null : unboundedValueTooltip(data, now))}>
                {unboundedDetail(data)}
              </span>
            </span>
            {subtitle ? <span className="uc-row-subtitle">{subtitle}</span> : null}
          </span>
        )}
      </HoverPopover>
    </div>
  );
}
