/** Copyright (c) Scott A Dixon */
import React from "react";

export interface GraphIconButtonProps {
  className?: string;
  disabled?: boolean;
  icon: string;
  label: string;
  mode?: "standard" | "micro";
  variant?: "ghost" | "primary" | "danger";
  onClick: React.MouseEventHandler<HTMLButtonElement>;
}
export interface GraphSliderProps {
  disabled?: boolean;
  formatValue?: (value: number) => string;
  label: string;
  max: number;
  min: number;
  onChange: (value: number) => void;
  onCommit?: () => void;
  step: number;
  title?: string;
  value: number;
}
export interface GraphBadgeProps {
  classifier: "controller";
  className?: string;
  helpText?: string;
  icon?: string;
  label: string;
  shape?: "icon" | "pill";
}
/** Presentation slots apply to every graph child, including custom nodes. */
export interface GraphPresentation {
  IconButton: React.ComponentType<GraphIconButtonProps>;
  Slider: React.ComponentType<GraphSliderProps>;
  Badge: React.ComponentType<GraphBadgeProps>;
}
const nativePresentation: GraphPresentation = {
  IconButton: ({ className, disabled, label, onClick }) => (
    <button
      type="button"
      className={className}
      disabled={disabled}
      aria-label={label}
      title={label}
      onClick={onClick}
    >
      {label}
    </button>
  ),
  Slider: ({
    disabled,
    formatValue,
    label,
    max,
    min,
    onChange,
    onCommit,
    step,
    title,
    value,
  }) => (
    <label title={title}>
      {label}
      <input
        aria-label={label}
        disabled={disabled}
        max={max}
        min={min}
        onChange={(event) => onChange(event.target.valueAsNumber)}
        onPointerUp={onCommit}
        onKeyUp={onCommit}
        step={step}
        type="range"
        value={value}
      />
      <output>{formatValue?.(value) ?? value}</output>
    </label>
  ),
  Badge: ({ className, helpText, label }) => (
    <span className={className} title={helpText}>
      {label}
    </span>
  ),
};
const PresentationContext = React.createContext(nativePresentation);
export function GraphPresentationProvider({
  children,
  value,
}: {
  children: React.ReactNode;
  value?: GraphPresentation;
}): React.JSX.Element {
  const inherited = React.useContext(PresentationContext);
  return (
    <PresentationContext.Provider value={value ?? inherited}>
      {children}
    </PresentationContext.Provider>
  );
}
export function GraphIconButton(
  props: GraphIconButtonProps,
): React.JSX.Element {
  const { IconButton } = React.useContext(PresentationContext);
  return <IconButton {...props} />;
}
export function GraphSlider(props: GraphSliderProps): React.JSX.Element {
  const { Slider } = React.useContext(PresentationContext);
  return <Slider {...props} />;
}
export function GraphBadge(props: GraphBadgeProps): React.JSX.Element {
  const { Badge } = React.useContext(PresentationContext);
  return <Badge {...props} />;
}
