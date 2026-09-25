/**
 * Hover detail panels behind unbounded values: the per-model spend breakdown with token splits
 * (upstream `ModelUsageDetail`, plus input/output/cache figures) and the reset-credit timeline
 * (a read-only upstream `RateLimitResetsDetail`).
 */
import { messagesFor, translate, type Language } from "@/i18n";
import type { ModelUsageBreakdown, TokenUsage } from "@/lib/types";
import { deadlineLabel, formatNumber } from "@/model/format";
import { foldedModels, modelShares, OTHER_MODEL_NAME, wholePercents } from "@/model/modelUsage";
import { expirySeverityFor, resetCreditCount, type WidgetData } from "@/model/widgetData";

function tokenCount(value: number, language: Language): string {
  return formatNumber(value, "count", "row", language);
}

function TokenSplit({ usage, language }: { usage: TokenUsage; language: Language }) {
  const dashboard = messagesFor(language).dashboard;
  const cells: Array<[string, number]> = [
    [dashboard.inputTokens, usage.inputTokens],
    [dashboard.outputTokens, usage.outputTokens],
    [dashboard.cacheReadTokens, usage.cachedInputTokens],
  ];
  if (usage.cacheCreationInputTokens > 0) cells.push([dashboard.cacheWriteTokens, usage.cacheCreationInputTokens]);
  return (
    <div className={`uc-token-split${cells.length > 3 ? " is-quad" : ""}`}>
      {cells.map(([label, value]) => (
        <div key={label} className="uc-token-cell">
          <span className="uc-token-label">{label}</span>
          <span className="uc-token-value uc-num">{tokenCount(value, language)}</span>
        </div>
      ))}
    </div>
  );
}

export function ModelBreakdownDetail({ title, breakdown, language }: { title: string; breakdown: ModelUsageBreakdown; language: Language }) {
  const models = foldedModels(breakdown);
  const shares = modelShares(models);
  const percents = wholePercents(shares);
  const dashboard = messagesFor(language).dashboard;
  return (
    <div className="uc-detail is-roomy">
      <div className="uc-detail-header">
        <span className="uc-detail-title is-large">{title}</span>
      </div>
      {breakdown.tokenUsage ? <TokenSplit usage={breakdown.tokenUsage} language={language} /> : null}
      <div className="uc-model-list">
        {models.map((model, index) => {
          const name = model.model === OTHER_MODEL_NAME ? dashboard.otherModels : model.model;
          const usage = model.tokenUsage;
          return (
            <div key={`${model.model}-${index}`} className="uc-model-row">
              <div className="uc-model-line">
                <span className="uc-model-name uc-truncate">{name}</span>
                {model.costUSD !== undefined && model.costUSD !== null ? (
                  <span className="uc-num">{formatNumber(model.costUSD, "dollars", "row", language)}</span>
                ) : (
                  <span className="uc-tertiary">—</span>
                )}
              </div>
              <div className="uc-model-line uc-secondary">
                <span className="uc-num">{percents[index] ?? 0}%</span>
                <span className="uc-num">{dashboard.tokensReadout(tokenCount(model.totalTokens, language))}</span>
              </div>
              {usage ? (
                <div className="uc-model-line uc-model-tokens uc-num">
                  <span>
                    {dashboard.inputTokens} {tokenCount(usage.inputTokens, language)} · {dashboard.outputTokens} {tokenCount(usage.outputTokens, language)}
                  </span>
                </div>
              ) : null}
              <div className="uc-share-bar">
                <div className="uc-share-fill" style={{ width: `${(shares[index] ?? 0) * 100}%` }} />
              </div>
            </div>
          );
        })}
      </div>
      <p className="uc-source-note">{translate(breakdown.sourceNote, language)}</p>
    </div>
  );
}

export function ResetsDetail({ data, now }: { data: WidgetData; now: Date }) {
  const language = data.language;
  const dashboard = messagesFor(language).dashboard;
  const count = resetCreditCount(data);
  const expiries = [...data.expiriesAt].sort((a, b) => a.getTime() - b.getTime());
  const unknown = Math.max(0, count - expiries.length);
  return (
    <div className="uc-detail">
      <div className="uc-detail-header">
        <span className="uc-detail-title">{data.title}</span>
        <span className="uc-detail-readout uc-num">{count}</span>
      </div>
      {count === 0 && expiries.length === 0 ? <p className="uc-detail-empty">{dashboard.resetsEmpty}</p> : null}
      {expiries.length > 0 ? (
        <ul className="uc-expiry-list">
          {expiries.map((date, index) => {
            const severity = expirySeverityFor((date.getTime() - now.getTime()) / 1000);
            const color = severity === "critical" ? "var(--uc-red)" : severity === "warning" ? "var(--uc-yellow)" : "var(--uc-blue)";
            return (
              <li key={`${date.getTime()}-${index}`} className="uc-expiry-item">
                <span className="uc-status-dot" style={{ background: color }} />
                <span className="uc-num">{deadlineLabel("resetExpires", date, data.resetDisplayMode, now, data.timeFormat, language)}</span>
              </li>
            );
          })}
        </ul>
      ) : null}
      {unknown > 0 ? <p className="uc-detail-empty">{dashboard.resetsUnknownExpiries(unknown)}</p> : null}
    </div>
  );
}
