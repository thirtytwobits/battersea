/**
 * Copyright (c) Scott A Dixon
 *
 * The auto-layout control rendered inside the shared canvas controls cluster.
 * It exposes a one-shot "apply" action plus an engine picker popover, replacing
 * the old persistent on/off toggle. Engine selection and the apply action are
 * owned by the feature tab (see the controller contract below); this component
 * only renders the picker and reports intent.
 */
import React from "react";
import { createPortal } from "react-dom";
import { ControlButton } from "@xyflow/react";

import { GraphSlider as Slider } from "./presentation.js";

import type {
  LayoutEngineDescriptor,
  LayoutEngineParameter,
} from "./layout/types.js";

export type AuthoringGraphAutoLayoutStatus = "idle" | "running";

export interface AuthoringGraphAutoLayoutController {
  /** Engines offered for this canvas, in display order. */
  engines: readonly LayoutEngineDescriptor[];
  /** Currently selected engine id; applied when the user presses apply. */
  activeEngineId: string;
  /** Selects a different engine without applying it. */
  onSelectEngine: (engineId: string) => void;
  /** Computes and commits a layout with the given engine. */
  onApply: (engineId: string) => void;
  /** Current values for each engine's parameters, keyed by engine id then parameter id. */
  parameterValues?: Readonly<Record<string, Readonly<Record<string, number>>>>;
  /** Updates one engine parameter value (does not apply); the tab persists it. */
  onParameterChange?: (
    engineId: string,
    parameterId: string,
    value: number,
  ) => void;
  /** `running` while a layout is being computed; disables the apply action. */
  status?: AuthoringGraphAutoLayoutStatus;
  /** Disables the whole control (e.g. while the document is loading). */
  disabled?: boolean;
  /** Accessible label prefix; defaults to "Auto layout". */
  label?: string;
  /** Hides the control entirely when false. */
  visible?: boolean;
}

/** A 0-based ratio parameter (max ≤ 2) reads as a percentage; anything else as a step-precision number. */
function formatLayoutParameterValue(
  value: number,
  parameter: LayoutEngineParameter,
): string {
  if (parameter.min === 0 && parameter.max <= 2) {
    return `${Math.round(value * 100)}%`;
  }
  const stepDecimals =
    parameter.step < 1
      ? (String(parameter.step).split(".")[1]?.length ?? 0)
      : 0;
  return value.toFixed(Math.min(2, stepDecimals));
}

interface AuthoringGraphAutoLayoutControlProps {
  controller?: AuthoringGraphAutoLayoutController;
  paneRef: React.RefObject<HTMLDivElement | null>;
}

export function AuthoringGraphAutoLayoutControl({
  controller,
  paneRef,
}: AuthoringGraphAutoLayoutControlProps): React.JSX.Element | null {
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [anchor, setAnchor] = React.useState<{
    bottom: number;
    left: number;
  } | null>(null);
  const menuButtonRef = React.useRef<HTMLButtonElement | null>(null);
  const popoverRef = React.useRef<HTMLDivElement | null>(null);

  const engines = controller?.engines ?? [];
  const activeEngine =
    engines.find((engine) => engine.id === controller?.activeEngineId) ??
    engines[0];

  const positionMenu = React.useCallback(() => {
    const pane = paneRef.current;
    const button = menuButtonRef.current;
    if (!pane || !button) {
      return;
    }
    const paneRect = pane.getBoundingClientRect();
    const buttonRect = button.getBoundingClientRect();
    setAnchor({
      bottom: paneRect.bottom - buttonRect.bottom,
      left: buttonRect.right - paneRect.left + 10,
    });
  }, [paneRef]);

  React.useEffect(() => {
    if (menuOpen) {
      positionMenu();
    }
  }, [menuOpen, positionMenu]);

  React.useEffect(() => {
    if (!menuOpen) {
      return undefined;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as globalThis.Node | null;
      if (
        popoverRef.current?.contains(target) ||
        menuButtonRef.current?.contains(target)
      ) {
        return;
      }
      setMenuOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMenuOpen(false);
        menuButtonRef.current?.focus();
      }
    };
    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [menuOpen]);

  if (
    !controller ||
    controller.visible === false ||
    engines.length === 0 ||
    !activeEngine
  ) {
    return null;
  }

  const label = controller.label ?? "Auto layout";
  const running = controller.status === "running";
  const disabled = controller.disabled ?? false;
  const canPickEngine = engines.length > 1;

  return (
    <>
      <ControlButton
        aria-label={`${label}: apply ${activeEngine.label}`}
        className="authoring-graph-canvas__auto-layout-apply"
        disabled={disabled || running}
        onClick={() => controller.onApply(activeEngine.id)}
        title={`${label}: apply ${activeEngine.label}`}
        type="button"
      >
        <span
          aria-hidden="true"
          className={
            running
              ? "codicon codicon-loading codicon-modifier-spin"
              : "codicon codicon-play"
          }
        />
      </ControlButton>
      {canPickEngine ? (
        <button
          aria-expanded={menuOpen}
          aria-haspopup="menu"
          aria-label={`${label}: choose engine (${activeEngine.label})`}
          className="react-flow__controls-button authoring-graph-canvas__auto-layout-menu"
          disabled={disabled}
          onClick={() => setMenuOpen((open) => !open)}
          ref={menuButtonRef}
          title={`${label}: choose engine`}
          type="button"
        >
          <span
            aria-hidden="true"
            className={`codicon codicon-${activeEngine.icon}`}
          />
        </button>
      ) : null}
      {menuOpen && canPickEngine && paneRef.current
        ? createPortal(
            <div
              className="authoring-graph-canvas__auto-layout-menu-popover"
              ref={popoverRef}
              role="menu"
              style={{
                bottom: anchor?.bottom ?? 14,
                left: anchor?.left ?? 58,
                visibility: anchor ? "visible" : "hidden",
              }}
            >
              <span className="authoring-graph-canvas__auto-layout-menu-heading">
                {label}
              </span>
              {engines.map((engine) => {
                const selected = engine.id === activeEngine.id;
                return (
                  <button
                    aria-checked={selected}
                    className="authoring-graph-canvas__auto-layout-menu-item"
                    key={engine.id}
                    // Selecting an engine applies it straight away, and the popover
                    // stays open so the chosen engine's parameter sliders appear in
                    // place; it dismisses on click-outside or Escape.
                    onClick={() => {
                      controller.onSelectEngine(engine.id);
                      controller.onApply(engine.id);
                    }}
                    role="menuitemradio"
                    type="button"
                  >
                    <span
                      aria-hidden="true"
                      className={`codicon codicon-${engine.icon} authoring-graph-canvas__auto-layout-menu-icon`}
                    />
                    <span className="authoring-graph-canvas__auto-layout-menu-text">
                      <span className="authoring-graph-canvas__auto-layout-menu-label">
                        {engine.label}
                      </span>
                      <span className="authoring-graph-canvas__auto-layout-menu-description">
                        {engine.description}
                      </span>
                    </span>
                    <span
                      aria-hidden="true"
                      className={
                        selected
                          ? "codicon codicon-check authoring-graph-canvas__auto-layout-menu-check"
                          : "authoring-graph-canvas__auto-layout-menu-check"
                      }
                    />
                  </button>
                );
              })}
              {activeEngine.parameters &&
              activeEngine.parameters.length > 0 &&
              controller.onParameterChange ? (
                <div
                  aria-label={`${activeEngine.label} parameters`}
                  className="authoring-graph-canvas__auto-layout-menu-params"
                  role="group"
                >
                  {activeEngine.parameters.map((parameter) => {
                    const value =
                      controller.parameterValues?.[activeEngine.id]?.[
                        parameter.id
                      ] ?? parameter.defaultValue;
                    return (
                      <Slider
                        disabled={disabled || running}
                        formatValue={(next) =>
                          formatLayoutParameterValue(next, parameter)
                        }
                        key={parameter.id}
                        label={parameter.label}
                        max={parameter.max}
                        min={parameter.min}
                        onChange={(next) =>
                          controller.onParameterChange?.(
                            activeEngine.id,
                            parameter.id,
                            next,
                          )
                        }
                        onCommit={() => controller.onApply(activeEngine.id)}
                        step={parameter.step}
                        title={parameter.description}
                        value={value}
                      />
                    );
                  })}
                </div>
              ) : null}
              <span className="authoring-graph-canvas__auto-layout-menu-foot">
                Drag a slider · releases re-apply.
              </span>
            </div>,
            paneRef.current,
          )
        : null}
    </>
  );
}
