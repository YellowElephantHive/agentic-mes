"use strict";

const scope = Object.freeze({
  tenantId: "b242333d-c1c1-581c-a71f-ab8b8a92dc2a",
  factoryId: "8642cb8c-d730-5ce2-a6b0-17f17c2f2444",
  lineId: "90c31f01-e9c2-56d9-b365-9d0047c7939c",
  startAt: "2026-01-30T00:00:00Z",
  endAt: "2026-01-31T00:00:00Z",
  baselineStartAt: "2026-01-29T00:00:00Z",
});

const elements = {
  systemLabel: document.querySelector("#system-label"),
  systemPulse: document.querySelector(".pulse"),
  refreshButton: document.querySelector("#refresh-button"),
  runButton: document.querySelector("#run-button"),
  runStatus: document.querySelector("#run-status"),
  question: document.querySelector("#question"),
  goodOutput: document.querySelector("#good-output"),
  outputChange: document.querySelector("#output-change"),
  downtime: document.querySelector("#downtime"),
  downtimeEvents: document.querySelector("#downtime-events"),
  defectRate: document.querySelector("#defect-rate"),
  inspectionCount: document.querySelector("#inspection-count"),
  evidenceHealth: document.querySelector("#evidence-health"),
  signalList: document.querySelector("#signal-list"),
  answerCard: document.querySelector("#answer-card"),
  agentAnswer: document.querySelector("#agent-answer"),
  answerMeta: document.querySelector("#answer-meta"),
  toolTable: document.querySelector("#tool-table"),
  toolCount: document.querySelector("#tool-count"),
  toast: document.querySelector("#toast"),
};

const authHeaders = Object.freeze({
  "x-dev-tenant-id": scope.tenantId,
  "x-dev-factory-id": scope.factoryId,
});

async function requestJson(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { ...authHeaders, ...(options.headers || {}) },
  });
  const contentType = response.headers.get("content-type") || "";
  const body = contentType.includes("application/json") ? await response.json() : await response.text();
  if (!response.ok) {
    const message = typeof body === "object" ? body?.error?.message : body;
    throw new Error(message || `Request failed with HTTP ${response.status}`);
  }
  return body;
}

function query(path, startAt = scope.startAt, endAt = scope.endAt) {
  const params = new URLSearchParams({ line_id: scope.lineId, start_at: startAt, end_at: endAt });
  return `${path}?${params}`;
}

function showToast(message, error = false) {
  elements.toast.textContent = message;
  elements.toast.classList.toggle("error", error);
  elements.toast.classList.add("show");
  window.clearTimeout(showToast.timeout);
  showToast.timeout = window.setTimeout(() => elements.toast.classList.remove("show"), 3200);
}

function setRunStatus(label, state) {
  elements.runStatus.textContent = label;
  elements.runStatus.className = `status-pill ${state}`;
}

function makeSignal(label, detail, tone = "amber") {
  const item = document.createElement("li");
  const dot = document.createElement("span");
  dot.className = "signal-dot";
  if (tone === "signal") dot.style.background = "var(--color-signal)";
  if (tone === "green") dot.style.background = "var(--color-green)";
  const name = document.createElement("span");
  name.textContent = label;
  const meta = document.createElement("small");
  meta.textContent = detail;
  item.append(dot, name, meta);
  return item;
}

async function loadDashboard() {
  elements.refreshButton.disabled = true;
  try {
    const [ready, baseline, production, downtime, quality] = await Promise.all([
      requestJson("/readyz", { headers: {} }),
      requestJson(query("/api/v1/production/summary", scope.baselineStartAt, scope.startAt)),
      requestJson(query("/api/v1/production/summary")),
      requestJson(query("/api/v1/equipment/downtime")),
      requestJson(query("/api/v1/quality/summary")),
    ]);
    const currentGood = production.data.good_qty;
    const previousGood = baseline.data.good_qty;
    const change = previousGood ? ((currentGood - previousGood) / previousGood) * 100 : null;
    elements.goodOutput.textContent = currentGood.toLocaleString();
    elements.outputChange.textContent = change === null ? "No baseline" : `${change.toFixed(0)}%`;
    elements.downtime.textContent = downtime.data.line_downtime_minutes.toLocaleString();
    elements.downtimeEvents.textContent = `${downtime.data.events.length} events`;
    elements.defectRate.textContent = (quality.data.qc_sample_defect_rate * 100).toFixed(1);
    elements.inspectionCount.textContent = `${quality.data.sample_size} samples`;
    elements.evidenceHealth.textContent = "Sources ready";
    elements.systemLabel.textContent = `${ready.provider.type} · ready`;
    elements.systemPulse.classList.add("ready");

    elements.signalList.replaceChildren();
    for (const event of downtime.data.events) {
      const durationMinutes = Math.round((Date.parse(event.ended_at) - Date.parse(event.started_at)) / 60_000);
      elements.signalList.append(makeSignal(event.reason_code, `${durationMinutes} min`, "signal"));
    }
    elements.signalList.append(makeSignal("QUALITY_SAMPLE", `${(quality.data.qc_sample_defect_rate * 100).toFixed(1)}% defect rate`, "green"));
    showToast("Source records refreshed");
  } catch (error) {
    elements.systemLabel.textContent = "Unavailable";
    elements.evidenceHealth.textContent = "Source error";
    showToast(error.message, true);
  } finally {
    elements.refreshButton.disabled = false;
  }
}

function renderToolCalls(calls) {
  elements.toolTable.replaceChildren();
  calls.forEach((call, index) => {
    const row = document.createElement("tr");
    const number = document.createElement("td");
    number.className = "call-index";
    number.textContent = String(index + 1).padStart(2, "0");
    const tool = document.createElement("td");
    const code = document.createElement("code");
    code.textContent = call.tool_name;
    tool.append(code);
    const result = document.createElement("td");
    const chip = document.createElement("span");
    chip.className = "result-chip";
    chip.textContent = call.auth_decision;
    result.append(chip);
    const evidence = document.createElement("td");
    const sourceCount = ["records", "events", "inspections"]
      .map((key) => call.result_summary?.[key])
      .find(Array.isArray)?.length || 0;
    evidence.textContent = `${sourceCount} source ref${sourceCount === 1 ? "" : "s"}`;
    row.append(number, tool, result, evidence);
    elements.toolTable.append(row);
  });
  elements.toolCount.textContent = `${calls.length} tool calls`;
}

async function runInvestigation() {
  const message = elements.question.value.trim();
  if (!message) {
    showToast("Enter an investigation question", true);
    return;
  }
  elements.runButton.disabled = true;
  setRunStatus("Running", "running");
  elements.agentAnswer.textContent = "Executing the four mandatory source queries…";
  elements.answerCard.classList.add("empty");
  elements.answerMeta.replaceChildren();
  try {
    const run = await requestJson("/api/v1/agent/runs", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        agent_id: "production-analyst",
        factory_id: scope.factoryId,
        line_id: scope.lineId,
        start_at: scope.startAt,
        end_at: scope.endAt,
        message,
      }),
    });
    const calls = await requestJson(`/api/v1/agent/runs/${run.data.run_id}/tool-calls`);
    elements.agentAnswer.textContent = run.data.answer;
    elements.answerCard.classList.remove("empty");
    for (const text of [
      `${calls.data.length} mandatory calls completed`,
      `${run.data.evidence.length} evidence references`,
      `Run ${run.data.run_id.slice(0, 8)}`,
    ]) {
      const tag = document.createElement("span");
      tag.textContent = text;
      elements.answerMeta.append(tag);
    }
    renderToolCalls(calls.data);
    setRunStatus("Completed", "success");
    showToast("Investigation completed with rebuilt evidence");
  } catch (error) {
    elements.agentAnswer.textContent = `The investigation failed closed: ${error.message}`;
    setRunStatus("Failed closed", "failed");
    showToast(error.message, true);
  } finally {
    elements.runButton.disabled = false;
  }
}

elements.refreshButton.addEventListener("click", loadDashboard);
elements.runButton.addEventListener("click", runInvestigation);
loadDashboard();
