"use client";

import {
  CheckCircledIcon,
  CrossCircledIcon,
  DotFilledIcon,
  DownloadIcon,
  FileTextIcon,
  PlayIcon,
  ReloadIcon,
  StopwatchIcon,
} from "@radix-ui/react-icons";
import { useEffect, useState, type ReactNode } from "react";

import {
  benchEvidenceUrl,
  fetchBenchRun,
  fetchBenchTask,
  listBenchRuns,
  listBenchSuites,
  startBenchRun,
} from "../lib/rove-client";
import type {
  BenchRunDetail,
  BenchRunSummary,
  BenchSuiteInfo,
  BenchTaskResult,
} from "../lib/rove-types";

type RunStatus = "running" | "passed" | "failed" | "idle";

export function BenchmarkPanel() {
  const [suites, setSuites] = useState<BenchSuiteInfo[]>([]);
  const [selectedSuite, setSelectedSuite] = useState<string>("dataprep");
  const [selectedProfile, setSelectedProfile] = useState<string>("default");
  const [runs, setRuns] = useState<BenchRunSummary[]>([]);
  const [activeRun, setActiveRun] = useState<BenchRunDetail | null>(null);
  const [activeTask, setActiveTask] = useState<BenchTaskResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pollTimer, setPollTimer] = useState<ReturnType<typeof setTimeout> | null>(null);

  // Load suites on mount
  useEffect(() => {
    void loadSuites();
    void loadRuns();
    return () => {
      if (pollTimer) clearTimeout(pollTimer);
    };
  }, []);

  async function loadSuites() {
    try {
      const result = await listBenchSuites();
      setSuites(result.suites);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function loadRuns() {
    setLoading(true);
    setError(null);
    try {
      const result = await listBenchRuns();
      setRuns(result.runs);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  async function handleStart() {
    setStarting(true);
    setError(null);
    setActiveTask(null);
    try {
      const resp = await startBenchRun({ suite: selectedSuite, profile: selectedProfile });
      await loadRuns();
      // Start polling for completion
      pollForCompletion(resp.bench_run_id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  }

  function pollForCompletion(benchRunId: string) {
    const tick = async () => {
      try {
        const detail = await fetchBenchRun(benchRunId);
        setActiveRun(detail);
        if (detail.status === "running") {
          setPollTimer(setTimeout(tick, 1000));
        } else {
          await loadRuns();
        }
      } catch {
        // keep polling
        setPollTimer(setTimeout(tick, 2000));
      }
    };
    setPollTimer(setTimeout(tick, 500));
  }

  async function handleSelectRun(run: BenchRunSummary) {
    setActiveTask(null);
    setError(null);
    try {
      const detail = await fetchBenchRun(run.bench_run_id);
      setActiveRun(detail);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function handleSelectTask(task: BenchTaskResult) {
    setError(null);
    try {
      if (activeRun) {
        const detail = await fetchBenchTask(activeRun.bench_run_id, task.name);
        setActiveTask(detail);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const currentSuite = suites.find((s) => s.name === selectedSuite);
  const passRate = activeRun && activeRun.total_tasks > 0
    ? Math.round((activeRun.passed_tasks / activeRun.total_tasks) * 100)
    : 0;

  return (
    <div className="bench-panel">
      <header className="bench-header">
        <div className="bench-header__title">
          <h2>Benchmark Runner</h2>
          <p>Deterministic multi-phase task evaluation with evidence packages</p>
        </div>
        <div className="bench-header__actions">
          <button type="button" className="secondary" onClick={loadRuns} disabled={loading}>
            <ReloadIcon width={14} height={14} />
            Refresh
          </button>
        </div>
      </header>

      {error && <div className="bench-error">{error}</div>}

      <section className="bench-config" aria-label="Benchmark configuration">
        <div className="bench-config__fields">
          <label className="field">
            <span>Suite</span>
            <select
              value={selectedSuite}
              onChange={(e) => {
                setSelectedSuite(e.target.value);
                setActiveRun(null);
                setActiveTask(null);
              }}
            >
              {suites.map((s) => (
                <option key={s.name} value={s.name}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Profile</span>
            <select
              value={selectedProfile}
              onChange={(e) => {
                setSelectedProfile(e.target.value);
                setActiveRun(null);
                setActiveTask(null);
              }}
            >
              {currentSuite?.profiles.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              )) ?? <option value="default">default</option>}
            </select>
          </label>
          <button type="button" className="bench-start" onClick={handleStart} disabled={starting}>
            <PlayIcon width={14} height={14} />
            {starting ? "Starting…" : "Run Benchmark"}
          </button>
        </div>
        {currentSuite && (
          <p className="bench-config__desc">{currentSuite.description}</p>
        )}
      </section>

      <section className="bench-body">
        <div className="bench-sidebar">
          <h3>Run History</h3>
          {loading && runs.length === 0 ? (
            <div className="bench-empty">Loading…</div>
          ) : runs.length === 0 ? (
            <div className="bench-empty">No benchmark runs yet</div>
          ) : (
            <div className="bench-runs-list">
              {runs.map((run) => (
                <button
                  key={run.bench_run_id}
                  type="button"
                  className={`bench-run-card ${
                    activeRun?.bench_run_id === run.bench_run_id ? "bench-run-card--active" : ""
                  }`}
                  onClick={() => void handleSelectRun(run)}
                >
                  <div className="bench-run-card__head">
                    <StatusBadge status={run.status as RunStatus} />
                    <strong>{run.suite}</strong>
                    <span className="bench-run-card__profile">{run.profile}</span>
                  </div>
                  <div className="bench-run-card__stats">
                    <span>
                      {run.passed_tasks}/{run.total_tasks} passed
                    </span>
                    <span>{shortId(run.bench_run_id)}</span>
                  </div>
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="bench-content">
          {activeRun ? (
            <>
              <div className="bench-summary">
                <div className="bench-summary__head">
                  <h3>
                    {activeRun.suite} / {activeRun.profile}
                  </h3>
                  <StatusBadge status={activeRun.status as RunStatus} />
                </div>
                <div className="bench-summary__grid">
                  <SummaryMetric label="Total tasks" value={String(activeRun.total_tasks)} />
                  <SummaryMetric label="Passed" value={String(activeRun.passed_tasks)} tone="ok" />
                  <SummaryMetric label="Failed" value={String(activeRun.failed_tasks)} tone={activeRun.failed_tasks > 0 ? "error" : undefined} />
                  <SummaryMetric label="Pass rate" value={`${passRate}%`} />
                </div>
                {activeRun.finished_at && (
                  <div className="bench-summary__time">
                    <StopwatchIcon width={13} height={13} />
                    <span>Completed {new Date(activeRun.finished_at).toLocaleString()}</span>
                  </div>
                )}
                {activeRun.evidence_root && (
                  <div className="bench-summary__evidence">
                    <FileTextIcon width={13} height={13} />
                    <span>Evidence: {activeRun.evidence_root}</span>
                    {activeRun.status !== "running" && (
                      <a
                        href={benchEvidenceUrl(activeRun.bench_run_id, "summary.md")}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="bench-link"
                      >
                        <DownloadIcon width={12} height={12} />
                        summary.md
                      </a>
                    )}
                  </div>
                )}
              </div>

              {activeRun.tasks.length > 0 && (
                <div className="bench-tasks">
                  <h4>Tasks ({activeRun.tasks.length})</h4>
                  <div className="bench-tasks-grid">
                    {activeRun.tasks.map((task) => (
                      <button
                        key={task.name}
                        type="button"
                        className={`bench-task-card bench-task-card--${task.outcome} ${
                          activeTask?.name === task.name ? "bench-task-card--active" : ""
                        }`}
                        onClick={() => void handleSelectTask(task)}
                      >
                        <div className="bench-task-card__head">
                          {task.outcome === "passed" ? (
                            <CheckCircledIcon width={14} height={14} />
                          ) : (
                            <CrossCircledIcon width={14} height={14} />
                          )}
                          <strong>{task.name}</strong>
                        </div>
                        <div className="bench-task-card__meta">
                          <span>{task.steps} steps</span>
                          <span>{task.tool_calls} tools</span>
                          <span>{task.check_results.filter((c) => c.passed).length}/{task.check_results.length} checks</span>
                        </div>
                        {task.failures.length > 0 && (
                          <div className="bench-task-card__failures">
                            {task.failures.slice(0, 1).map((f, i) => (
                              <span key={i} className="bench-failure-text">{f.slice(0, 80)}…</span>
                            ))}
                          </div>
                        )}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {activeTask && (
                <div className="bench-task-detail">
                  <h4>Task detail: {activeTask.name}</h4>
                  <div className="bench-task-detail__grid">
                    <SummaryMetric label="Outcome" value={activeTask.outcome} tone={activeTask.outcome === "passed" ? "ok" : "error"} />
                    <SummaryMetric label="Steps" value={String(activeTask.steps)} />
                    <SummaryMetric label="Tool calls" value={String(activeTask.tool_calls)} />
                    <SummaryMetric label="Tool failures" value={String(activeTask.tool_failures)} tone={activeTask.tool_failures > 0 ? "warn" : undefined} />
                    <SummaryMetric label="Termination" value={activeTask.termination_reason} />
                  </div>

                  <div className="bench-artifacts">
                    <h5>Artifacts</h5>
                    <div className="bench-artifacts-list">
                      <ArtifactLink benchRunId={activeRun.bench_run_id} path={relPath(activeTask.artifacts.trace_jsonl, activeRun.evidence_root)} label="trace.jsonl" />
                      <ArtifactLink benchRunId={activeRun.bench_run_id} path={relPath(activeTask.artifacts.task_state_json, activeRun.evidence_root)} label="task_state.json" />
                      <ArtifactLink benchRunId={activeRun.bench_run_id} path={relPath(activeTask.artifacts.report_json, activeRun.evidence_root)} label="report.json" />
                    </div>
                  </div>

                  <div className="bench-checks">
                    <h5>Checks ({activeTask.check_results.filter((c) => c.passed).length}/{activeTask.check_results.length} passed)</h5>
                    <div className="bench-checks-list">
                      {activeTask.check_results.map((check, i) => (
                        <div key={i} className={`bench-check bench-check--${check.passed ? "pass" : "fail"}`}>
                          <div className="bench-check__head">
                            {check.passed ? <CheckCircledIcon width={13} height={13} /> : <CrossCircledIcon width={13} height={13} />}
                            <strong>{check.description}</strong>
                            <span className="bench-check__kind">{check.kind}</span>
                          </div>
                          <p>{check.detail}</p>
                        </div>
                      ))}
                    </div>
                  </div>

                  {activeTask.failures.length > 0 && (
                    <div className="bench-failures">
                      <h5>Failures</h5>
                      <ul>
                        {activeTask.failures.map((f, i) => (
                          <li key={i}>{f}</li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              )}
            </>
          ) : (
            <div className="bench-welcome">
              <div className="bench-welcome__icon">
                <PlayIcon width={32} height={32} />
              </div>
              <h3>Run a benchmark</h3>
              <p>Select a suite and profile above, then click "Run Benchmark" to start a deterministic evaluation using the FakeProvider.</p>
              <div className="bench-welcome__features">
                <Feature icon={<FileTextIcon width={16} height={16} />} title="Multi-phase tasks" desc="Read inputs, write intermediates, recover from failures, aggregate results" />
                <Feature icon={<CheckCircledIcon width={16} height={16} />} title="4 check types" desc="File existence, content matching, trace events, command oracle" />
                <Feature icon={<StopwatchIcon width={16} height={16} />} title="Evidence packages" desc="Every run produces machine-readable metrics and human reviewable artifacts" />
              </div>
            </div>
          )}
        </div>
      </section>
    </div>
  );
}

function StatusBadge({ status }: { status: RunStatus }) {
  return (
    <span className={`status-chip status-chip--${status}`}>
      <DotFilledIcon />
      {status}
    </span>
  );
}

function SummaryMetric({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "ok" | "error" | "warn";
}) {
  return (
    <div className={`bench-metric ${tone ? `bench-metric--${tone}` : ""}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ArtifactLink({
  benchRunId,
  path,
  label,
}: {
  benchRunId: string;
  path: string;
  label: string;
}) {
  if (!path) return null;
  return (
    <a
      href={benchEvidenceUrl(benchRunId, path)}
      target="_blank"
      rel="noopener noreferrer"
      className="bench-artifact-link"
    >
      <FileTextIcon width={12} height={12} />
      {label}
    </a>
  );
}

function Feature({ icon, title, desc }: { icon: ReactNode; title: string; desc: string }) {
  return (
    <div className="bench-feature">
      <span className="bench-feature__icon">{icon}</span>
      <div>
        <strong>{title}</strong>
        <p>{desc}</p>
      </div>
    </div>
  );
}

function relPath(absPath: string, evidenceRoot: string | null): string {
  if (!evidenceRoot) return absPath;
  const root = evidenceRoot.replace(/\/$/, "");
  if (absPath.startsWith(root + "/")) {
    return absPath.slice(root.length + 1);
  }
  return absPath;
}

function shortId(id: string): string {
  return id.slice(0, 16);
}
