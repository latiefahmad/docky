import type { ReactNode } from "react";

interface CardProps {
  /** Heading above the group, as macOS System Settings labels its groups. */
  title?: string;
  children: ReactNode;
}

export function Card({ title, children }: CardProps) {
  if (!title) return <section className="card">{children}</section>;
  return (
    <section className="card-group" aria-label={title}>
      <h2 className="card-title">{title}</h2>
      <div className="card">{children}</div>
    </section>
  );
}

interface RowProps {
  label: string;
  hint?: string;
  children: ReactNode;
}

export function Row({ label, hint, children }: RowProps) {
  return (
    <div className="row">
      <div className="row-text">
        <span className="row-label">{label}</span>
        {hint ? <span className="row-hint">{hint}</span> : null}
      </div>
      <div className="row-control">{children}</div>
    </div>
  );
}

interface SubRowProps extends RowProps {
  /** Whether the option it depends on is on; the row folds away while it is off. */
  open: boolean;
}

/**
 * An option that only makes sense while the row above it is on. It stays
 * mounted so it can fold open and shut, and is `inert` while folded so the
 * keyboard and screen readers skip it.
 */
export function SubRow({ open, ...row }: SubRowProps) {
  return (
    <div className="row-sub" data-open={open} inert={!open}>
      <div className="row-sub-clip">
        <Row {...row} />
      </div>
    </div>
  );
}

interface ToggleProps {
  checked: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}

export function Toggle({ checked, label, onChange }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="toggle"
      onClick={() => onChange(!checked)}
    >
      <span className="toggle-knob" />
    </button>
  );
}

interface SliderProps {
  value: number;
  min: number;
  max: number;
  label: string;
  before?: string;
  after?: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}

export function Slider({ value, min, max, label, before, after, disabled, onChange }: SliderProps) {
  const filled = ((value - min) / (max - min)) * 100;
  return (
    <div className="slider">
      {before ? <span className="slider-edge">{before}</span> : null}
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        value={value}
        disabled={disabled}
        style={{ "--filled": `${filled}%` } as React.CSSProperties}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      {after ? <span className="slider-edge">{after}</span> : null}
    </div>
  );
}

interface SegmentedProps<T extends string> {
  value: T;
  label: string;
  options: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
}

export function Segmented<T extends string>({ value, label, options, onChange }: SegmentedProps<T>) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          className="segment"
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
