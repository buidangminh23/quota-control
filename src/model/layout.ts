/**
 * The customizable layout: which metrics are on, provider and metric order, taskbar stars, and what
 * sits behind each card's caret. Port of upstream `LayoutStore`, `LayoutBootstrap` and
 * `DefaultLayout`, as a plain document with pure functions over it (the zustand store owns undo and
 * persistence).
 *
 * The catalog is the source of truth for what exists. Account cards (`claude@…`) inherit their
 * family's defaults like upstream `DefaultLayout.expandingAccounts`; local-history cards
 * (`claude-local`) have their own. Defaults reach each descriptor exactly once (`offered`), so a
 * metric the user turned off stays off, while a metric shipped later still appears. A later change
 * to a default reaches saved layouts through `LAYOUT_REVISIONS`, once each.
 */
import type { Provider, ProviderEntry, WidgetDescriptor } from "@/lib/types";

export const LAYOUT_VERSION = 1;
export const MAX_PINS_PER_PROVIDER = 2;

export interface LayoutDocument {
  version: typeof LAYOUT_VERSION;
  /** Enabled descriptor ids. */
  placed: string[];
  /** Provider ids in display order; ids of providers that are gone are kept for their return. */
  providerOrder: string[];
  /** Per provider, every descriptor id in the user's order. */
  metricOrder: Record<string, string[]>;
  /** Descriptor ids starred for the taskbar (at most two per provider). */
  pinned: string[];
  /** Descriptor ids below the caret (On Demand). Membership survives turning a metric off. */
  onDemand: string[];
  /** Provider ids whose caret is open. */
  openProviders: string[];
  /** Optional metrics that start On Demand the first time they are turned on. */
  onDemandWhenEnabled: string[];
  /** Descriptor ids the defaults were already applied to. */
  offered: string[];
  /** Ids of the `LAYOUT_REVISIONS` this document already went through. */
  revisions: string[];
}

interface FamilyDefaults {
  enabled: readonly string[];
  onDemand: readonly string[];
  pinned: readonly string[];
}

const LOCAL_HISTORY_DEFAULTS: FamilyDefaults = {
  enabled: ["trend", "today", "yesterday", "last30", "inputTokens", "outputTokens", "cachedInputTokens"],
  onDemand: ["inputTokens", "outputTokens", "cachedInputTokens"],
  pinned: [],
};

/** Upstream `DefaultLayout`, keyed by layout family and expressed as descriptor suffixes. */
const FAMILY_DEFAULTS: Readonly<Record<string, FamilyDefaults>> = {
  claude: {
    enabled: ["session", "weekly", "fable", "extra", "trend", "rateLimitResets", "today", "yesterday", "last30"],
    onDemand: ["sonnet", "rateLimitResets", "today", "yesterday", "last30"],
    pinned: ["session", "weekly"],
  },
  codex: {
    enabled: ["session", "weekly", "spark", "sparkWeekly", "trend", "credits", "rateLimitResets", "today", "yesterday", "last30"],
    onDemand: ["spark", "sparkWeekly", "credits", "today", "yesterday", "last30"],
    pinned: ["session", "weekly"],
  },
  "claude-local": LOCAL_HISTORY_DEFAULTS,
  "codex-local": LOCAL_HISTORY_DEFAULTS,
  cursor: {
    enabled: ["usage", "auto", "api", "grokBot", "trend", "onDemand", "today", "yesterday", "last30"],
    onDemand: ["grokBot", "onDemand", "requests", "credits", "today", "yesterday", "last30"],
    pinned: ["auto", "api"],
  },
  grok: {
    enabled: ["weekly", "trend", "payAsYouGo", "today", "yesterday", "last30"],
    onDemand: ["payAsYouGo", "today", "yesterday", "last30"],
    pinned: [],
  },
};

interface LayoutRevision {
  id: string;
  apply(layout: LayoutDocument): LayoutDocument;
}

/**
 * Changes to the defaults that also reach layouts saved before them, each applied once. They only
 * move metrics between sections, so a metric the user turned off stays off.
 */
const LAYOUT_REVISIONS: readonly LayoutRevision[] = [
  {
    id: "codex-rate-limit-resets-always-visible",
    apply: (layout) => ({
      ...layout,
      onDemand: layout.onDemand.filter((id) => !(layoutFamily(descriptorProviderId(id)) === "codex" && id.endsWith(".rateLimitResets"))),
    }),
  },
];

/** Established providers lead (upstream AGENTS.md "Default order"); the rest follow alphabetically. */
const LEADING_BRANDS = ["claude", "codex", "cursor"];

/** The family an account-scoped card belongs to: `claude@1a2b` → `claude`; others are their own. */
export function layoutFamily(providerId: string): string {
  const at = providerId.indexOf("@");
  return at >= 0 ? providerId.slice(0, at) : providerId;
}

/** The brand a card shows (its mark and color): `claude@1a2b` and `claude-local` → `claude`. */
export function brandOf(providerId: string): string {
  return layoutFamily(providerId).replace(/-local$/, "");
}

export function isAccountCard(providerId: string): boolean {
  return providerId.includes("@");
}

export function isLocalHistoryCard(providerId: string): boolean {
  return layoutFamily(providerId).endsWith("-local");
}

/**
 * Whether a provider gets a card on the Hạn mức tab. Local-history providers (`claude-local`) log the
 * same usage as the signed-in accounts, so they only feed the Token tab (`tokenGroups`).
 */
export function hasDashboardCard(providerId: string): boolean {
  return !isLocalHistoryCard(providerId);
}

/** Spend tiles and the usage trend belong to the Token tab, never to an account's card. */
export function isTokenMetric(descriptor: WidgetDescriptor): boolean {
  return descriptor.isSpendTile || descriptor.template.isChart === true;
}

function brandRank(brand: string): [number, string] {
  const index = LEADING_BRANDS.indexOf(brand);
  return index >= 0 ? [index, ""] : [LEADING_BRANDS.length, brand];
}

/** Catalog order for a fresh layout: brand rank, then account cards before the brand's local card. */
export function defaultProviderOrder(catalog: readonly ProviderEntry[]): string[] {
  return catalog
    .map((entry, index) => ({ id: entry.provider.id, index }))
    .sort((a, b) => {
      const [rankA, nameA] = brandRank(brandOf(a.id));
      const [rankB, nameB] = brandRank(brandOf(b.id));
      if (rankA !== rankB) return rankA - rankB;
      if (nameA !== nameB) return nameA < nameB ? -1 : 1;
      const localA = isLocalHistoryCard(a.id) ? 1 : 0;
      const localB = isLocalHistoryCard(b.id) ? 1 : 0;
      return localA !== localB ? localA - localB : a.index - b.index;
    })
    .map((item) => item.id);
}

interface DefaultIds {
  enabled: Set<string>;
  onDemand: Set<string>;
  pinned: string[];
}

function defaultIds(entry: ProviderEntry): DefaultIds {
  const id = entry.provider.id;
  const valid = new Set(entry.descriptors.map((descriptor) => descriptor.id));
  const defaults = FAMILY_DEFAULTS[layoutFamily(id)];
  if (!defaults) return { enabled: valid, onDemand: new Set(), pinned: [] };
  const ids = (suffixes: readonly string[]) => suffixes.map((suffix) => `${id}.${suffix}`).filter((candidate) => valid.has(candidate));
  return { enabled: new Set(ids(defaults.enabled)), onDemand: new Set(ids(defaults.onDemand)), pinned: ids(defaults.pinned) };
}

export function emptyLayout(): LayoutDocument {
  return {
    version: LAYOUT_VERSION,
    placed: [],
    providerOrder: [],
    metricOrder: {},
    pinned: [],
    onDemand: [],
    openProviders: [],
    onDemandWhenEnabled: [],
    offered: [],
    revisions: LAYOUT_REVISIONS.map((revision) => revision.id),
  };
}

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? [...new Set(value.filter((item): item is string => typeof item === "string"))] : [];
}

/** Read a stored layout, or `null` when there is none or it is not a layout this version understands. */
export function parseLayout(raw: unknown): LayoutDocument | null {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) return null;
  const stored = raw as Record<string, unknown>;
  if (stored.version !== LAYOUT_VERSION) return null;
  const metricOrder: Record<string, string[]> = {};
  const rawOrder = stored.metricOrder;
  if (rawOrder && typeof rawOrder === "object" && !Array.isArray(rawOrder)) {
    for (const [providerId, ids] of Object.entries(rawOrder as Record<string, unknown>)) metricOrder[providerId] = stringList(ids);
  }
  return {
    version: LAYOUT_VERSION,
    placed: stringList(stored.placed),
    providerOrder: stringList(stored.providerOrder),
    metricOrder,
    pinned: stringList(stored.pinned),
    onDemand: stringList(stored.onDemand),
    openProviders: stringList(stored.openProviders),
    onDemandWhenEnabled: stringList(stored.onDemandWhenEnabled),
    offered: stringList(stored.offered),
    revisions: stringList(stored.revisions),
  };
}

/** A saved layout with the revisions it has not gone through yet applied and recorded. */
function revised(layout: LayoutDocument): LayoutDocument {
  const done = new Set(layout.revisions);
  const pending = LAYOUT_REVISIONS.filter((revision) => !done.has(revision.id));
  if (pending.length === 0) return layout;
  const next = pending.reduce((current, revision) => revision.apply(current), layout);
  return { ...next, revisions: [...layout.revisions, ...pending.map((revision) => revision.id)] };
}

function insertProvider(order: string[], id: string, defaultOrder: readonly string[]): string[] {
  const brand = brandOf(id);
  const next = [...order];
  if (isAccountCard(id)) {
    const localIndex = next.findIndex((existing) => brandOf(existing) === brand && isLocalHistoryCard(existing));
    if (localIndex >= 0) {
      next.splice(localIndex, 0, id);
      return next;
    }
  }
  let lastSameBrand = -1;
  next.forEach((existing, index) => {
    if (brandOf(existing) === brand) lastSameBrand = index;
  });
  if (lastSameBrand >= 0) {
    next.splice(lastSameBrand + 1, 0, id);
    return next;
  }
  const position = defaultOrder.indexOf(id);
  const successor = defaultOrder.slice(position + 1).find((candidate) => next.includes(candidate));
  if (successor === undefined) next.push(id);
  else next.splice(next.indexOf(successor), 0, id);
  return next;
}

/** Saved order restricted to valid ids, then anything missing in declaration order. */
function normalizedOrder(saved: readonly string[] | undefined, valid: readonly string[]): string[] {
  const validSet = new Set(valid);
  const kept = (saved ?? []).filter((id) => validSet.has(id));
  const keptSet = new Set(kept);
  return [...kept, ...valid.filter((id) => !keptSet.has(id))];
}

function pinCount(pinned: readonly string[], providerId: string): number {
  return pinned.filter((id) => descriptorProviderId(id) === providerId).length;
}

/** The provider part of a descriptor id (`claude@1a2b.session` → `claude@1a2b`). */
export function descriptorProviderId(descriptorId: string): string {
  const dot = descriptorId.lastIndexOf(".");
  return dot >= 0 ? descriptorId.slice(0, dot) : descriptorId;
}

/** Bring a stored layout (or none) in line with the catalog, applying defaults to new metrics once. */
export function reconcileLayout(stored: LayoutDocument | null, catalog: readonly ProviderEntry[]): LayoutDocument {
  const layout = stored ? revised(structuredClone(stored)) : emptyLayout();
  const defaultOrder = defaultProviderOrder(catalog);
  for (const id of defaultOrder) {
    if (!layout.providerOrder.includes(id)) layout.providerOrder = insertProvider(layout.providerOrder, id, defaultOrder);
  }
  const placed = new Set(layout.placed);
  const onDemand = new Set(layout.onDemand);
  const onDemandWhenEnabled = new Set(layout.onDemandWhenEnabled);
  const offered = new Set(layout.offered);
  const pinned = [...layout.pinned];
  for (const entry of catalog) {
    const providerId = entry.provider.id;
    layout.metricOrder[providerId] = normalizedOrder(
      layout.metricOrder[providerId],
      entry.descriptors.map((descriptor) => descriptor.id),
    );
    const defaults = defaultIds(entry);
    for (const descriptor of entry.descriptors) {
      const id = descriptor.id;
      if (offered.has(id)) continue;
      offered.add(id);
      const enabled = defaults.enabled.has(id);
      if (enabled) placed.add(id);
      if (defaults.onDemand.has(id)) (enabled ? onDemand : onDemandWhenEnabled).add(id);
      if (defaults.pinned.includes(id) && descriptor.pinnable && pinCount(pinned, providerId) < MAX_PINS_PER_PROVIDER) {
        if (!pinned.includes(id)) pinned.push(id);
      }
    }
  }
  layout.placed = [...placed];
  layout.onDemand = [...onDemand];
  layout.onDemandWhenEnabled = [...onDemandWhenEnabled];
  layout.offered = [...offered];
  layout.pinned = pinned;
  return layout;
}

export type IsEnabled = (providerId: string) => boolean;

export interface ProviderMetrics {
  provider: Provider;
  always: WidgetDescriptor[];
  onDemand: WidgetDescriptor[];
}

function entriesById(catalog: readonly ProviderEntry[]): Map<string, ProviderEntry> {
  return new Map(catalog.map((entry) => [entry.provider.id, entry]));
}

/** Catalog entries in the layout's provider order (entries the layout has not seen trail). */
export function orderedEntries(layout: LayoutDocument, catalog: readonly ProviderEntry[]): ProviderEntry[] {
  const byId = entriesById(catalog);
  const ordered = layout.providerOrder.flatMap((id) => {
    const entry = byId.get(id);
    return entry ? [entry] : [];
  });
  const seen = new Set(ordered.map((entry) => entry.provider.id));
  return [...ordered, ...catalog.filter((entry) => !seen.has(entry.provider.id))];
}

/** A provider's descriptors in the user's metric order. */
export function orderedDescriptors(layout: LayoutDocument, entry: ProviderEntry): WidgetDescriptor[] {
  const byId = new Map(entry.descriptors.map((descriptor) => [descriptor.id, descriptor]));
  return normalizedOrder(layout.metricOrder[entry.provider.id], [...byId.keys()]).flatMap((id) => {
    const descriptor = byId.get(id);
    return descriptor ? [descriptor] : [];
  });
}

function split(layout: LayoutDocument, descriptors: WidgetDescriptor[]): { always: WidgetDescriptor[]; onDemand: WidgetDescriptor[] } {
  const below = new Set(layout.onDemand);
  return {
    always: descriptors.filter((descriptor) => !below.has(descriptor.id)),
    onDemand: descriptors.filter((descriptor) => below.has(descriptor.id)),
  };
}

/** A card's metrics in the user's order, without the token metrics the Token tab totals. */
function cardDescriptors(layout: LayoutDocument, entry: ProviderEntry): WidgetDescriptor[] {
  return orderedDescriptors(layout, entry).filter((descriptor) => !isTokenMetric(descriptor));
}

/**
 * The Hạn mức tab's sections: enabled providers that have a card and at least one enabled metric. A
 * card whose every metric is On Demand shows them all above the caret, so a card never renders empty.
 */
export function displayGroups(layout: LayoutDocument, catalog: readonly ProviderEntry[], isEnabled: IsEnabled): ProviderMetrics[] {
  const placed = new Set(layout.placed);
  return orderedEntries(layout, catalog).flatMap((entry) => {
    if (!isEnabled(entry.provider.id) || !hasDashboardCard(entry.provider.id)) return [];
    const visible = cardDescriptors(layout, entry).filter((descriptor) => placed.has(descriptor.id));
    if (visible.length === 0) return [];
    const { always, onDemand } = split(layout, visible);
    return [always.length === 0 ? { provider: entry.provider, always: onDemand, onDemand: [] } : { provider: entry.provider, always, onDemand }];
  });
}

/**
 * The Token tab's sections under the total: each enabled token source with its usage trend and its
 * period rows, in the user's metric order. Customize no longer lists these providers, so the rows
 * ignore `placed` and are all shown, and the per-kind token rows stay out: the tab counts every kind
 * of token together.
 */
export function tokenGroups(layout: LayoutDocument, catalog: readonly ProviderEntry[], isEnabled: IsEnabled): ProviderMetrics[] {
  return orderedEntries(layout, catalog).flatMap((entry) => {
    if (hasDashboardCard(entry.provider.id) || !isEnabled(entry.provider.id)) return [];
    const always = orderedDescriptors(layout, entry).filter(isTokenMetric);
    return always.length === 0 ? [] : [{ provider: entry.provider, always, onDemand: [] }];
  });
}

/** Every metric a provider's card supports, split by section (Customize detail). */
export function customizeDetail(layout: LayoutDocument, catalog: readonly ProviderEntry[], providerId: string): ProviderMetrics | null {
  const entry = entriesById(catalog).get(providerId);
  if (!entry || !hasDashboardCard(providerId)) return null;
  return { provider: entry.provider, ...split(layout, cardDescriptors(layout, entry)) };
}

export interface ProviderRow {
  provider: Provider;
  enabled: boolean;
  metricCount: number;
}

/** The Customize provider list: every provider with a card, including disabled ones. */
export function customizeRows(layout: LayoutDocument, catalog: readonly ProviderEntry[], isEnabled: IsEnabled): ProviderRow[] {
  return orderedEntries(layout, catalog)
    .filter((entry) => hasDashboardCard(entry.provider.id))
    .map((entry) => ({
      provider: entry.provider,
      enabled: isEnabled(entry.provider.id),
      metricCount: entry.descriptors.filter((descriptor) => !isTokenMetric(descriptor)).length,
    }));
}

/** Starred metrics per enabled provider, Always Visible first (the strip's order). */
export function pinnedGroups(layout: LayoutDocument, catalog: readonly ProviderEntry[], isEnabled: IsEnabled): ProviderMetrics[] {
  const pinned = new Set(layout.pinned);
  return orderedEntries(layout, catalog).flatMap((entry) => {
    if (!isEnabled(entry.provider.id)) return [];
    const metrics = orderedDescriptors(layout, entry).filter((descriptor) => pinned.has(descriptor.id));
    return metrics.length === 0 ? [] : [{ provider: entry.provider, ...split(layout, metrics) }];
  });
}

/** Enabled providers that ship spend tiles: exactly what the Total Spend card aggregates. */
export function spendCapableProviders(layout: LayoutDocument, catalog: readonly ProviderEntry[], isEnabled: IsEnabled): Provider[] {
  return orderedEntries(layout, catalog)
    .filter((entry) => isEnabled(entry.provider.id) && entry.descriptors.some((descriptor) => descriptor.isSpendTile))
    .map((entry) => entry.provider);
}

export function isMetricEnabled(layout: LayoutDocument, descriptorId: string): boolean {
  return layout.placed.includes(descriptorId);
}

export function isPinned(layout: LayoutDocument, descriptorId: string): boolean {
  return layout.pinned.includes(descriptorId);
}

function findDescriptor(catalog: readonly ProviderEntry[], descriptorId: string): WidgetDescriptor | undefined {
  for (const entry of catalog) {
    const found = entry.descriptors.find((descriptor) => descriptor.id === descriptorId);
    if (found) return found;
  }
  return undefined;
}

export function canPin(layout: LayoutDocument, catalog: readonly ProviderEntry[], descriptorId: string): boolean {
  if (layout.pinned.includes(descriptorId)) return true;
  const descriptor = findDescriptor(catalog, descriptorId);
  if (!descriptor?.pinnable) return false;
  return pinCount(layout.pinned, descriptor.providerId) < MAX_PINS_PER_PROVIDER;
}

export function setMetricEnabled(layout: LayoutDocument, descriptorId: string, enabled: boolean): LayoutDocument {
  const isOn = layout.placed.includes(descriptorId);
  if (enabled === isOn) return layout;
  if (!enabled) return { ...layout, placed: layout.placed.filter((id) => id !== descriptorId) };
  const next = { ...layout, placed: [...layout.placed, descriptorId] };
  if (layout.onDemandWhenEnabled.includes(descriptorId)) {
    next.onDemandWhenEnabled = layout.onDemandWhenEnabled.filter((id) => id !== descriptorId);
    if (!layout.onDemand.includes(descriptorId)) next.onDemand = [...layout.onDemand, descriptorId];
  }
  return next;
}

export function setPinned(layout: LayoutDocument, catalog: readonly ProviderEntry[], descriptorId: string, pinned: boolean): LayoutDocument {
  const isOn = layout.pinned.includes(descriptorId);
  if (pinned === isOn) return layout;
  if (!pinned) return { ...layout, pinned: layout.pinned.filter((id) => id !== descriptorId) };
  if (!canPin(layout, catalog, descriptorId)) return layout;
  return { ...layout, pinned: [...layout.pinned, descriptorId] };
}

export function setProviderOpen(layout: LayoutDocument, providerId: string, open: boolean): LayoutDocument {
  const isOpen = layout.openProviders.includes(providerId);
  if (open === isOpen) return layout;
  return { ...layout, openProviders: open ? [...layout.openProviders, providerId] : layout.openProviders.filter((id) => id !== providerId) };
}

/** Move `dragged` to `target`'s position (upstream `LayoutStore.reordered`), or `null` for a no-op. */
export function reorderedIds(ids: readonly string[], dragged: string, target: string): string[] | null {
  if (dragged === target) return null;
  const from = ids.indexOf(dragged);
  const to = ids.indexOf(target);
  if (from < 0 || to < 0) return null;
  const next = [...ids];
  next.splice(from, 1);
  next.splice(to, 0, dragged);
  return next;
}

/** Reorder among enabled providers; disabled ones keep their relative place at the tail. */
export function reorderProvider(
  layout: LayoutDocument,
  catalog: readonly ProviderEntry[],
  isEnabled: IsEnabled,
  dragged: string,
  target: string,
): LayoutDocument {
  const all = orderedEntries(layout, catalog).map((entry) => entry.provider.id);
  const shown = all.filter(isEnabled);
  const next = reorderedIds(shown, dragged, target);
  if (!next) return layout;
  const rest = layout.providerOrder.filter((id) => !next.includes(id));
  return { ...layout, providerOrder: [...next, ...rest] };
}

/**
 * Apply a drag result: the provider's metrics as the Always Visible list followed by the On Demand
 * list. Section membership follows the lists; ids the lists omit keep their place. Only the dragged
 * metric's "start On Demand when enabled" default is consumed, as an explicit placement.
 */
export function applyMetricSections(
  layout: LayoutDocument,
  providerId: string,
  always: readonly string[],
  onDemand: readonly string[],
  dragged: string,
): LayoutDocument {
  const current = layout.metricOrder[providerId] ?? [];
  const listed = new Set([...always, ...onDemand]);
  const order = [...always, ...onDemand, ...current.filter((id) => !listed.has(id))];
  const below = new Set(layout.onDemand);
  for (const id of always) below.delete(id);
  for (const id of onDemand) below.add(id);
  const nextOnDemand = [...below];
  const consumed = layout.onDemandWhenEnabled.includes(dragged);
  const sameOrder = order.length === current.length && order.every((id, index) => id === current[index]);
  const sameSections = nextOnDemand.length === layout.onDemand.length && nextOnDemand.every((id) => layout.onDemand.includes(id));
  if (sameOrder && sameSections && !consumed) return layout;
  return {
    ...layout,
    metricOrder: { ...layout.metricOrder, [providerId]: order },
    onDemand: nextOnDemand,
    onDemandWhenEnabled: consumed ? layout.onDemandWhenEnabled.filter((id) => id !== dragged) : layout.onDemandWhenEnabled,
  };
}

/** Restore one provider's defaults (metrics, order, stars, sections), leaving everything else alone. */
export function resetProvider(layout: LayoutDocument, catalog: readonly ProviderEntry[], providerId: string): LayoutDocument {
  const entry = entriesById(catalog).get(providerId);
  if (!entry) return layout;
  const owned = new Set(entry.descriptors.map((descriptor) => descriptor.id));
  const defaults = defaultIds(entry);
  const keep = (ids: readonly string[]) => ids.filter((id) => !owned.has(id));
  const pinned = keep(layout.pinned);
  for (const id of defaults.pinned) {
    if (pinCount(pinned, providerId) < MAX_PINS_PER_PROVIDER) pinned.push(id);
  }
  return {
    ...layout,
    placed: [...keep(layout.placed), ...defaults.enabled],
    metricOrder: { ...layout.metricOrder, [providerId]: entry.descriptors.map((descriptor) => descriptor.id) },
    pinned,
    onDemand: [...keep(layout.onDemand), ...[...defaults.onDemand].filter((id) => defaults.enabled.has(id))],
    onDemandWhenEnabled: [...keep(layout.onDemandWhenEnabled), ...[...defaults.onDemand].filter((id) => !defaults.enabled.has(id))],
    openProviders: layout.openProviders.filter((id) => id !== providerId),
    offered: [...new Set([...layout.offered, ...owned])],
  };
}

/** Every provider back to defaults, in the default order. */
export function resetAllLayout(catalog: readonly ProviderEntry[]): LayoutDocument {
  return reconcileLayout(null, catalog);
}

export function sameLayout(a: LayoutDocument, b: LayoutDocument): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
