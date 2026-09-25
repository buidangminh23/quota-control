/**
 * Small form controls in the upstream look: the compact on/off switch, the popup picker that hugs its
 * selection (upstream uses menu pickers because segmented controls do not fit 320pt), and the
 * transient confirmation pill.
 */
import { useRef, type ReactNode } from "react";
import { CheckCircleFill, ChevronUpDown } from "./icons";
import { openMenuAt } from "./menu";
import { tooltipProps } from "./tooltip";

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
  tooltip?: string;
}

export function Switch({ checked, onChange, label, disabled, tooltip }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={`uc-switch${checked ? " is-on" : ""}`}
      onClick={() => onChange(!checked)}
      {...tooltipProps(tooltip)}
    >
      <span className="uc-switch-knob" />
    </button>
  );
}

interface PickerProps<T extends string> {
  value: T;
  options: readonly T[];
  label: (option: T) => string;
  onChange: (value: T) => void;
  ariaLabel: string;
  disabled?: boolean;
}

export function Picker<T extends string>({ value, options, label, onChange, ariaLabel, disabled }: PickerProps<T>) {
  const ref = useRef<HTMLButtonElement>(null);
  const open = () => {
    if (!ref.current) return;
    openMenuAt(
      ref.current,
      options.map((option) => ({ kind: "item" as const, label: label(option), checked: option === value, onSelect: () => onChange(option) })),
      { align: "end", checkable: true },
    );
  };
  return (
    <button
      ref={ref}
      type="button"
      className="uc-picker"
      aria-haspopup="menu"
      aria-label={`${ariaLabel}: ${label(value)}`}
      disabled={disabled}
      onClick={open}
    >
      <span className="uc-truncate">{label(value)}</span>
      <ChevronUpDown size={10} />
    </button>
  );
}

export function Pill({ text, tone }: { text: string; tone: "positive" | "notice" }) {
  return (
    <div className={`uc-pill is-${tone}`} role="status">
      {tone === "positive" ? <CheckCircleFill size={12} /> : null}
      <span>{text}</span>
    </div>
  );
}

export function Button({
  children,
  onClick,
  variant = "bordered",
  disabled,
  className,
  ...rest
}: {
  children: ReactNode;
  onClick: () => void;
  variant?: "bordered" | "prominent" | "destructive" | "plain";
  disabled?: boolean;
  className?: string;
  "aria-label"?: string;
}) {
  return (
    <button type="button" className={`uc-button is-${variant}${className ? ` ${className}` : ""}`} disabled={disabled} onClick={onClick} {...rest}>
      {children}
    </button>
  );
}
