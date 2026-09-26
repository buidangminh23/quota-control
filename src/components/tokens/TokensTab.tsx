/**
 * The Token tab: four views over the core's usage ledger, picked from a colored capsule row. Tổng quan
 * (context windows, the ring, every year, the sources' trends), Lịch sử (any past day), Biểu đồ (time
 * and ranking charts) and Project (use per repository).
 */
import type { ReactNode } from "react";
import { messagesFor } from "@/i18n";
import { VIEW_COLORS } from "@/model/palette";
import { TOKEN_VIEWS, type TokenView } from "@/model/settings";
import { useSettings } from "@/state/hooks";
import { updateSettings, useApp } from "@/state/store";
import { BarChartIcon, ClockIcon, FolderIcon, GaugeIcon } from "../ui/icons";
import { ChartsView } from "./Charts";
import { HistoryView } from "./History";
import { OverviewView } from "./Overview";
import { Capsules, UsageNote } from "./parts";
import { ProjectsView } from "./Projects";

const VIEW_ICONS: Readonly<Record<TokenView, ReactNode>> = {
  overview: <GaugeIcon size={13} />,
  history: <ClockIcon size={13} />,
  charts: <BarChartIcon size={13} />,
  projects: <FolderIcon size={13} />,
};

function CurrentView({ view }: { view: TokenView }) {
  switch (view) {
    case "overview":
      return <OverviewView />;
    case "history":
      return <HistoryView />;
    case "charts":
      return <ChartsView />;
    case "projects":
      return <ProjectsView />;
  }
}

export function TokensTab() {
  const settings = useSettings();
  const messages = messagesFor(settings.language).usage;
  const importing = useApp((state) => state.ledgerInfo?.importing === true);
  return (
    <>
      <Capsules
        stacked
        label={messages.viewsLabel}
        options={TOKEN_VIEWS.map((view) => ({ value: view, label: messages.view(view), icon: VIEW_ICONS[view], color: VIEW_COLORS[view] }))}
        value={settings.tokenView}
        onChange={(view) => updateSettings({ tokenView: view })}
      />
      {importing ? <UsageNote text={messages.importing} /> : null}
      <CurrentView view={settings.tokenView} />
    </>
  );
}
