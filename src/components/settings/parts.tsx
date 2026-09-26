/** The Settings screen's building blocks: a titled card of rows, a labelled row, an inline notice. */
import type { ReactNode } from "react";
import { WarningTriangle } from "../ui/icons";

export function Section({ title, children, warning }: { title: string; children: ReactNode; warning?: boolean }) {
  return (
    <section className="uc-group">
      <h2 className="uc-group-title">
        {title}
        {warning ? (
          <span className="uc-inline-icon" style={{ color: "var(--uc-orange)" }}>
            <WarningTriangle size={10} />
          </span>
        ) : null}
      </h2>
      <div className="uc-card uc-settings-card">{children}</div>
    </section>
  );
}

export function Row({ label, children, note, nested }: { label: string; children: ReactNode; note?: string; nested?: boolean }) {
  return (
    <div className="uc-settings-row-group">
      <div className={`uc-settings-row${nested ? " is-nested" : ""}`}>
        <span className="uc-settings-label">{label}</span>
        {children}
      </div>
      {note ? <p className={`uc-settings-note${nested ? " is-nested" : ""}`}>{note}</p> : null}
    </div>
  );
}

export function InlineNotice({ text }: { text: string }) {
  return <p className="uc-settings-notice">{text}</p>;
}
