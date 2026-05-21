const form = document.querySelector("#job-form");
const messageInput = document.querySelector("#message");
const modelInput = document.querySelector("#model");
const maxStepsInput = document.querySelector("#max-steps");
const submitButton = document.querySelector("#submit");
const cancelButton = document.querySelector("#cancel");
const messages = document.querySelector("#messages");
const runMeta = document.querySelector("#run-meta");
const planList = document.querySelector("#plan");
const toolList = document.querySelector("#tools");
const trace = document.querySelector("#trace");

let activeJobId = null;
let events = null;
let assistantMessage = null;
const planSteps = new Map();
const tools = new Map();

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const message = messageInput.value.trim();
  if (!message) return;

  resetRun();
  appendMessage("user", message);
  setBusy(true);

  const payload = {
    message,
    model: modelInput.value.trim() || undefined,
    max_steps: Number(maxStepsInput.value) || undefined,
  };

  try {
    const response = await fetch("/jobs", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!response.ok) throw new Error(await response.text());
    const job = await response.json();
    activeJobId = job.job_id;
    runMeta.textContent = `Job ${job.job_id} · Run ${job.run_id}`;
    setBusy(true);
    openEvents(job.job_id);
  } catch (error) {
    appendTrace("error", `create job failed: ${error.message}`);
    setBusy(false);
  }
});

cancelButton.addEventListener("click", async () => {
  if (!activeJobId) return;
  await fetch(`/jobs/${activeJobId}/cancel`, { method: "POST" });
  appendTrace("cancelled", "cancel requested");
  setBusy(false);
});

function openEvents(jobId) {
  if (events) events.close();
  events = new EventSource(`/jobs/${jobId}/events`);

  [
    "run_started",
    "llm_chunk",
    "llm_message",
    "tool_call_started",
    "tool_call_completed",
    "tool_call_failed",
    "plan_created",
    "plan_step_started",
    "plan_step_completed",
    "run_completed",
  ].forEach((name) => events.addEventListener(name, (event) => handleEvent(name, event)));

  events.onerror = () => appendTrace("stream", "waiting for events");
}

function handleEvent(name, event) {
  const data = JSON.parse(event.data);
  appendTrace(name, summarize(data));

  if (name === "llm_chunk") {
    appendAssistantDelta(data.delta);
  } else if (name === "llm_message") {
    setAssistantText(data.full);
  } else if (name === "plan_created") {
    renderPlan(data.plan.steps, data.plan.current_step);
  } else if (name === "plan_step_started") {
    updatePlanStep(data.step, "running");
  } else if (name === "plan_step_completed") {
    updatePlanStep(data.step, "done");
  } else if (name === "tool_call_started") {
    renderTool(data.call_id, data.name, data.args, "running");
  } else if (name === "tool_call_completed") {
    renderTool(data.call_id, "tool", data.result.output, "done");
  } else if (name === "tool_call_failed") {
    renderTool(data.call_id, "tool", data.error, "error");
  } else if (name === "run_completed") {
    if (data.output) setAssistantText(data.output);
    setBusy(false);
    if (events) events.close();
  }
}

function resetRun() {
  if (events) events.close();
  activeJobId = null;
  assistantMessage = null;
  planSteps.clear();
  tools.clear();
  messages.textContent = "";
  planList.textContent = "";
  toolList.textContent = "";
  trace.textContent = "";
  runMeta.textContent = "Starting run";
}

function setBusy(busy) {
  submitButton.disabled = busy;
  cancelButton.disabled = !busy || !activeJobId;
}

function appendMessage(role, text) {
  const item = document.createElement("article");
  item.className = `message ${role}`;
  item.textContent = text;
  messages.append(item);
  messages.scrollTop = messages.scrollHeight;
  return item;
}

function appendAssistantDelta(delta) {
  if (!assistantMessage) assistantMessage = appendMessage("assistant", "");
  assistantMessage.textContent += delta;
  messages.scrollTop = messages.scrollHeight;
}

function setAssistantText(text) {
  if (!assistantMessage) assistantMessage = appendMessage("assistant", "");
  assistantMessage.textContent = text;
  messages.scrollTop = messages.scrollHeight;
}

function renderPlan(steps, current) {
  planList.textContent = "";
  steps.forEach((step, index) => {
    planSteps.set(step.id, step);
    const item = document.createElement("li");
    item.dataset.stepId = step.id;
    item.className = step.done ? "done" : index === current ? "running" : "";
    item.textContent = step.title;
    planList.append(item);
  });
}

function updatePlanStep(step, state) {
  planSteps.set(step.id, step);
  let item = planList.querySelector(`[data-step-id="${CSS.escape(step.id)}"]`);
  if (!item) {
    item = document.createElement("li");
    item.dataset.stepId = step.id;
    item.textContent = step.title;
    planList.append(item);
  }
  item.className = state;
}

function renderTool(id, name, details, state) {
  const key = String(id);
  const current = tools.get(key) || { name };
  current.name = current.name === "tool" ? name : current.name;
  current.details = details;
  current.state = state;
  tools.set(key, current);

  let item = toolList.querySelector(`[data-tool-id="${CSS.escape(key)}"]`);
  if (!item) {
    item = document.createElement("article");
    item.dataset.toolId = key;
    item.innerHTML = "<h3></h3><pre></pre>";
    toolList.append(item);
  }
  item.className = `tool ${state}`;
  item.querySelector("h3").textContent = current.name || name;
  item.querySelector("pre").textContent = format(details);
}

function appendTrace(name, text) {
  const item = document.createElement("div");
  item.className = "trace-row";
  item.innerHTML = `<span>${escapeHtml(name)}</span><p>${escapeHtml(text)}</p>`;
  trace.prepend(item);
}

function summarize(data) {
  if (data.type === "llm_chunk") return data.delta;
  if (data.type === "tool_call_started") return `${data.name} ${format(data.args)}`;
  if (data.type === "run_completed") return data.reason;
  return format(data);
}

function format(value) {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (char) => {
    return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[char];
  });
}
