/**
 * One provider on the dashboard (upstream `WidgetGroupedListView.section`): the header with mark,
 * name, plan and refresh state, then a card of metric rows. On Demand metrics sit behind the caret
 * together with the provider's links. Right-click on the header or a row opens its context menu.
 */
import { useMemo, useRef } from "react";
import { messagesFor, translate, type Messages } from "@/i18n";
import { backend } from "@/lib/backend";
import type { AccountProvider, ProviderRuntimeState, WidgetDescriptor } from "@/lib/types";
import {
  canPin,
  isAccountCard,
  isLocalHistoryCard,
  isPinned,
  MAX_PINS_PER_PROVIDER,
  setMetricEnabled,
  setPinned,
  setProviderOpen,
  type ProviderMetrics,
} from "@/model/layout";
import { accountLabelOf, headerNotice, providerBrand, providerTitle, stalenessHint } from "@/model/providerText";
import { condensedTextRowOffsets, widgetDataFor, type DisplayOptions, type WidgetData } from "@/model/widgetData";
import { copyScreenshot } from "@/share/screenshot";
import { navigate, openChatFor, refresh, setProviderEnabled, showNotice, updateLayout, useApp } from "@/state/store";
import { ChevronDown, ChevronUp, ExternalIcon, ShareIcon, Spinner, WarningTriangle } from "../ui/icons";
import { openMenu, type MenuEntry } from "../ui/menu";
import { ProviderMark } from "../ui/ProviderMark";
import { tooltipProps } from "../ui/tooltip";
import { MetricRow } from "./MetricRow";

const CHAT_PRODUCTS: Record<AccountProvider, string> = { claude: "Claude", codex: "ChatGPT" };

interface ProviderSectionProps {
  group: ProviderMetrics;
  runtime: ProviderRuntimeState | undefined;
  display: DisplayOptions;
  refreshIntervalMs: number;
  now: Date;
}

interface ResolvedRow {
  descriptor: WidgetDescriptor;
  data: WidgetData;
}

function resolve(descriptors: WidgetDescriptor[], runtime: ProviderRuntimeState | undefined, display: DisplayOptions): ResolvedRow[] {
  return descriptors.map((descriptor) => ({ descriptor, data: widgetDataFor(descriptor, runtime?.snapshot, display) }));
}

function condensedIds(rows: ResolvedRow[]): Set<string> {
  const offsets = condensedTextRowOffsets(rows.map((row) => row.data));
  return new Set([...offsets].map((offset) => rows[offset]!.descriptor.id));
}

/** The in-app chat target for a Claude or Codex card: the account label, or the brand for local cards. */
function chatTarget(group: ProviderMetrics): { provider: AccountProvider; label: string } | null {
  const brand = providerBrand(group.provider);
  if (brand !== "claude" && brand !== "codex") return null;
  if (isAccountCard(group.provider.id)) return { provider: brand, label: accountLabelOf(group.provider) ?? brand };
  return isLocalHistoryCard(group.provider.id) ? { provider: brand, label: brand } : null;
}

async function share(element: HTMLElement | null, messages: Messages): Promise<void> {
  const copied = await copyScreenshot(element);
  showNotice(copied ? messages.chrome.copiedToClipboard : messages.chrome.copyFailed, copied ? "positive" : "notice");
}

function openChat(target: { provider: AccountProvider; label: string }, messages: Messages): void {
  openChatFor(target.provider, target.label).catch((error: unknown) => {
    console.error("Opening chat failed", error);
    showNotice(messages.accounts.chatOpenFailed, "notice");
  });
}

export function ProviderSection({ group, runtime, display, refreshIntervalMs, now }: ProviderSectionProps) {
  const language = display.language;
  const messages = messagesFor(language);
  const providerId = group.provider.id;
  const title = providerTitle(group.provider, language);
  const sectionRef = useRef<HTMLElement>(null);
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const open = layout.openProviders.includes(providerId);
  const always = useMemo(() => resolve(group.always, runtime, display), [group.always, runtime, display]);
  const onDemand = useMemo(() => resolve(group.onDemand, runtime, display), [group.onDemand, runtime, display]);
  const condensed = useMemo(() => new Set([...condensedIds(always), ...(open ? condensedIds(onDemand) : [])]), [always, onDemand, open]);
  const links = group.provider.links ?? [];
  const hasExpandable = onDemand.length > 0 || links.length > 0;
  const notice = headerNotice(runtime, language);
  const stale = stalenessHint(runtime, refreshIntervalMs, now, language);
  const refreshing = runtime?.refreshing ?? false;
  const plan = isLocalHistoryCard(providerId) ? undefined : runtime?.snapshot?.plan;
  const chat = chatTarget(group);

  const headerEntries = (): MenuEntry[] => [
    { kind: "item", label: messages.dashboard.hideProvider(title), onSelect: () => setProviderEnabled(providerId, false) },
    { kind: "separator" },
    { kind: "item", label: messages.dashboard.refreshProvider(title), onSelect: () => refresh(providerId) },
    { kind: "item", label: messages.dashboard.customizeEllipsis, onSelect: () => navigate("customize", providerId) },
    ...(chat ? [{ kind: "item" as const, label: messages.dashboard.openChat(CHAT_PRODUCTS[chat.provider]), onSelect: () => openChat(chat, messages) }] : []),
    { kind: "separator" },
    { kind: "item", label: messages.dashboard.shareScreenshot, onSelect: () => void share(sectionRef.current, messages) },
  ];

  const rowEntries = (descriptor: WidgetDescriptor): MenuEntry[] => {
    const pinned = isPinned(layout, descriptor.id);
    return [
      { kind: "item", label: messages.dashboard.hide, onSelect: () => updateLayout((current) => setMetricEnabled(current, descriptor.id, false)) },
      ...(descriptor.pinnable
        ? [
            {
              kind: "item" as const,
              label: pinned ? messages.dashboard.unstar : messages.dashboard.starForTaskbar,
              onSelect: () => {
                if (!pinned && !canPin(layout, catalog, descriptor.id)) {
                  showNotice(messages.dashboard.pinLimit(MAX_PINS_PER_PROVIDER), "notice");
                  return;
                }
                updateLayout((current) => setPinned(current, catalog, descriptor.id, !pinned));
              },
            },
          ]
        : []),
      { kind: "separator" },
      { kind: "item", label: messages.dashboard.refreshProvider(title), onSelect: () => refresh(providerId) },
      { kind: "item", label: messages.dashboard.customizeEllipsis, onSelect: () => navigate("customize", providerId) },
    ];
  };

  const renderRow = ({ descriptor, data }: ResolvedRow) => (
    <div
      key={descriptor.id}
      className="uc-row-host"
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        openMenu({ entries: rowEntries(descriptor), anchor: { x: event.clientX, y: event.clientY } });
      }}
    >
      <MetricRow data={data} now={now} condensedTop={condensed.has(descriptor.id)} />
    </div>
  );

  return (
    <section ref={sectionRef} className="uc-section" aria-label={title} data-provider-section={providerId}>
      <header
        className="uc-section-header"
        onContextMenu={(event) => {
          event.preventDefault();
          openMenu({ entries: headerEntries(), anchor: { x: event.clientX, y: event.clientY } });
        }}
      >
        <span className="uc-section-mark">
          <ProviderMark brand={providerBrand(group.provider)} size={16} />
        </span>
        <span className="uc-section-titles">
          <span className="uc-section-name">{title}</span>
          {plan ? <span className="uc-section-plan">{translate(plan, language)}</span> : null}
          {stale && !refreshing ? (
            <span className="uc-section-stale" {...tooltipProps(stale.tooltip)}>
              {stale.label}
            </span>
          ) : null}
        </span>
        {refreshing ? (
          <span className="uc-section-status uc-secondary" aria-label={messages.dashboard.refreshing}>
            <Spinner size={11} />
          </span>
        ) : notice ? (
          <span className="uc-section-status" style={{ color: "var(--uc-orange)" }} role="img" aria-label={notice} {...tooltipProps(notice)}>
            <WarningTriangle size={11} />
          </span>
        ) : null}
        <span className="uc-spacer" />
        <button
          type="button"
          className="uc-icon-button is-reveal"
          data-share-exclude="true"
          aria-label={messages.dashboard.copyScreenshot(title)}
          onClick={() => void share(sectionRef.current, messages)}
          {...tooltipProps(messages.dashboard.copyScreenshot(title))}
        >
          <ShareIcon size={12} />
        </button>
      </header>
      <div className="uc-card uc-metric-card">
        {always.map(renderRow)}
        {hasExpandable ? (
          <button
            type="button"
            className="uc-caret"
            data-share-exclude="true"
            aria-expanded={open}
            aria-label={open ? messages.dashboard.showLess : messages.dashboard.showMore}
            onClick={() => updateLayout((current) => setProviderOpen(current, providerId, !open), { undoable: false })}
          >
            {open ? <ChevronUp size={10} /> : <ChevronDown size={10} />}
          </button>
        ) : null}
        {open ? onDemand.map(renderRow) : null}
        {open && links.length > 0 ? (
          <div className="uc-links" style={{ gridTemplateColumns: `repeat(${Math.min(3, links.length)}, minmax(0, 1fr))` }}>
            {links.map((link) => (
              <button key={link.url} type="button" className="uc-button is-bordered is-small" onClick={() => void backend().openUrl(link.url)}>
                <span className="uc-truncate">{translate(link.label, language)}</span>
                <ExternalIcon size={9} />
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </section>
  );
}
