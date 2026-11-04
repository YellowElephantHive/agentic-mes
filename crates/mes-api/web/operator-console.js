"use strict";

const scope = {
  tenantId: "b242333d-c1c1-581c-a71f-ab8b8a92dc2a",
  factoryId: "8642cb8c-d730-5ce2-a6b0-17f17c2f2444",
  lineId: "90c31f01-e9c2-56d9-b365-9d0047c7939c",
  startAt: "2026-01-30T00:00:00Z",
  endAt: "2026-01-31T00:00:00Z",
  baselineStartAt: "2026-01-29T00:00:00Z",
};

const elements = {
  systemLabel: document.querySelector("#system-label"),
  systemPulse: document.querySelector(".pulse"),
  refreshButton: document.querySelector("#refresh-button"),
  scenarioSelect: document.querySelector("#scenario-select"),
  scopeWindow: document.querySelector("#scope-window"),
  scopeName: document.querySelector("#scope-name"),
  queueRefresh: document.querySelector("#queue-refresh"),
  queueType: document.querySelector("#queue-type"),
  queueStatus: document.querySelector("#queue-status"),
  queueList: document.querySelector("#queue-list"),
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
  workflowStatus: document.querySelector("#workflow-status"),
  proposalStatus: document.querySelector("#proposal-status"),
  proposalMeta: document.querySelector("#proposal-meta"),
  createProposal: document.querySelector("#create-proposal"),
  submitProposal: document.querySelector("#submit-proposal"),
  approveProposal: document.querySelector("#approve-proposal"),
  rejectProposal: document.querySelector("#reject-proposal"),
  proposalRationale: document.querySelector("#proposal-rationale"),
  planStatus: document.querySelector("#plan-status"),
  planMeta: document.querySelector("#plan-meta"),
  createPlan: document.querySelector("#create-plan"),
  validatePlan: document.querySelector("#validate-plan"),
  simulatePlan: document.querySelector("#simulate-plan"),
  submitPlan: document.querySelector("#submit-plan"),
  approvePlan: document.querySelector("#approve-plan"),
  rejectPlan: document.querySelector("#reject-plan"),
  planRationale: document.querySelector("#plan-rationale"),
  simulationSummary: document.querySelector("#simulation-summary"),
  feedbackStatus: document.querySelector("#feedback-status"),
  evidenceRating: document.querySelector("#evidence-rating"),
  realismRating: document.querySelector("#realism-rating"),
  nextSource: document.querySelector("#next-source"),
  missingFactor: document.querySelector("#missing-factor"),
  feedbackConsent: document.querySelector("#feedback-consent"),
  submitFeedback: document.querySelector("#submit-feedback"),
  toast: document.querySelector("#toast"),
};

const authHeaders = () => ({ "x-dev-tenant-id": scope.tenantId, "x-dev-factory-id": scope.factoryId });

const reviewerHeaders = Object.freeze({ "x-dev-user-id": "m9-demo-reviewer" });
const workflow = { runId: null, proposal: null, plan: null, simulation: null, busy: false };
let deepLinkRecovered = false;

async function requestJson(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { ...authHeaders(), ...(options.headers || {}) },
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

function operationHeaders(operation, record, reviewer = false) {
  const headers = {
    "content-type": "application/json",
    "idempotency-key": `m9.9-${operation}-${record?.id || workflow.runId}`,
  };
  if (record?.version !== undefined) headers["if-match"] = String(record.version);
  return reviewer ? { ...headers, ...reviewerHeaders } : headers;
}

function setRecordMeta(element, record) {
  const values = element.querySelectorAll("dd");
  values[0].textContent = record?.id || "—";
  values[0].title = record?.id || "";
  values[1].textContent = record?.version === undefined ? "—" : String(record.version);
}

function statusTone(status) {
  if (["review_approved", "plan_approved", "simulated", "validated"].includes(status)) return "success";
  if (["review_rejected", "plan_rejected", "cancelled", "expired", "invalidated_by_drift"].includes(status)) return "failed";
  return status ? "running" : "neutral";
}

function setWorkflowBusy(busy) {
  workflow.busy = busy;
  renderWorkflow();
}

function renderWorkflow() {
  const proposal = workflow.proposal;
  const plan = workflow.plan;
  elements.proposalStatus.textContent = proposal?.status || "Not created";
  elements.proposalStatus.className = `status-pill ${statusTone(proposal?.status)}`;
  elements.planStatus.textContent = plan?.status || (proposal?.status === "review_approved" ? "Ready" : "Locked");
  elements.planStatus.className = `status-pill ${statusTone(plan?.status)}`;
  setRecordMeta(elements.proposalMeta, proposal);
  setRecordMeta(elements.planMeta, plan);

  const disabled = workflow.busy;
  elements.createProposal.disabled = disabled || !workflow.runId || Boolean(proposal);
  elements.submitProposal.disabled = disabled || proposal?.status !== "draft";
  elements.approveProposal.disabled = disabled || proposal?.status !== "pending_review";
  elements.rejectProposal.disabled = disabled || proposal?.status !== "pending_review";
  elements.createPlan.disabled = disabled || proposal?.status !== "review_approved" || Boolean(plan);
  elements.validatePlan.disabled = disabled || plan?.status !== "draft";
  elements.simulatePlan.disabled = disabled || plan?.status !== "validated";
  elements.submitPlan.disabled = disabled || plan?.status !== "simulated";
  elements.approvePlan.disabled = disabled || plan?.status !== "pending_approval";
  elements.rejectPlan.disabled = disabled || plan?.status !== "pending_approval";

  let label = "Waiting for investigation";
  let tone = "neutral";
  if (workflow.runId) { label = "Evidence ready"; tone = "running"; }
  if (proposal?.status === "pending_review" || plan?.status === "pending_approval") label = "Human review required";
  if (proposal?.status === "review_approved") label = "Proposal approved";
  if (plan?.status === "plan_approved") { label = "Plan approved · no execution"; tone = "success"; }
  if ([proposal?.status, plan?.status].some((value) => ["review_rejected", "plan_rejected", "invalidated_by_drift"].includes(value))) { label = "Workflow closed"; tone = "failed"; }
  elements.workflowStatus.textContent = workflow.busy ? "Recording transition" : label;
  elements.workflowStatus.className = `status-pill ${workflow.busy ? "running" : tone}`;
  elements.submitFeedback.disabled = workflow.busy || !workflow.runId;
  elements.feedbackStatus.textContent = workflow.runId ? "Ready for annotation" : "Requires an investigation";
  elements.feedbackStatus.className = `status-pill ${workflow.runId ? "running" : "neutral"}`;
}

function dayBefore(iso) {
  return new Date(Date.parse(iso) - 86_400_000).toISOString();
}

async function loadSandboxScope() {
  const response = await requestJson("/api/v1/sandbox/scope");
  const presets = response.data.scenario_presets;
  elements.scenarioSelect.replaceChildren();
  for (const preset of presets) {
    const option = document.createElement("option");
    option.value = JSON.stringify(preset);
    option.textContent = `${preset.line_name} · ${preset.label}`;
    if (preset.line_id === scope.lineId && preset.start_at === scope.startAt) option.selected = true;
    elements.scenarioSelect.append(option);
  }
}

async function applyScenario() {
  const preset = JSON.parse(elements.scenarioSelect.value);
  scope.factoryId = preset.factory_id;
  scope.lineId = preset.line_id;
  scope.startAt = preset.start_at;
  scope.endAt = preset.end_at;
  scope.baselineStartAt = dayBefore(preset.start_at);
  elements.scopeWindow.textContent = `${preset.line_name} · ${preset.start_at.slice(0, 10)} UTC`;
  elements.scopeName.textContent = `${preset.factory_name || "Demo Factory"} / ${preset.line_name}`;
  elements.question.value = preset.question;
  workflow.runId = null; workflow.proposal = null; workflow.plan = null; workflow.simulation = null;
  renderWorkflow();
  await Promise.all([loadDashboard(), loadWorkQueue()]);
}

function queueLabel(type) {
  return ({agent_run:"Investigation",disposition_proposal:"Disposition proposal",action_plan:"Action plan"})[type] || type;
}

async function loadWorkQueue() {
  elements.queueRefresh.disabled = true;
  try {
    const params = new URLSearchParams({ limit: "30", offset: "0" });
    if (elements.queueType.value) params.set("record_type", elements.queueType.value);
    if (elements.queueStatus.value.trim()) params.set("status", elements.queueStatus.value.trim());
    const response = await requestJson(`/api/v1/work-queue?${params}`);
    elements.queueList.replaceChildren();
    if (!response.data.items.length) {
      const empty = document.createElement("p"); empty.textContent = "No authorized cases match this filter."; elements.queueList.append(empty); return;
    }
    for (const item of response.data.items) {
      const button = document.createElement("button"); button.type = "button"; button.className = "queue-item";
      const kind = document.createElement("span"); kind.className = "queue-kind"; kind.textContent = queueLabel(item.record_type);
      const title = document.createElement("strong"); title.textContent = item.status;
      const meta = document.createElement("small"); meta.textContent = `${item.id.slice(0,8)} · ${new Date(item.created_at).toLocaleString()}`;
      button.append(kind,title,meta); button.addEventListener("click", () => recoverCase(item)); elements.queueList.append(button);
    }
    if (!deepLinkRecovered && window.location.hash.startsWith("#case=")) {
      deepLinkRecovered = true;
      const [recordType, id] = window.location.hash.slice(6).split(":");
      const item = response.data.items.find((value) => value.record_type === recordType && value.id === id);
      if (item) await recoverCase(item);
    }
  } catch (error) { elements.queueList.textContent = error.message; }
  finally { elements.queueRefresh.disabled = false; }
}

async function restoreRun(runId) {
  const [run, calls] = await Promise.all([
    requestJson(`/api/v1/agent/runs/${runId}`), requestJson(`/api/v1/agent/runs/${runId}/tool-calls`)
  ]);
  workflow.runId = runId;
  elements.agentAnswer.textContent = run.data.final_response || `Run status: ${run.data.status}`;
  elements.answerCard.classList.remove("empty");
  renderToolCalls(calls.data);
  setRunStatus(run.data.status, run.data.status === "completed" ? "success" : "running");
}

async function recoverCase(item) {
  try {
    workflow.proposal = null; workflow.plan = null; workflow.simulation = null;
    if (item.record_type === "agent_run") await restoreRun(item.id);
    if (item.record_type === "disposition_proposal") {
      const response = await requestJson(`/api/v1/disposition-proposals/${item.id}`);
      workflow.proposal = response.data; await restoreRun(response.data.source_run_id);
    }
    if (item.record_type === "action_plan") {
      const plan = await requestJson(`/api/v1/action-plans/${item.id}`);
      workflow.plan = plan.data;
      const proposal = await requestJson(`/api/v1/disposition-proposals/${plan.data.source_proposal_id}`);
      workflow.proposal = proposal.data; await restoreRun(proposal.data.source_run_id);
      if (["simulated", "pending_approval", "plan_approved", "plan_rejected"].includes(workflow.plan.status)) {
        elements.simulationSummary.textContent = `Persisted plan status: ${workflow.plan.status}. Open the audit API for the original dry-run record.`;
        elements.simulationSummary.classList.add("success");
      }
    }
    window.location.hash = `case=${item.record_type}:${item.id}`;
    renderWorkflow(); showToast("Case recovered from the authorized work queue");
  } catch (error) { showToast(error.message, true); }
}

async function workflowRequest(label, callback) {
  setWorkflowBusy(true);
  try {
    await callback();
    await loadWorkQueue();
    showToast(label);
  } catch (error) {
    showToast(error.message, true);
  } finally {
    setWorkflowBusy(false);
  }
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
    workflow.runId = run.data.run_id;
    workflow.proposal = null;
    workflow.plan = null;
    workflow.simulation = null;
    elements.simulationSummary.textContent = "No dry-run has been performed.";
    elements.simulationSummary.classList.remove("success");
    renderWorkflow();
    await loadWorkQueue();
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

async function createProposal() {
  await workflowRequest("Evidence-bound proposal created", async () => {
    const response = await requestJson(`/api/v1/agent/runs/${workflow.runId}/disposition-proposals`, {
      method: "POST",
      headers: operationHeaders("proposal-create"),
      body: JSON.stringify({ proposal_type: "maintenance_investigation" }),
    });
    workflow.proposal = response.data;
  });
}

async function submitProposal() {
  await workflowRequest("Proposal submitted for independent review", async () => {
    const response = await requestJson(`/api/v1/disposition-proposals/${workflow.proposal.id}/submit`, {
      method: "POST",
      headers: operationHeaders("proposal-submit", workflow.proposal),
      body: "{}",
    });
    workflow.proposal = response.data.proposal;
  });
}

async function reviewProposal(decision) {
  const rationale = elements.proposalRationale.value.trim();
  if (!rationale) return showToast("Reviewer rationale is required", true);
  await workflowRequest(`Proposal ${decision}`, async () => {
    const response = await requestJson(`/api/v1/disposition-proposals/${workflow.proposal.id}/reviews`, {
      method: "POST",
      headers: operationHeaders(`proposal-${decision}`, workflow.proposal, true),
      body: JSON.stringify({ decision, rationale }),
    });
    workflow.proposal = response.data.proposal;
  });
}

async function createPlan() {
  await workflowRequest("Deterministic action plan compiled", async () => {
    const response = await requestJson(`/api/v1/disposition-proposals/${workflow.proposal.id}/action-plans`, {
      method: "POST",
      headers: operationHeaders("plan-create", workflow.proposal),
      body: "{}",
    });
    workflow.plan = response.data;
  });
}

async function validatePlan() {
  await workflowRequest("Trusted-state snapshot validated", async () => {
    const response = await requestJson(`/api/v1/action-plans/${workflow.plan.id}/validate`, {
      method: "POST",
      headers: operationHeaders("plan-validate", workflow.plan),
      body: "{}",
    });
    workflow.plan = response.data.plan;
  });
}

async function simulatePlan() {
  await workflowRequest("Isolated deterministic dry-run completed", async () => {
    const response = await requestJson(`/api/v1/action-plans/${workflow.plan.id}/simulate`, {
      method: "POST",
      headers: operationHeaders("plan-simulate", workflow.plan),
      body: "{}",
    });
    workflow.plan = response.data.plan;
    workflow.simulation = response.data.simulation;
    elements.simulationSummary.textContent = `Dry-run seed ${workflow.simulation.deterministic_seed} · protected state unchanged: ${workflow.simulation.protected_state_unchanged} · no physical action occurred.`;
    elements.simulationSummary.classList.add("success");
  });
}

async function submitPlan() {
  await workflowRequest("Action plan submitted for independent approval", async () => {
    const response = await requestJson(`/api/v1/action-plans/${workflow.plan.id}/submit`, {
      method: "POST",
      headers: operationHeaders("plan-submit", workflow.plan),
      body: "{}",
    });
    workflow.plan = response.data.plan;
  });
}

async function reviewPlan(decision) {
  const rationale = elements.planRationale.value.trim();
  if (!rationale) return showToast("Reviewer rationale is required", true);
  await workflowRequest(`Action plan ${decision}; no execution occurred`, async () => {
    const response = await requestJson(`/api/v1/action-plans/${workflow.plan.id}/reviews`, {
      method: "POST",
      headers: operationHeaders(`plan-${decision}`, workflow.plan, true),
      body: JSON.stringify({ decision, rationale }),
    });
    workflow.plan = response.data.plan;
  });
}

async function submitFeedback() {
  if (!workflow.runId) return;
  elements.submitFeedback.disabled = true;
  try {
    await requestJson("/api/v1/domain-feedback", {
      method: "POST", headers: {"content-type":"application/json"},
      body: JSON.stringify({
        run_id: workflow.runId, proposal_id: workflow.proposal?.id || null, plan_id: workflow.plan?.id || null,
        evidence_rating: elements.evidenceRating.value, realism_rating: elements.realismRating.value,
        next_source: elements.nextSource.value, missing_factor: elements.missingFactor.value,
        consent_for_research: elements.feedbackConsent.checked,
      }),
    });
    elements.feedbackStatus.textContent = "Annotation saved"; elements.feedbackStatus.className = "status-pill success";
    showToast("Expert annotation saved separately from manufacturing ground truth");
  } catch (error) { showToast(error.message, true); }
  finally { elements.submitFeedback.disabled = false; }
}

elements.refreshButton.addEventListener("click", loadDashboard);
elements.scenarioSelect.addEventListener("change", applyScenario);
elements.queueRefresh.addEventListener("click", loadWorkQueue);
elements.queueType.addEventListener("change", loadWorkQueue);
elements.runButton.addEventListener("click", runInvestigation);
elements.createProposal.addEventListener("click", createProposal);
elements.submitProposal.addEventListener("click", submitProposal);
elements.approveProposal.addEventListener("click", () => reviewProposal("approved"));
elements.rejectProposal.addEventListener("click", () => reviewProposal("rejected"));
elements.createPlan.addEventListener("click", createPlan);
elements.validatePlan.addEventListener("click", validatePlan);
elements.simulatePlan.addEventListener("click", simulatePlan);
elements.submitPlan.addEventListener("click", submitPlan);
elements.approvePlan.addEventListener("click", () => reviewPlan("approved"));
elements.rejectPlan.addEventListener("click", () => reviewPlan("rejected"));
elements.submitFeedback.addEventListener("click", submitFeedback);
renderWorkflow();
Promise.all([loadSandboxScope(), loadDashboard(), loadWorkQueue()]).catch((error) => showToast(error.message, true));
