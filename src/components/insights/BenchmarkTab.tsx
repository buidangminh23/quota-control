/**
 * The Benchmark tab: model quality on the user's own projects (Dự án), public leaderboards across
 * every kind of work (Công khai) and a side-by-side comparison (So sánh).
 */
import { useEffect } from "react";
import { insightsFor } from "@/i18n/insights";
import { useSettings } from "@/state/hooks";
import { BENCHMARK_VIEWS, loadQuality, setBenchmarkView, useInsights } from "@/state/insights";
import { CompareModels } from "./CompareModels";
import { MyQuality } from "./MyQuality";
import { Segmented } from "./parts";
import { PublicBoards } from "./PublicBoards";

export function BenchmarkTab() {
  const { language } = useSettings();
  const text = insightsFor(language);
  const view = useInsights((state) => state.view);

  useEffect(() => loadQuality(), []);

  return (
    <>
      <Segmented value={view} options={BENCHMARK_VIEWS} label={text.view} onChange={setBenchmarkView} ariaLabel={text.viewsLabel} />
      {view === "mine" ? <MyQuality /> : view === "public" ? <PublicBoards /> : <CompareModels />}
    </>
  );
}
