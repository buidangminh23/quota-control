/**
 * The per-model breakdown behind a spend row's hover: ranked models, a folded "Other" tail, share
 * percentages that always total 100, and token splits. Port of upstream `ModelUsageDetail.shares`,
 * `wholePercents` and the mapper's "top named models, 5% floor" folding rule.
 */
import type { ModelUsageBreakdown, ModelUsageEntry, TokenUsage } from "@/lib/types";

/** At most this many models are listed by name; the rest fold into "Other". */
export const MAX_NAMED_MODELS = 5;
/** A model under this share of the period folds into "Other". */
export const MINIMUM_NAMED_SHARE = 0.05;
/** Source-text name of the folded tail row (displayed through the catalog). */
export const OTHER_MODEL_NAME = "Other";

function addUsage(a: TokenUsage | undefined, b: TokenUsage | undefined): TokenUsage | undefined {
  if (!a || !b) return undefined;
  return {
    inputTokens: a.inputTokens + b.inputTokens,
    outputTokens: a.outputTokens + b.outputTokens,
    cachedInputTokens: a.cachedInputTokens + b.cachedInputTokens,
    cacheCreationInputTokens: a.cacheCreationInputTokens + b.cacheCreationInputTokens,
  };
}

function allPriced(models: readonly ModelUsageEntry[]): boolean {
  return models.every((model) => model.costUSD !== undefined && model.costUSD !== null);
}

/** Cost shares when every model is priced, otherwise token shares (one basis for the whole list). */
export function modelShares(models: readonly ModelUsageEntry[]): number[] {
  if (allPriced(models)) {
    const total = models.reduce((sum, model) => sum + (model.costUSD ?? 0), 0);
    if (total > 0) return models.map((model) => Math.min(Math.max((model.costUSD ?? 0) / total, 0), 1));
  }
  const tokens = models.reduce((sum, model) => sum + model.totalTokens, 0);
  if (!(tokens > 0)) return models.map(() => 0);
  return models.map((model) => Math.min(Math.max(model.totalTokens / tokens, 0), 1));
}

/** Whole percentages totalling exactly 100 (largest remainder); all-zero shares stay zero. */
export function wholePercents(shares: readonly number[]): number[] {
  if (!shares.some((share) => share > 0)) return shares.map(() => 0);
  const raw = shares.map((share) => share * 100);
  const percents = raw.map((value) => Math.floor(value));
  let leftover = 100 - percents.reduce((sum, value) => sum + value, 0);
  const byRemainder = raw
    .map((value, index) => ({ index, remainder: value - Math.floor(value) }))
    .sort((a, b) => (a.remainder !== b.remainder ? b.remainder - a.remainder : a.index - b.index));
  for (const { index } of byRemainder) {
    if (leftover <= 0) break;
    percents[index]! += 1;
    leftover -= 1;
  }
  return percents;
}

/** Models ranked by their share, the long tail folded into one "Other" row. */
export function foldedModels(breakdown: ModelUsageBreakdown): ModelUsageEntry[] {
  const models = breakdown.models.filter((model) => model.totalTokens > 0 || (model.costUSD ?? 0) > 0);
  const shares = modelShares(models);
  const ranked = models
    .map((model, index) => ({ model, share: shares[index] ?? 0 }))
    .sort((a, b) => (a.share !== b.share ? b.share - a.share : a.model.model.localeCompare(b.model.model)));
  const named = ranked.filter((item, index) => index < MAX_NAMED_MODELS && item.share >= MINIMUM_NAMED_SHARE);
  const tail = ranked.filter((item) => !named.includes(item));
  if (tail.length === 0) return named.map((item) => item.model);
  if (tail.length === 1 && named.length < MAX_NAMED_MODELS) return ranked.map((item) => item.model);
  const priced = allPriced(tail.map((item) => item.model));
  const usages = tail.map((item) => item.model.tokenUsage);
  const complete = usages.every((usage): usage is TokenUsage => usage !== undefined);
  const other: ModelUsageEntry = {
    model: OTHER_MODEL_NAME,
    totalTokens: tail.reduce((sum, item) => sum + item.model.totalTokens, 0),
    costUSD: priced ? tail.reduce((sum, item) => sum + (item.model.costUSD ?? 0), 0) : undefined,
    tokenUsage: complete ? usages.slice(1).reduce<TokenUsage | undefined>((acc, usage) => addUsage(acc, usage), usages[0]) : undefined,
  };
  return [...named.map((item) => item.model), other];
}
