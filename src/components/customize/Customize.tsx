/**
 * The Customize screen (upstream `CustomizeView`): the provider list with enable switches and drag
 * reorder, and per provider the Always Visible / On Demand metric sections with stars (taskbar) and
 * switches. Every change is undoable (Ctrl+Z) except opening a provider.
 */
import { useMemo, useState, type ReactNode } from "react";
import {
  closestCenter,
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragOverEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import { arrayMove, SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { messagesFor, type Language } from "@/i18n";
import type { WidgetDescriptor } from "@/lib/types";
import {
  applyMetricSections,
  canPin,
  customizeDetail,
  customizeRows,
  isMetricEnabled,
  isPinned,
  MAX_PINS_PER_PROVIDER,
  reorderProvider,
  setMetricEnabled,
  setPinned,
  type ProviderRow,
} from "@/model/layout";
import { providerBrand, providerTitle } from "@/model/providerText";
import { descriptorTitle } from "@/model/widgetData";
import { useBarKind, useIsEnabled, useLanguage } from "@/state/hooks";
import { navigate, openCustomizeDetail, setProviderEnabled, showNotice, updateLayout, useApp } from "@/state/store";
import { Switch } from "../ui/controls";
import { ChevronRight, GearIcon, GripIcon, StarIcon } from "../ui/icons";
import { ProviderMark } from "../ui/ProviderMark";
import { tooltipProps } from "../ui/tooltip";

const DRAG_DISTANCE = 4;

function useDragSensors() {
  return useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: DRAG_DISTANCE } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
}

function sortableStyle(transform: { x: number; y: number } | null, transition: string | undefined, dragging: boolean) {
  return {
    transform: transform ? `translate3d(${Math.round(transform.x)}px, ${Math.round(transform.y)}px, 0)` : undefined,
    transition,
    opacity: dragging ? 0 : 1,
  };
}

export function Customize() {
  const providerId = useApp((state) => state.customizeProviderId);
  return providerId ? <ProviderDetail providerId={providerId} /> : <ProviderList />;
}

export function CrossLink({ icon, title, subtitle, onClick }: { icon: ReactNode; title: string; subtitle: string; onClick: () => void }) {
  return (
    <button type="button" className="uc-card uc-crosslink" onClick={onClick}>
      <span className="uc-crosslink-icon">{icon}</span>
      <span className="uc-crosslink-text">
        <span className="uc-crosslink-title">{title}</span>
        <span className="uc-crosslink-subtitle">{subtitle}</span>
      </span>
      <ChevronRight size={12} className="uc-tertiary" />
    </button>
  );
}

function ProviderList() {
  const language = useLanguage();
  const messages = messagesFor(language);
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const isEnabled = useIsEnabled();
  const rows = useMemo(() => customizeRows(layout, catalog, isEnabled), [layout, catalog, isEnabled]);
  const sensors = useDragSensors();
  const [activeId, setActiveId] = useState<string | null>(null);
  const enabledIds = rows.filter((row) => row.enabled).map((row) => row.provider.id);
  const activeRow = rows.find((row) => row.provider.id === activeId);

  const onDragEnd = ({ active, over }: DragEndEvent) => {
    setActiveId(null);
    if (!over || active.id === over.id) return;
    updateLayout((current) => reorderProvider(current, catalog, isEnabled, String(active.id), String(over.id)));
  };

  return (
    <div className="uc-stack">
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragStart={({ active }: DragStartEvent) => setActiveId(String(active.id))}
        onDragEnd={onDragEnd}
        onDragCancel={() => setActiveId(null)}
      >
        <SortableContext items={enabledIds} strategy={verticalListSortingStrategy}>
          <div className="uc-card uc-list-card">
            {rows.map((row) => (
              <SortableProviderRow key={row.provider.id} row={row} language={language} />
            ))}
          </div>
        </SortableContext>
        <DragOverlay>{activeRow ? <ProviderRowView row={activeRow} language={language} lifted /> : null}</DragOverlay>
      </DndContext>
      <CrossLink
        icon={<GearIcon size={15} />}
        title={messages.customize.settingsLinkTitle}
        subtitle={messages.customize.settingsLinkSubtitle}
        onClick={() => navigate("settings")}
      />
    </div>
  );
}

function SortableProviderRow({ row, language }: { row: ProviderRow; language: Language }) {
  const sortable = useSortable({ id: row.provider.id, disabled: !row.enabled });
  return (
    <div ref={sortable.setNodeRef} style={sortableStyle(sortable.transform, sortable.transition, sortable.isDragging)}>
      <ProviderRowView
        row={row}
        language={language}
        handle={
          <span
            ref={sortable.setActivatorNodeRef}
            className={`uc-grip${row.enabled ? "" : " is-disabled"}`}
            {...(row.enabled ? { ...sortable.attributes, ...sortable.listeners } : {})}
            aria-label={messagesFor(language).customize.reorder}
          >
            <GripIcon size={12} />
          </span>
        }
      />
    </div>
  );
}

function ProviderRowView({ row, language, handle, lifted }: { row: ProviderRow; language: Language; handle?: ReactNode; lifted?: boolean }) {
  const messages = messagesFor(language);
  const title = providerTitle(row.provider, language);
  const open = () => openCustomizeDetail(row.provider.id);
  return (
    <div className={`uc-list-row${row.enabled ? "" : " is-disabled"}${lifted ? " is-lifted" : ""}`}>
      {handle ?? (
        <span className="uc-grip">
          <GripIcon size={12} />
        </span>
      )}
      <button type="button" className="uc-list-row-main" onClick={open}>
        <span className="uc-list-mark">
          <ProviderMark brand={providerBrand(row.provider)} size={18} />
        </span>
        <span className="uc-list-text">
          <span className="uc-list-title uc-truncate">{title}</span>
          <span className="uc-list-subtitle">{messages.customize.metricCount(row.metricCount)}</span>
        </span>
      </button>
      <Switch checked={row.enabled} label={messages.customize.enable(title)} onChange={(enabled) => setProviderEnabled(row.provider.id, enabled)} />
      <button type="button" className="uc-chevron-button" aria-label={title} onClick={open}>
        <ChevronRight size={12} />
      </button>
    </div>
  );
}

type Section = "always" | "onDemand";
interface Sections {
  always: string[];
  onDemand: string[];
}

function sectionOf(sections: Sections, id: string): Section | null {
  if (id === "always" || id === "onDemand") return id;
  if (sections.always.includes(id)) return "always";
  if (sections.onDemand.includes(id)) return "onDemand";
  return null;
}

function ProviderDetail({ providerId }: { providerId: string }) {
  const language = useLanguage();
  const messages = messagesFor(language);
  const catalog = useApp((state) => state.catalog);
  const layout = useApp((state) => state.layout);
  const detail = useMemo(() => customizeDetail(layout, catalog, providerId), [layout, catalog, providerId]);
  const sensors = useDragSensors();
  const [dragging, setDragging] = useState<Sections | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  if (!detail) return null;
  const byId = new Map([...detail.always, ...detail.onDemand].map((descriptor) => [descriptor.id, descriptor]));
  const base: Sections = { always: detail.always.map((d) => d.id), onDemand: detail.onDemand.map((d) => d.id) };
  const sections = dragging ?? base;

  const onDragOver = ({ active, over }: DragOverEvent) => {
    if (!over || !dragging) return;
    const from = sectionOf(dragging, String(active.id));
    const to = sectionOf(dragging, String(over.id));
    if (!from || !to || from === to) return;
    const moving = String(active.id);
    const target = dragging[to];
    const overIndex = target.indexOf(String(over.id));
    const insertAt = overIndex >= 0 ? overIndex : target.length;
    setDragging({
      ...dragging,
      [from]: dragging[from].filter((id) => id !== moving),
      [to]: [...target.slice(0, insertAt), moving, ...target.slice(insertAt)],
    });
  };

  const onDragEnd = ({ active, over }: DragEndEvent) => {
    const current = dragging;
    setDragging(null);
    setActiveId(null);
    if (!current || !over) return;
    const moving = String(active.id);
    const section = sectionOf(current, moving);
    let final = current;
    if (section) {
      const list = current[section];
      const from = list.indexOf(moving);
      const to = list.indexOf(String(over.id));
      if (from >= 0 && to >= 0 && from !== to) final = { ...current, [section]: arrayMove(list, from, to) };
    }
    updateLayout((currentLayout) => applyMetricSections(currentLayout, providerId, final.always, final.onDemand, moving));
  };

  const activeDescriptor = activeId ? byId.get(activeId) : undefined;
  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragStart={({ active }) => {
        setActiveId(String(active.id));
        setDragging(base);
      }}
      onDragOver={onDragOver}
      onDragEnd={onDragEnd}
      onDragCancel={() => {
        setDragging(null);
        setActiveId(null);
      }}
    >
      <div className="uc-stack">
        {(["always", "onDemand"] as const).map((section) => (
          <MetricSection
            key={section}
            section={section}
            title={section === "always" ? messages.customize.alwaysVisible : messages.customize.onDemand}
            ids={sections[section]}
            byId={byId}
            language={language}
          />
        ))}
      </div>
      <DragOverlay>{activeDescriptor ? <MetricRowView descriptor={activeDescriptor} language={language} lifted /> : null}</DragOverlay>
    </DndContext>
  );
}

function MetricSection({ section, title, ids, byId, language }: { section: Section; title: string; ids: string[]; byId: Map<string, WidgetDescriptor>; language: Language }) {
  const droppable = useDroppable({ id: section });
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">{title}</h2>
      <SortableContext id={section} items={ids} strategy={verticalListSortingStrategy}>
        <div ref={droppable.setNodeRef} className="uc-card uc-list-card">
          {ids.length === 0 ? (
            <div className="uc-drop-zone">{messagesFor(language).customize.dragHere}</div>
          ) : (
            ids.map((id) => {
              const descriptor = byId.get(id);
              return descriptor ? <SortableMetricRow key={id} descriptor={descriptor} language={language} /> : null;
            })
          )}
        </div>
      </SortableContext>
    </section>
  );
}

function SortableMetricRow({ descriptor, language }: { descriptor: WidgetDescriptor; language: Language }) {
  const sortable = useSortable({ id: descriptor.id });
  return (
    <div ref={sortable.setNodeRef} style={sortableStyle(sortable.transform, sortable.transition, sortable.isDragging)}>
      <MetricRowView
        descriptor={descriptor}
        language={language}
        handle={
          <span ref={sortable.setActivatorNodeRef} className="uc-grip" {...sortable.attributes} {...sortable.listeners} aria-label={messagesFor(language).customize.reorder}>
            <GripIcon size={12} />
          </span>
        }
      />
    </div>
  );
}

function MetricRowView({ descriptor, language, handle, lifted }: { descriptor: WidgetDescriptor; language: Language; handle?: ReactNode; lifted?: boolean }) {
  const messages = messagesFor(language);
  const bar = useBarKind();
  const layout = useApp((state) => state.layout);
  const catalog = useApp((state) => state.catalog);
  const snapshot = useApp((state) => state.engine?.providers[descriptor.providerId]?.snapshot);
  const title = descriptorTitle(descriptor, snapshot, language);
  const enabled = isMetricEnabled(layout, descriptor.id);
  const pinned = isPinned(layout, descriptor.id);
  const toggleStar = () => {
    if (!pinned && !canPin(layout, catalog, descriptor.id)) {
      showNotice(messages.dashboard.pinLimit(MAX_PINS_PER_PROVIDER), "notice");
      return;
    }
    updateLayout((current) => setPinned(current, catalog, descriptor.id, !pinned));
    showNotice(pinned ? messages.customize.unstarred(bar) : messages.customize.starred(bar), "positive");
  };
  return (
    <div className={`uc-list-row is-metric${lifted ? " is-lifted" : ""}`}>
      {handle ?? (
        <span className="uc-grip">
          <GripIcon size={12} />
        </span>
      )}
      <span className="uc-list-metric-title uc-truncate">{title}</span>
      {descriptor.pinnable ? (
        <button
          type="button"
          className={`uc-star-button${pinned ? " is-on" : ""}`}
          aria-pressed={pinned}
          aria-label={pinned ? messages.customize.unstar : messages.customize.star(bar)}
          onClick={toggleStar}
          {...tooltipProps(pinned ? messages.customize.unstar : messages.customize.star(bar))}
        >
          <StarIcon size={12} filled={pinned} />
        </button>
      ) : (
        <span className="uc-star-placeholder" aria-hidden="true" />
      )}
      <Switch checked={enabled} label={title} onChange={(on) => updateLayout((current) => setMetricEnabled(current, descriptor.id, on))} />
    </div>
  );
}
