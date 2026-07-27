"use client";

import {
  ActivityLogIcon,
  ArchiveIcon,
  CheckCircledIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  CodeIcon,
  CopyIcon,
  Cross2Icon,
  DesktopIcon,
  DotsHorizontalIcon,
  ExclamationTriangleIcon,
  FileIcon,
  GearIcon,
  HamburgerMenuIcon,
  InfoCircledIcon,
  LockClosedIcon,
  MagnifyingGlassIcon,
  MixerHorizontalIcon,
  MoonIcon,
  PaperPlaneIcon,
  PlusIcon,
  ReaderIcon,
  ReloadIcon,
  RowsIcon,
  StopIcon,
  SunIcon,
  TrashIcon,
} from "@radix-ui/react-icons";
import {
  FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  INITIAL_PRODUCT_UI_V2_MOCK_SESSION_ID,
  PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT,
  PRODUCT_UI_V2_MOCK_SESSIONS,
  PRODUCT_UI_V2_PREVIEW_BOUNDARY,
  type MockTranscriptEntry,
  type ProductUiV2MockSession,
} from "./product-ui-v2-mock";
import styles from "./product-ui-v2.module.css";

type Theme = "light" | "dark";
type View = "chat" | "settings";
type ComposerMode = "steer" | "follow-up";
type ApprovalState = "pending" | "approved" | "rejected";
type SettingsSection = "providers" | "approvals" | "memory" | "sessions" | "desktop";

const settingsSections: Array<{
  id: SettingsSection;
  label: string;
  icon: ReactNode;
}> = [
  { id: "providers", label: "Providers & models", icon: <RowsIcon /> },
  { id: "approvals", label: "Tools & approvals", icon: <LockClosedIcon /> },
  { id: "memory", label: "Memory", icon: <ArchiveIcon /> },
  { id: "sessions", label: "Sessions", icon: <ReaderIcon /> },
  { id: "desktop", label: "Desktop host", icon: <DesktopIcon /> },
];

function trapDialogFocus(event: ReactKeyboardEvent<HTMLElement>) {
  if (event.key !== "Tab") {
    return;
  }

  const focusable = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((element) => element.getClientRects().length > 0);

  const first = focusable.at(0);
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

export function ProductUiV2Preview() {
  const [theme, setTheme] = useState<Theme>("light");
  const [view, setView] = useState<View>("chat");
  const [mobileRailOpen, setMobileRailOpen] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [activeSessionId, setActiveSessionId] = useState<string>(
    INITIAL_PRODUCT_UI_V2_MOCK_SESSION_ID,
  );
  const focusSelectedSessionHeadingRef = useRef(false);
  const activeSession =
    PRODUCT_UI_V2_MOCK_SESSIONS.find((session) => session.id === activeSessionId) ??
    PRODUCT_UI_V2_MOCK_SESSIONS[0];
  const overlayOpen = mobileRailOpen || inspectorOpen;

  function restoreFocus(label: string) {
    window.setTimeout(() => {
      document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)?.focus();
    }, 0);
  }

  function closeWorkspaceRail() {
    setMobileRailOpen(false);
    restoreFocus("Open workspaces");
  }

  function closeInspector() {
    setInspectorOpen(false);
    restoreFocus("Open run evidence");
  }

  function selectSession(sessionId: string) {
    focusSelectedSessionHeadingRef.current = mobileRailOpen;
    setActiveSessionId(sessionId);
    setMobileRailOpen(false);
    setInspectorOpen(false);
  }

  useEffect(() => {
    if (!focusSelectedSessionHeadingRef.current || mobileRailOpen) {
      return;
    }
    focusSelectedSessionHeadingRef.current = false;
    const focusTimer = window.setTimeout(() => {
      document.querySelector<HTMLHeadingElement>("#v2-main h1")?.focus();
    }, 0);
    return () => window.clearTimeout(focusTimer);
  }, [activeSessionId, mobileRailOpen]);

  useEffect(() => {
    if (!overlayOpen) {
      return;
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key !== "Escape") {
        return;
      }
      if (mobileRailOpen) {
        closeWorkspaceRail();
      } else {
        closeInspector();
      }
    }

    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [inspectorOpen, mobileRailOpen, overlayOpen]);

  return (
    <div className={styles.preview} data-theme={theme} data-view={view}>
      <a className={styles.skipLink} href="#v2-main">
        Skip to main content
      </a>
      <ProductBar
        inactive={overlayOpen}
        theme={theme}
        view={view}
        onThemeChange={setTheme}
        onViewChange={(nextView) => {
          setView(nextView);
          setMobileRailOpen(false);
          setInspectorOpen(false);
        }}
        onToggleRail={() => {
          if (view === "settings") {
            setView("chat");
            setInspectorOpen(false);
            setMobileRailOpen(true);
            return;
          }
          setMobileRailOpen((open) => !open);
        }}
      />

      {view === "chat" ? (
        <div className={styles.shell}>
          <WorkspaceRail
            activeSessionId={activeSession.id}
            inactive={inspectorOpen}
            open={mobileRailOpen}
            onClose={closeWorkspaceRail}
            onSelectSession={selectSession}
            sessions={PRODUCT_UI_V2_MOCK_SESSIONS}
          />
          <ChatSurface
            inactive={overlayOpen}
            onOpenInspector={() => setInspectorOpen(true)}
            session={activeSession}
          />
          <EvidenceInspector
            inactive={mobileRailOpen}
            open={inspectorOpen}
            onClose={closeInspector}
            session={activeSession}
          />
        </div>
      ) : (
        <SettingsSurface theme={theme} onThemeChange={setTheme} />
      )}

      {mobileRailOpen || inspectorOpen ? (
        <button
          type="button"
          className={styles.mobileScrim}
          aria-label="Close open panel"
          tabIndex={-1}
          onClick={() => {
            if (mobileRailOpen) {
              closeWorkspaceRail();
            } else {
              closeInspector();
            }
          }}
        />
      ) : null}
    </div>
  );
}

function ProductBar({
  inactive,
  theme,
  view,
  onThemeChange,
  onViewChange,
  onToggleRail,
}: {
  inactive: boolean;
  theme: Theme;
  view: View;
  onThemeChange: (theme: Theme) => void;
  onViewChange: (view: View) => void;
  onToggleRail: () => void;
}) {
  return (
    <header className={styles.productBar} inert={inactive ? true : undefined}>
      <div className={styles.brandGroup}>
        <button
          type="button"
          className={`${styles.iconButton} ${styles.mobileOnly}`}
          aria-label="Open workspaces"
          title="Open workspaces"
          onClick={onToggleRail}
        >
          <HamburgerMenuIcon />
        </button>
        <span className={styles.brandMark} aria-hidden="true">
          R
        </span>
        <span className={styles.brandName}>rove</span>
        <span
          className={styles.previewFlag}
          aria-label={PRODUCT_UI_V2_PREVIEW_BOUNDARY}
          title={PRODUCT_UI_V2_PREVIEW_BOUNDARY}
        >
          Inert UI mock
        </span>
      </div>

      <div className={styles.viewSwitch} aria-label="Preview surface">
        <button
          type="button"
          data-active={view === "chat"}
          title="Chat preview"
          onClick={() => onViewChange("chat")}
        >
          <ActivityLogIcon />
          Chat
        </button>
        <button
          type="button"
          data-active={view === "settings"}
          title="Settings preview"
          onClick={() => onViewChange("settings")}
        >
          <GearIcon />
          Settings
        </button>
      </div>

      <div className={styles.productActions}>
        <span className={styles.runtimeState}>
          <span aria-hidden="true" />
          local runtime
        </span>
        <button
          type="button"
          className={styles.iconButton}
          onClick={() => onThemeChange(theme === "light" ? "dark" : "light")}
          aria-label={theme === "light" ? "Use dark theme" : "Use light theme"}
          title={theme === "light" ? "Use dark theme" : "Use light theme"}
        >
          {theme === "light" ? <MoonIcon /> : <SunIcon />}
        </button>
      </div>
    </header>
  );
}

function WorkspaceRail({
  activeSessionId,
  inactive,
  open,
  onClose,
  onSelectSession,
  sessions,
}: {
  activeSessionId: string;
  inactive: boolean;
  open: boolean;
  onClose: () => void;
  onSelectSession: (sessionId: string) => void;
  sessions: ReadonlyArray<ProductUiV2MockSession>;
}) {
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) {
      closeButtonRef.current?.focus();
    }
  }, [open]);

  return (
    <aside
      className={styles.workspaceRail}
      data-open={open}
      aria-label="Workspaces and sessions"
      aria-modal={open ? true : undefined}
      inert={inactive ? true : undefined}
      onKeyDown={open ? trapDialogFocus : undefined}
      role={open ? "dialog" : undefined}
    >
      <div className={styles.railHeading}>
        <div>
          <span className={styles.sectionLabel}>Workspaces</span>
          <strong>Local roots</strong>
        </div>
        <button
          type="button"
          className={styles.iconButton}
          aria-label="Open workspace"
          title="Open workspace"
        >
          <PlusIcon />
        </button>
        <button
          ref={closeButtonRef}
          type="button"
          className={`${styles.iconButton} ${styles.mobileOnly}`}
          aria-label="Close workspaces"
          title="Close workspaces"
          onClick={onClose}
        >
          <Cross2Icon />
        </button>
      </div>

      <button type="button" className={styles.railSearch}>
        <MagnifyingGlassIcon />
        Search sessions
        <kbd>Ctrl K</kbd>
      </button>

      <nav className={styles.workspaceList} aria-label="Workspace catalog">
        <section className={styles.workspaceGroup} data-active="true">
          <button type="button" className={styles.workspaceRow}>
            <span className={styles.workspaceGlyph}>rv</span>
            <span>
              <strong>rove</strong>
              <small>{PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT}</small>
            </span>
            <ChevronDownIcon />
          </button>
          <ul className={styles.sessionList} aria-label="Sessions in rove">
            {sessions.map((session) => {
              const active = session.id === activeSessionId;
              return (
                <li key={session.id}>
                  <button
                    type="button"
                    className={styles.sessionRow}
                    data-active={active}
                    data-session-id={session.id}
                    aria-controls="v2-main"
                    aria-current={active ? "page" : undefined}
                    onClick={() => onSelectSession(session.id)}
                  >
                    <span
                      className={styles.sessionState}
                      data-state={session.status}
                      aria-hidden="true"
                    />
                    <span>
                      <strong>{session.title}</strong>
                      <small>
                        <span>{session.statusLabel}</span>
                        <time dateTime={session.updatedDateTime}>{session.updatedAt}</time>
                      </small>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
          <button type="button" className={styles.newSessionButton}>
            <PlusIcon /> New session
          </button>
        </section>

        <section className={styles.workspaceGroup}>
          <button type="button" className={styles.workspaceRow}>
            <span className={styles.workspaceGlyph}>pi</span>
            <span>
              <strong>pi-web-reference</strong>
              <small>Reference only</small>
            </span>
            <ChevronRightIcon />
          </button>
        </section>
      </nav>

      <div className={styles.railFooter}>
        <div className={styles.boundaryNote}>
          <LockClosedIcon />
          <span>
            <strong>Bounded workspace</strong>
            <small>Reads and writes stay inside rove</small>
          </span>
        </div>
        <button type="button" className={styles.railFooterButton}>
          <GearIcon /> Product settings
        </button>
      </div>
    </aside>
  );
}

function ChatSurface({
  inactive,
  onOpenInspector,
  session,
}: {
  inactive: boolean;
  onOpenInspector: () => void;
  session: ProductUiV2MockSession;
}) {
  const [approval, setApproval] = useState<ApprovalState>("pending");
  const [toolOpen, setToolOpen] = useState(true);
  const [composerMode, setComposerMode] = useState<ComposerMode>(
    session.composer.canSteer ? "steer" : "follow-up",
  );
  const [draft, setDraft] = useState("");
  const [queued, setQueued] = useState(false);
  const transcriptRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (transcriptRef.current) {
      transcriptRef.current.scrollTop = transcriptRef.current.scrollHeight;
    }
    setApproval("pending");
    setToolOpen(true);
    setComposerMode(session.composer.canSteer ? "steer" : "follow-up");
    setDraft("");
    setQueued(false);
  }, [session.composer.canSteer, session.id]);

  function handleComposerSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!draft.trim()) {
      return;
    }
    setQueued(true);
    setDraft("");
  }

  return (
    <main
      id="v2-main"
      className={styles.chatSurface}
      data-session-id={session.id}
      inert={inactive ? true : undefined}
      aria-labelledby={`session-heading-${session.id}`}
    >
      <header className={styles.chatHeader}>
        <div className={styles.chatIdentity}>
          <span className={styles.sectionLabel}>rove / {session.branch}</span>
          <h1 id={`session-heading-${session.id}`} tabIndex={-1}>{session.title}</h1>
        </div>
        <div className={styles.chatHeaderMeta}>
          <span className={styles.runStatus} data-state={session.status}>
            <span aria-hidden="true" /> {session.headerStatusLabel}
          </span>
          <button
            type="button"
            className={`${styles.iconButton} ${styles.inspectorTrigger}`}
            aria-label="Open run evidence"
            title="Open run evidence"
            onClick={onOpenInspector}
          >
            <ActivityLogIcon />
          </button>
          <button
            type="button"
            className={styles.iconButton}
            aria-label="Session actions"
            title="Session actions"
          >
            <DotsHorizontalIcon />
          </button>
        </div>
      </header>

      <div
        ref={transcriptRef}
        className={styles.transcript}
        role="log"
        aria-label={`${session.title} mock chronological conversation`}
      >
        <div className={styles.previewBoundary} role="note">
          <InfoCircledIcon />
          <span>{PRODUCT_UI_V2_PREVIEW_BOUNDARY}</span>
        </div>
        {session.transcript.map((entry, index) => (
          <SessionTimelineEntry
            key={entry.id}
            approval={approval}
            entry={entry}
            last={index === session.transcript.length - 1}
            onApprovalChange={setApproval}
            onToolToggle={() => setToolOpen((value) => !value)}
            sessionId={session.id}
            toolOpen={toolOpen}
          />
        ))}
      </div>

      <form className={styles.composer} onSubmit={handleComposerSubmit}>
        <div className={styles.composerStatus} data-state={session.status}>
          <span>
            <span className={styles.activeDot} data-state={session.status} aria-hidden="true" />
            {session.composer.statusLabel}
          </span>
          <span>
            {queued ? "Mock instruction queued at the next safe boundary" : session.composer.helper}
          </span>
        </div>
        <textarea
          value={draft}
          onChange={(event) => {
            setDraft(event.target.value);
            setQueued(false);
          }}
          placeholder={
            composerMode === "steer" ? session.composer.placeholder : "Queue the next instruction..."
          }
          aria-label={composerMode === "steer" ? "Steer active run" : "Queue follow-up"}
        />
        <div className={styles.composerBar}>
          <div className={styles.composerTools}>
            <button type="button" className={styles.iconButton} aria-label="Attach context" title="Attach context">
              <PlusIcon />
            </button>
            <button type="button" className={styles.modelButton}>
              <span>local / configured-long-context-model</span> <ChevronDownIcon />
            </button>
            <button type="button" className={styles.modelButton}>
              <span>balanced reasoning</span> <ChevronDownIcon />
            </button>
          </div>
          <div className={styles.composerCommit}>
            <div className={styles.modeSwitch} aria-label="Instruction timing">
              <button type="button" data-active={composerMode === "steer"} disabled={!session.composer.canSteer} onClick={() => setComposerMode("steer")}>Steer</button>
              <button type="button" data-active={composerMode === "follow-up"} onClick={() => setComposerMode("follow-up")}>Follow-up</button>
            </div>
            <button type="button" className={styles.stopButton} disabled={!session.composer.canStop} aria-label="Stop run" title="Stop run"><StopIcon /></button>
            <button type="submit" className={styles.sendButton} disabled={!draft.trim()} aria-label="Send instruction" title="Send instruction"><PaperPlaneIcon /></button>
          </div>
        </div>
      </form>
    </main>
  );
}

function SessionTimelineEntry({
  approval,
  entry,
  last,
  onApprovalChange,
  onToolToggle,
  sessionId,
  toolOpen,
}: {
  approval: ApprovalState;
  entry: MockTranscriptEntry;
  last: boolean;
  onApprovalChange: (state: ApprovalState) => void;
  onToolToggle: () => void;
  sessionId: string;
  toolOpen: boolean;
}) {
  if (entry.kind === "message") {
    return (
      <TimelineItem meta={entry.meta} state={entry.state} last={last} event={entry.event}>
        <article
          className={entry.actor === "user" ? styles.userMessage : styles.assistantMessage}
          data-streaming={entry.streaming || undefined}
        >
          <MessageByline label={entry.byline} detail={entry.detail} />
          <p>{entry.text}</p>
          {entry.streaming ? (
            <span className={styles.streamingCursor} aria-label="Response is streaming" />
          ) : null}
        </article>
      </TimelineItem>
    );
  }

  if (entry.kind === "event") {
    return (
      <TimelineItem meta={entry.meta} state={entry.state} last={last} event={entry.event}>
        <div className={styles.eventRow} data-state={entry.state}>
          <span className={styles.eventIcon}>
            <TimelineStateIcon state={entry.state} />
          </span>
          <div>
            <strong>{entry.title}</strong>
            <span>{entry.detail}</span>
          </div>
          {entry.tag ? <code>{entry.tag}</code> : null}
        </div>
      </TimelineItem>
    );
  }

  if (entry.kind === "tool") {
    const detailId = `tool-details-${sessionId}-${entry.id}`;
    return (
      <TimelineItem meta={entry.meta} state={entry.state} last={last} event={entry.event}>
        <article className={styles.toolGroup}>
          <button
            type="button"
            className={styles.toolSummary}
            aria-controls={detailId}
            aria-expanded={toolOpen}
            onClick={onToolToggle}
          >
            <span className={styles.eventIcon}><CodeIcon /></span>
            <span>
              <strong>{entry.title}</strong>
              <small>{entry.subtitle}</small>
            </span>
            <span className={styles.toolOutcome} data-state={entry.state}>{entry.outcome}</span>
            <ChevronDownIcon data-open={toolOpen} />
          </button>
          {toolOpen ? (
            <div id={detailId} className={styles.toolDetails}>
              <div className={styles.commandLine}>
                <code>{entry.command}</code>
                <button
                  type="button"
                  aria-label={`Copy ${entry.title} command`}
                  title="Copy command"
                >
                  <CopyIcon />
                </button>
              </div>
              <dl>
                {entry.facts.map((fact) => (
                  <div key={fact.label}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>
                ))}
              </dl>
            </div>
          ) : null}
        </article>
      </TimelineItem>
    );
  }

  return (
    <TimelineItem
      meta={entry.meta}
      state={approval === "pending" ? "attention" : "complete"}
      last={last}
      event={entry.event}
    >
      <article className={styles.approvalBlock} data-state={approval}>
        <div className={styles.approvalHeading}>
          <span className={styles.eventIcon}>
            {approval === "approved" ? (
              <CheckIcon />
            ) : approval === "rejected" ? (
              <Cross2Icon />
            ) : (
              <LockClosedIcon />
            )}
          </span>
          <div>
            <strong>
              {approval === "pending"
                ? "Approval required"
                : approval === "approved"
                  ? "Approved for this run"
                  : "Mutation rejected"}
            </strong>
            <span>{entry.description}</span>
          </div>
          <span className={styles.riskTag}>{entry.risk}</span>
        </div>
        <div className={styles.approvalFacts}>
          {entry.facts.map((fact) => (
            <span key={fact.label}><strong>{fact.label}</strong> {fact.value}</span>
          ))}
        </div>
        {approval === "pending" ? (
          <div className={styles.approvalActions}>
            <button
              type="button"
              className={styles.primaryButton}
              onClick={() => onApprovalChange("approved")}
            >
              <CheckIcon /> Approve once
            </button>
            <button
              type="button"
              className={styles.secondaryButton}
              onClick={() => onApprovalChange("rejected")}
            >
              <Cross2Icon /> Reject
            </button>
          </div>
        ) : (
          <button
            type="button"
            className={styles.textButton}
            onClick={() => onApprovalChange("pending")}
          >
            Reset mock state
          </button>
        )}
      </article>
    </TimelineItem>
  );
}

function TimelineStateIcon({ state }: { state: "complete" | "attention" | "running" }) {
  if (state === "complete") {
    return <CheckIcon />;
  }
  if (state === "attention") {
    return <ExclamationTriangleIcon />;
  }
  return <ReloadIcon />;
}

function TimelineItem({
  meta,
  event,
  state,
  last,
  children,
}: {
  meta: string;
  event?: string;
  state: "message" | "complete" | "attention" | "running";
  last: boolean;
  children: ReactNode;
}) {
  return (
    <div className={styles.timelineItem} data-state={state} data-last={last}>
      <div className={styles.timelineMeta}>
        <time>{meta}</time>
        {event ? <code>{event}</code> : null}
      </div>
      <div className={styles.traceTrack} aria-hidden="true"><span /></div>
      <div className={styles.timelineContent}>{children}</div>
    </div>
  );
}

function MessageByline({ label, detail }: { label: string; detail: string }) {
  return (
    <div className={styles.messageByline}>
      <strong>{label}</strong>
      <span>{detail}</span>
    </div>
  );
}

function EvidenceInspector({
  inactive,
  open,
  onClose,
  session,
}: {
  inactive: boolean;
  open: boolean;
  onClose: () => void;
  session: ProductUiV2MockSession;
}) {
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) {
      closeButtonRef.current?.focus();
    }
  }, [open]);

  return (
    <aside
      className={styles.inspector}
      data-open={open}
      aria-label="Run evidence"
      aria-modal={open ? true : undefined}
      inert={inactive ? true : undefined}
      onKeyDown={open ? trapDialogFocus : undefined}
      role={open ? "dialog" : undefined}
    >
      <div className={styles.inspectorHeading}>
        <div>
          <span className={styles.sectionLabel}>Run evidence</span>
          <strong>{session.inspector.heading}</strong>
        </div>
        <button ref={closeButtonRef} type="button" className={`${styles.iconButton} ${styles.inspectorClose}`} onClick={onClose} aria-label="Close run evidence" title="Close run evidence">
          <Cross2Icon />
        </button>
      </div>

      <div className={styles.inspectorScroll}>
        <section className={styles.inspectorSection}>
          <h2>Continuity</h2>
          <div className={styles.continuityChain}>
            <div data-state="exact"><span><CheckIcon /></span><p><strong>Workspace</strong><small>rove, exact root</small></p></div>
            <div data-state="exact"><span><CheckIcon /></span><p><strong>Session</strong><small>server-owned binding</small></p></div>
            <div data-state={session.status === "running" ? "active" : session.status}>
              <span>
                {session.status === "complete" ? <CheckIcon /> : null}
                {session.status === "attention" ? <ExclamationTriangleIcon /> : null}
              </span>
              <p><strong>Run</strong><small>{session.inspector.runDetail}</small></p>
            </div>
          </div>
        </section>

        <section className={styles.inspectorSection}>
          <div className={styles.sectionHeadingRow}>
            <h2>Plan</h2>
            <span data-state={session.status}>{session.inspector.planStatus}</span>
          </div>
          <ol className={styles.planList}>
            {session.inspector.planItems.map((item) => (
              <li key={item.id} data-state={item.state === "pending" ? undefined : item.state}>
                {item.state === "complete" ? <CheckIcon /> : <span />}
                <span>{item.label}</span>
              </li>
            ))}
          </ol>
        </section>

        <section className={styles.inspectorSection}>
          <h2>Execution facts</h2>
          <dl className={styles.factList}>
            {session.inspector.facts.map((fact) => (
              <div key={fact.label}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>
            ))}
          </dl>
        </section>

        <section className={styles.inspectorSection}>
          <h2>Canonical events</h2>
          <ul className={styles.eventList}>
            {session.inspector.events.map((event) => (
              <li key={event.id}><span data-state={event.state} /><code>{event.label}</code></li>
            ))}
          </ul>
        </section>
      </div>
    </aside>
  );
}

function SettingsSurface({
  theme,
  onThemeChange,
}: {
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
}) {
  const [section, setSection] = useState<SettingsSection>("providers");

  return (
    <main id="v2-main" className={styles.settingsSurface}>
      <aside className={styles.settingsNav} aria-label="Settings sections">
        <div className={styles.settingsNavHeading}>
          <span className={styles.sectionLabel}>Product settings</span>
          <strong>Shared by Web and Desktop</strong>
        </div>
        <nav>
          {settingsSections.map((item) => (
            <button
              type="button"
              key={item.id}
              data-active={section === item.id}
              onClick={() => setSection(item.id)}
            >
              {item.icon}
              <span>{item.label}</span>
              <ChevronRightIcon />
            </button>
          ))}
        </nav>
        <div className={styles.settingsTheme}>
          <span>Appearance</span>
          <div className={styles.modeSwitch} aria-label="Color theme">
            <button type="button" data-active={theme === "light"} onClick={() => onThemeChange("light")}><SunIcon /> Light</button>
            <button type="button" data-active={theme === "dark"} onClick={() => onThemeChange("dark")}><MoonIcon /> Dark</button>
          </div>
        </div>
      </aside>

      <div className={styles.settingsContent}>
        <SettingsHeader section={section} />
        {section === "providers" ? <ProviderSettings /> : null}
        {section === "approvals" ? <ApprovalSettings /> : null}
        {section === "memory" ? <MemorySettings /> : null}
        {section === "sessions" ? <SessionSettings /> : null}
        {section === "desktop" ? <DesktopSettings /> : null}
      </div>
    </main>
  );
}

function SettingsHeader({ section }: { section: SettingsSection }) {
  const copy: Record<SettingsSection, { title: string; description: string }> = {
    providers: {
      title: "Providers & models",
      description: "Choose server-owned profiles. Raw provider keys never enter browser state.",
    },
    approvals: {
      title: "Tools & approvals",
      description: "Set the default boundary for consequential tool calls.",
    },
    memory: {
      title: "Workspace memory",
      description: "Inspect memory only for the currently selected workspace root.",
    },
    sessions: {
      title: "Session management",
      description: "Export complete evidence or remove a session and its product binding.",
    },
    desktop: {
      title: "Desktop host",
      description: "Preview host-only capabilities without forking the shared product UI.",
    },
  };

  return (
    <header className={styles.settingsHeader}>
      <div>
        <span className={styles.sectionLabel}>Settings</span>
        <h1>{copy[section].title}</h1>
        <p>{copy[section].description}</p>
      </div>
      <span className={styles.mockNotice} title={PRODUCT_UI_V2_PREVIEW_BOUNDARY}>
        <InfoCircledIcon /> Inert mock, no live actions
      </span>
    </header>
  );
}

function ProviderSettings() {
  const [reasoning, setReasoning] = useState("balanced");
  const [tested, setTested] = useState(false);

  return (
    <div className={styles.settingsStack}>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.settingsSectionHeading}>
          <div><h2>Active profile</h2><p>Used for new runs in this workspace unless the session overrides it.</p></div>
          <button type="button" className={styles.secondaryButton}><PlusIcon /> Add profile</button>
        </div>
        <div className={styles.providerRow}>
          <span className={styles.providerMark}>LC</span>
          <div className={styles.providerIdentity}>
            <strong>Local configured provider</strong>
            <span>Server profile, credential via environment variable</span>
          </div>
          <span className={styles.activeTag}><CheckIcon /> Active</span>
          <button type="button" className={styles.iconButton} aria-label="Profile actions" title="Profile actions"><DotsHorizontalIcon /></button>
        </div>
      </section>

      <section className={styles.settingsSectionBlock}>
        <div className={styles.settingsSectionHeading}>
          <div><h2>Run defaults</h2><p>Compact controls remain available in the session composer.</p></div>
        </div>
        <div className={styles.settingsFields}>
          <label>
            <span>Model</span>
            <button type="button" className={styles.selectButton}>Configured default <ChevronDownIcon /></button>
          </label>
          <fieldset>
            <legend>Reasoning</legend>
            <div className={styles.choiceGrid}>
              {[
                ["fast", "Fast", "Shorter iteration for routine edits."],
                ["balanced", "Balanced", "Default for product work."],
                ["deep", "Deep", "More time for ambiguous tasks."],
              ].map(([id, label, detail]) => (
                <button key={id} type="button" data-active={reasoning === id} onClick={() => setReasoning(id)}>
                  <span>{reasoning === id ? <CheckCircledIcon /> : <span className={styles.emptyRadio} />}</span>
                  <strong>{label}</strong>
                  <small>{detail}</small>
                </button>
              ))}
            </div>
          </fieldset>
        </div>
      </section>

      <section className={styles.settingsSectionBlock}>
        <div className={styles.connectionTest} data-tested={tested}>
          <span className={styles.eventIcon}>{tested ? <CheckIcon /> : <ReloadIcon />}</span>
          <div><strong>{tested ? "Connection verified in mock state" : "Connection check"}</strong><span>No secret values are returned to this surface.</span></div>
          <button type="button" className={styles.secondaryButton} onClick={() => setTested(true)}>{tested ? "Test again" : "Test profile"}</button>
        </div>
      </section>
    </div>
  );
}

function ApprovalSettings() {
  const [policy, setPolicy] = useState("ask-mutations");
  return (
    <div className={styles.settingsStack}>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.settingsSectionHeading}><div><h2>Default approval policy</h2><p>The runtime remains authoritative even when this preference changes.</p></div></div>
        <div className={styles.choiceGrid}>
          {[
            ["ask-mutations", "Ask for mutations", "Read-only tools proceed. Writes and external effects pause."],
            ["ask-all", "Ask for every tool", "Every tool call requires an explicit decision."],
            ["runtime", "Runtime policy", "Use the server policy without a product override."],
          ].map(([id, label, detail]) => (
            <button key={id} type="button" data-active={policy === id} onClick={() => setPolicy(id)}>
              <span>{policy === id ? <CheckCircledIcon /> : <span className={styles.emptyRadio} />}</span>
              <strong>{label}</strong>
              <small>{detail}</small>
            </button>
          ))}
        </div>
      </section>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.settingsSectionHeading}><div><h2>Boundary summary</h2><p>Remote annotations and generated text cannot grant permission.</p></div></div>
        <dl className={styles.managementList}>
          <div><dt><LockClosedIcon /> Workspace paths</dt><dd>Resolved root only</dd><span>Enforced</span></div>
          <div><dt><MixerHorizontalIcon /> Tool registry</dt><dd>Shared safety path</dd><span>Enforced</span></div>
          <div><dt><ActivityLogIcon /> Side effects</dt><dd>Conservative on uncertain cancellation</dd><span>Fail closed</span></div>
        </dl>
      </section>
    </div>
  );
}

function MemorySettings() {
  return (
    <div className={styles.settingsStack}>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.scopeBanner}>
          <span className={styles.eventIcon}><LockClosedIcon /></span>
          <div><strong>Scope: rove workspace</strong><span>{PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT}</span></div>
          <span className={styles.activeTag}>Exact root</span>
        </div>
      </section>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.settingsSectionHeading}><div><h2>Layered memory</h2><p>Memory is evidence and context. It is not an automatic permission source.</p></div></div>
        <dl className={styles.managementList}>
          <div><dt><FileIcon /> Workspace instructions</dt><dd>Repository files</dd><span>Read only</span></div>
          <div><dt><ArchiveIcon /> Durable memory</dt><dd>Selected workspace</dd><span>Available</span></div>
          <div><dt><CodeIcon /> Tool output</dt><dd>Current run context</dd><span>Bounded</span></div>
        </dl>
      </section>
    </div>
  );
}

function SessionSettings() {
  return (
    <div className={styles.settingsStack}>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.settingsSectionHeading}><div><h2>Current session</h2><p>Export includes transcript, canonical event references, and continuity identity.</p></div></div>
        <div className={styles.sessionManagementRow}>
          <div><strong>C4 web control surface</strong><span>rove / active run</span></div>
          <button type="button" className={styles.secondaryButton}><ArchiveIcon /> Export evidence</button>
          <button type="button" className={styles.dangerButton}><TrashIcon /> Delete session</button>
        </div>
      </section>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.infoRow}><InfoCircledIcon /><span>Deletion requires confirmation and never removes the workspace root.</span></div>
      </section>
    </div>
  );
}

function DesktopSettings() {
  return (
    <div className={styles.settingsStack}>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.settingsSectionHeading}><div><h2>Host capability contract</h2><p>The product pages remain shared. Only host integrations vary.</p></div></div>
        <dl className={styles.managementList}>
          <div><dt><DesktopIcon /> Native folder picker</dt><dd>Desktop adapter</dd><span>Proposed</span></div>
          <div><dt><LockClosedIcon /> Secret storage</dt><dd>Host-owned secure store</dd><span>Proposed</span></div>
          <div><dt><ActivityLogIcon /> Runtime lifecycle</dt><dd>Start, observe, stop</dd><span>Proposed</span></div>
        </dl>
      </section>
      <section className={styles.settingsSectionBlock}>
        <div className={styles.warningRow}><ExclamationTriangleIcon /><span>This preview does not imply that the Tauri host is implemented.</span></div>
      </section>
    </div>
  );
}
