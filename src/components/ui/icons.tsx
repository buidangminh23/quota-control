/**
 * The popup's glyphs, drawn as small inline SVGs in the spirit of the SF Symbols upstream uses
 * (chevron, info, share, star, flame, warning, grip…). All take their color from `currentColor`.
 */
import type { ReactNode, SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement> & { size?: number };

function Svg({ size = 12, children, ...rest }: IconProps & { children: ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden="true" focusable="false" {...rest}>
      {children}
    </svg>
  );
}

const STROKE = { stroke: "currentColor", strokeWidth: 1.8, strokeLinecap: "round", strokeLinejoin: "round" } as const;

export function CheckIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 8.4l3.2 3.1L13 4.6" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" />
    </Svg>
  );
}

export function ChevronUpDown(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5 6.2L8 3.2l3 3M5 9.8l3 3 3-3" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </Svg>
  );
}

export function PowerIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M8 1.8v5.4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M4.6 4a5.2 5.2 0 1 0 6.8 0" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </Svg>
  );
}

export function CloseIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </Svg>
  );
}

export function CopyIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="5.2" y="5.2" width="8.3" height="8.3" rx="1.6" stroke="currentColor" strokeWidth="1.4" />
      <path d="M10.4 3.1a1.3 1.3 0 0 0-1.3-.9H3.6a1.4 1.4 0 0 0-1.4 1.4v5.5a1.3 1.3 0 0 0 .9 1.3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </Svg>
  );
}

export function ChevronDown(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3.5 6l4.5 4.5L12.5 6" {...STROKE} />
    </Svg>
  );
}

export function ChevronUp(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3.5 10L8 5.5l4.5 4.5" {...STROKE} />
    </Svg>
  );
}

export function ChevronLeft(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M10 3.5L5.5 8l4.5 4.5" {...STROKE} />
    </Svg>
  );
}

export function ChevronRight(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M6 3.5l4.5 4.5L6 12.5" {...STROKE} />
    </Svg>
  );
}

export function InfoCircle(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="8" cy="8" r="6.4" stroke="currentColor" strokeWidth="1.3" />
      <path d="M8 7.2v3.9" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="8" cy="5.1" r="0.85" fill="currentColor" />
    </Svg>
  );
}

export function CheckCircleFill(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="8" cy="8" r="7" fill="currentColor" />
      <path d="M4.9 8.2l2.1 2.1 4.1-4.4" stroke="var(--uc-on-accent)" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </Svg>
  );
}

export function StarIcon({ filled, ...props }: IconProps & { filled?: boolean }) {
  return (
    <Svg {...props}>
      <path
        d="M8 1.9l1.8 3.8 4.1.5-3 2.9.8 4.1L8 11.2l-3.7 2 .8-4.1-3-2.9 4.1-.5z"
        fill={filled ? "currentColor" : "none"}
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinejoin="round"
      />
    </Svg>
  );
}

export function FlameIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path
        d="M8.2 1.2c.4 2.2 3.9 3.8 3.9 7.6A4.2 4.2 0 0 1 7.9 13c-2.4 0-4.1-1.7-4.1-4.1 0-1.9 1.1-3 1.9-3.7-.1 1.4.6 2.3 1.4 2.4-.6-2.2.3-4.1 1.1-6.4z"
        fill="currentColor"
      />
    </Svg>
  );
}

export function WarningTriangle(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M7.1 1.9a1 1 0 0 1 1.8 0l6 11.1a1 1 0 0 1-.9 1.5H2a1 1 0 0 1-.9-1.5z" fill="currentColor" />
      <path d="M8 5.6v4" stroke="var(--uc-tray)" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="8" cy="12" r="0.9" fill="var(--uc-tray)" />
    </Svg>
  );
}

export function GripIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 5h10M3 8h10M3 11h10" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </Svg>
  );
}

export function ResetIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3.2 8a4.8 4.8 0 1 0 1.5-3.5" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M4.4 1.9v2.9h2.9" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </Svg>
  );
}

export function UndoIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5.2 3.4L2.4 6.2 5.2 9" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M2.8 6.2h6.4a3.8 3.8 0 0 1 0 7.6H6.6" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </Svg>
  );
}

export function GearIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="8" cy="8" r="2.2" stroke="currentColor" strokeWidth="1.4" />
      <path
        d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </Svg>
  );
}

export function SlidersIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
      <circle cx="10.5" cy="4.5" r="1.6" fill="var(--uc-card-solid)" stroke="currentColor" strokeWidth="1.3" />
      <circle cx="5.5" cy="8" r="1.6" fill="var(--uc-card-solid)" stroke="currentColor" strokeWidth="1.3" />
      <circle cx="9" cy="11.5" r="1.6" fill="var(--uc-card-solid)" stroke="currentColor" strokeWidth="1.3" />
    </Svg>
  );
}

export function PersonIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="8" cy="5.4" r="2.6" stroke="currentColor" strokeWidth="1.4" />
      <path d="M2.9 13.6c.5-2.5 2.5-3.9 5.1-3.9s4.6 1.4 5.1 3.9" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </Svg>
  );
}

export function ChatIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path
        d="M3 3.2h10a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H7.2L4.4 13.4V11.2H3a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1z"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinejoin="round"
      />
    </Svg>
  );
}

export function PlusIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M8 3v10M3 8h10" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </Svg>
  );
}

export function ExternalIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M9.5 2.5h4v4M13.3 2.7L7.5 8.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M12 9.5v3a1 1 0 0 1-1 1H3.5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </Svg>
  );
}

export function GaugeIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M2.6 11.6a5.6 5.6 0 1 1 10.8 0" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      <path d="M8 10.2l2.6-3.4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="8" cy="10.4" r="1.1" fill="currentColor" />
    </Svg>
  );
}

export function ClockIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="8" cy="8" r="6.1" stroke="currentColor" strokeWidth="1.4" />
      <path d="M8 4.6V8l2.3 1.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
    </Svg>
  );
}

export function BarChartIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3.2 13.2V8.6M6.4 13.2V3.6M9.6 13.2V6.6M12.8 13.2V10" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" />
    </Svg>
  );
}

export function FolderIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M2 4.6a1 1 0 0 1 1-1h3.1l1.4 1.5H13a1 1 0 0 1 1 1v6.2a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
    </Svg>
  );
}

export function WindowIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="2" y="2.8" width="12" height="10.4" rx="1.6" stroke="currentColor" strokeWidth="1.4" />
      <path d="M2.4 5.8h11.2" stroke="currentColor" strokeWidth="1.4" />
      <path d="M4.6 9.4h4.2" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </Svg>
  );
}

export function CalendarIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="2.2" y="3.2" width="11.6" height="10.4" rx="1.6" stroke="currentColor" strokeWidth="1.4" />
      <path d="M2.6 6.6h10.8M5.4 1.9v2.4M10.6 1.9v2.4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </Svg>
  );
}

export function TagIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M8.6 2.2H13a.8.8 0 0 1 .8.8v4.4a.8.8 0 0 1-.23.57l-6 6a.8.8 0 0 1-1.14 0L2.03 9.56a.8.8 0 0 1 0-1.14l6-6A.8.8 0 0 1 8.6 2.2z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
      <circle cx="10.9" cy="5.1" r="1.1" fill="currentColor" />
    </Svg>
  );
}

export function Spinner({ size = 11 }: { size?: number }) {
  return (
    <svg className="uc-spinner" width={size} height={size} viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" strokeOpacity="0.25" strokeWidth="2" />
      <path d="M8 2a6 6 0 0 1 6 6" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
