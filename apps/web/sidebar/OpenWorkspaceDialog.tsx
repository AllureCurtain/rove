"use client";

import { FileIcon } from "@radix-ui/react-icons";
import {
  FormEvent,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { useCopy } from "../copy/CopyProvider";
import {
  desktopWorkspacePickerAvailable,
  selectDesktopWorkspace,
} from "../platform/desktop-commands";
import { createProductApiClient } from "../product/product-client";
import type { WorkspaceKind } from "../state/product-types";

/**
 * The one workspace-selection flow (design §9.1): the absolute-path field, the
 * folder picker, and the `folder|repo` selector. The home surface embeds it
 * behind its collapsed "enter manually"; `OpenWorkspaceDialog` wraps the same
 * fields in modal chrome for the sidebar `+`.
 *
 * The picker has two backends with the same outcome — the Desktop host's
 * native dialog, or the local API's folder picker. Neither may be present, so
 * an unavailable answer becomes an inline error instead of a hidden button.
 */
export function WorkspaceOpenForm({
  idPrefix,
  submitLabel,
  autoFocus = false,
  actions,
  onOpen,
  onKeyDown,
}: {
  idPrefix: string;
  submitLabel: string;
  autoFocus?: boolean;
  /** Extra buttons rendered before the submit control (the dialog's Cancel). */
  actions?: ReactNode;
  onOpen: (path: string, kind: WorkspaceKind) => void;
  onKeyDown?: (event: KeyboardEvent<HTMLFormElement>) => void;
}) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [path, setPath] = useState("");
  const [kind, setKind] = useState<WorkspaceKind>("folder");
  const [error, setError] = useState<string | null>(null);
  const [pickerBusy, setPickerBusy] = useState(false);
  // A picker result may only land if nothing newer happened: another pick,
  // a manual edit, a submit, or unmount — a late answer never overwrites
  // input the user has since typed.
  const pickerGeneration = useRef(0);

  useEffect(
    () => () => {
      pickerGeneration.current += 1;
    },
    [],
  );

  function invalidatePicker() {
    pickerGeneration.current += 1;
    setPickerBusy(false);
  }

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    invalidatePicker();
    const trimmed = path.trim();
    if (!trimmed) {
      setError(t("empty.pathRequired"));
      return;
    }
    setError(null);
    onOpen(trimmed, kind);
  }

  async function browseWorkspace() {
    const generation = ++pickerGeneration.current;
    setPickerBusy(true);
    setError(null);
    try {
      if (desktopWorkspacePickerAvailable()) {
        const selected = await selectDesktopWorkspace();
        if (generation !== pickerGeneration.current) {
          return;
        }
        if (selected) {
          setPath(selected);
        }
        return;
      }
      const result = await client.pickWorkspaceFolder();
      if (generation !== pickerGeneration.current) {
        return;
      }
      if (result.status === "selected") {
        setPath(result.path);
        return;
      }
      if (result.status === "unavailable") {
        setError(t("empty.pickerUnavailable"));
      }
    } catch (caught) {
      if (generation !== pickerGeneration.current) {
        return;
      }
      setError(
        caught instanceof Error
          ? caught.message
          : t("workspace.openPickerFailed"),
      );
    } finally {
      if (generation === pickerGeneration.current) {
        setPickerBusy(false);
      }
    }
  }

  const pathId = `${idPrefix}-path`;
  const kindId = `${idPrefix}-kind`;
  const errorId = `${idPrefix}-error`;

  return (
    <form onSubmit={handleSubmit} onKeyDown={onKeyDown}>
      <div className="field">
        <label htmlFor={pathId}>{t("empty.absolutePath")}</label>
        <div className="workspace-path-control">
          <input
            id={pathId}
            value={path}
            onChange={(event) => {
              invalidatePicker();
              setPath(event.target.value);
            }}
            placeholder={t("empty.pathPlaceholder")}
            autoFocus={autoFocus}
            aria-invalid={error ? "true" : undefined}
            aria-describedby={error ? errorId : undefined}
          />
          <button
            type="button"
            className="secondary"
            onClick={() => void browseWorkspace()}
            disabled={pickerBusy}
          >
            <FileIcon aria-hidden="true" />
            {pickerBusy ? t("common.loading") : t("empty.browse")}
          </button>
        </div>
      </div>
      <div className="field">
        <label htmlFor={kindId}>{t("empty.kind")}</label>
        <select
          id={kindId}
          value={kind}
          onChange={(event) => {
            invalidatePicker();
            setKind(event.target.value as WorkspaceKind);
          }}
        >
          <option value="folder">{t("empty.folder")}</option>
          <option value="repo">{t("empty.repo")}</option>
        </select>
      </div>
      {error ? (
        <div className="chat-error" id={errorId} role="alert">
          {error}
        </div>
      ) : null}
      <div className="modal-actions">
        {actions}
        <button type="submit">{submitLabel}</button>
      </div>
    </form>
  );
}

/**
 * The sidebar `+` entry into the shared selection flow: a modal shell around
 * `WorkspaceOpenForm` with Escape, a focus loop, and an explicit cancel.
 */
export function OpenWorkspaceDialog({
  onOpen,
  onCancel,
}: {
  onOpen: (path: string, kind: WorkspaceKind) => void;
  onCancel: () => void;
}) {
  const { t } = useCopy();

  function handleKeyDown(event: KeyboardEvent<HTMLFormElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onCancel();
      return;
    }
    if (event.key !== "Tab") {
      return;
    }
    event.stopPropagation();
    const focusable = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(
        "button:not([disabled]), input:not([disabled]), select:not([disabled])",
      ),
    );
    const first = focusable[0];
    const last = focusable.at(-1);
    if (!first || !last) {
      return;
    }
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <div
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="open-workspace-title"
      >
        <h2 id="open-workspace-title">{t("empty.open")}</h2>
        <p className="modal-card__lede">{t("workspace.openDescription")}</p>
        <WorkspaceOpenForm
          idPrefix="open-workspace"
          submitLabel={t("empty.open")}
          autoFocus
          onOpen={onOpen}
          onKeyDown={handleKeyDown}
          actions={
            <button type="button" className="secondary" onClick={onCancel}>
              {t("common.cancel")}
            </button>
          }
        />
      </div>
    </div>
  );
}
