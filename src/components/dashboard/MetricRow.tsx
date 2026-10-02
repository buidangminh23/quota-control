/**
 * One metric inside a provider card (upstream `WidgetRowView`). A bounded metric is a title line with
 * the countdown to its reset, a capsule meter, then its reading with the exact moment it resets; both
 * reset texts keep their place whatever Reset Times is set to, and the countdown steps every second
 * through its last five minutes. An unbounded metric is a single line with the value right-aligned,
 * which reveals a hover detail when there is more to show.
 */
import type { Language } from "@/i18n";
import { messagesFor } from "@/i18n";
import {
  boundedDetailText,
  boundedDetailTooltip,
  boundedResetMoment,
  hasMeterStyleToggle,
  hasResetLabel,
  meterSeverity,
  meterState,
  meterStyleTooltip,
  meterTooltip,
  resetCountdownText,
  spareText,
  type MeterState,
} from "@/model/meterState";
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
import { useFinalCountdown } from "@/state/hooks";
import { updateSettings } from "@/state/store";
import { HoverPopover } from "../ui/hoverPopover";
import { FlameIcon, WarningTriangle } from "../ui/icons";
import { clippedTooltipProps, tooltipProps, truncatedTooltipProps } from "../ui/tooltip";
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

function toggleMeterStyle(data: WidgetData): void {
  updateSettings({ displayMode: data.displayMode === "remaining" ? "used" : "remaining" });
}

/**
 * The pace note has the title line to itself only on a row with no reset words there; beside a
 * countdown it is the countdown's and the meter's tooltip. The exact moment is drawn in two parts so
 * a line too narrow for it leaves out its lead-in before it cuts the time or the day.
 */
function BoundedRow({ data, now, interactive }: { data: WidgetData; now: Date; interactive: boolean }) {
  const clock = useFinalCountdown(hasResetLabel(data, now) ? data.resetsAt : null, now);
  const state = meterState(data, clock);
  const language = data.language;
  const countdown = resetCountdownText(data, clock);
  const detail = boundedDetailText(data, clock);
  const detailTooltip = boundedDetailTooltip(data, clock);
  const moment = boundedResetMoment(data, clock);
  const styleToggle = interactive && hasMeterStyleToggle(data);
  return (
    <div className="uc-row is-bounded">
      <div className="uc-row-label">
        <span className="uc-row-title uc-truncate" {...truncatedTooltipProps(data.title)}>
          {data.title}
        </span>
        {countdown ? (
          <span className="uc-row-countdown uc-num" {...tooltipProps(meterTooltip(state, language))}>
            {countdown}
          </span>
        ) : (
          <PaceWarning data={data} state={state} language={language} />
        )}
      </div>
      <Meter data={data} state={state} now={clock} />
      <div className="uc-row-primary">
        {styleToggle ? (
          <button type="button" className="uc-row-headline uc-num" onClick={() => toggleMeterStyle(data)} {...tooltipProps(meterStyleTooltip(data))}>
            {headline(data)}
          </button>
        ) : (
          <span className="uc-row-headline uc-num">{headline(data)}</span>
        )}
        {moment ? (
          <span className="uc-row-trailing uc-row-moment uc-num" {...clippedTooltipProps(moment.text)}>
            <span className="uc-row-moment-lead">{moment.lead}</span>
            <span className="uc-row-moment-time">{moment.moment}</span>
          </span>
        ) : detail ? (
          <span className="uc-row-trailing uc-num" {...(detailTooltip ? tooltipProps(detailTooltip) : truncatedTooltipProps(detail))}>
            {detail}
          </span>
        ) : null}
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
