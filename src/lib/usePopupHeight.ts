import { useLayoutEffect, useRef, type RefObject } from "react";
import { backend } from "./backend";

type Measurement = { height: number; viewport: string; revision: number };

export function usePopupHeight(topRef: RefObject<HTMLElement | null>, contentRef: RefObject<HTMLElement | null>, footerRef: RefObject<HTMLElement | null>, view: string, visible: boolean): void {
  const reportRef = useRef<((force?: boolean) => void) | null>(null);
  useLayoutEffect(() => {
    let disposed = false;
    let frame = 0;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let inFlight = false;
    let dirty = false;
    let revision = 0;
    let successful: Measurement | undefined;
    let attemptedKey = "";
    let blockedKey = "";
    let failures = 0;
    const observed = new Set<HTMLElement>();
    const viewport = () => `${window.innerWidth}:${window.innerHeight}:${window.devicePixelRatio}`;
    const measure = (): Measurement => ({
      height: Math.ceil((topRef.current?.offsetHeight ?? 0) + (contentRef.current?.offsetHeight ?? 0) + (footerRef.current?.offsetHeight ?? 0)),
      viewport: viewport(),
      revision,
    });
    const observer = new ResizeObserver(() => report());
    const report = (force = false) => {
      if (disposed) return;
      if (force) revision += 1;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const current = new Set([topRef.current, contentRef.current, footerRef.current].filter((element): element is HTMLElement => element !== null));
        for (const element of observed) if (!current.has(element)) {
          observer.unobserve(element);
          observed.delete(element);
        }
        for (const element of current) if (!observed.has(element)) {
          observer.observe(element);
          observed.add(element);
        }
        if (inFlight) {
          dirty = true;
          return;
        }
        const measurement = measure();
        if (measurement.height <= 0) return;
        if (successful?.height === measurement.height && successful.viewport === measurement.viewport && successful.revision === measurement.revision) return;
        const key = `${measurement.height}:${measurement.viewport}:${measurement.revision}`;
        if (key === blockedKey) return;
        if (key !== attemptedKey) {
          clearTimeout(retryTimer);
          retryTimer = undefined;
          attemptedKey = key;
          failures = 0;
        }
        if (retryTimer !== undefined) return;
        inFlight = true;
        void backend().resizePopup(measurement.height).then(() => {
          if (disposed) return;
          successful = measurement;
          attemptedKey = "";
          blockedKey = "";
          failures = 0;
        }).catch((error: unknown) => {
          if (disposed) return;
          console.error("Resizing the popup failed", error);
          const delay = [250, 1_000, 3_000][failures++];
          if (delay === undefined) blockedKey = key;
          else retryTimer = setTimeout(() => {
            retryTimer = undefined;
            report();
          }, delay);
        }).finally(() => {
          inFlight = false;
          if (!disposed && dirty) {
            dirty = false;
            report();
          }
        });
      });
    };
    const onResize = () => report();
    reportRef.current = report;
    window.addEventListener("resize", onResize);
    report();
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      clearTimeout(retryTimer);
      observer.disconnect();
      window.removeEventListener("resize", onResize);
      if (reportRef.current === report) reportRef.current = null;
    };
  }, [topRef, contentRef, footerRef]);
  useLayoutEffect(() => {
    reportRef.current?.(visible);
  }, [view, visible]);
}
