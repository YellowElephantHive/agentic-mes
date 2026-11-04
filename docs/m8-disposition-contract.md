# M8 evidence-bound disposition contract

Status: **FROZEN FOR IMPLEMENTATION**

Frozen on 2026-11-03 as the binding contract for M8.1 through M8.4. M8.0
defines behavior, authority, evidence, lifecycle, and evaluation boundaries
only. It does not claim that disposition proposals or approval endpoints are
implemented.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **SHOULD NOT** describe
requirements that later M8 code and tests must enforce.

## Objective

Extend the current read-only, source-grounded Agent workflow with a typed,
evidence-bound disposition proposal that an authorized human may review.

M8 closes this loop:

```text
read-only diagnosis
    -> deterministic evidence reconstruction
    -> typed disposition proposal
    -> independent human review
    -> approved or rejected proposal record
```

Approval means that a human accepted the proposal record. It MUST NOT execute
the recommendation, mutate MES operational data, hold or release production,
change material genealogy, call an ERP, or actuate equipment.

## Roadmap alignment guard

M8 preserves the project direction established by M0 through M7:

- deterministic MES and authorization code remains authoritative;
- the Agent remains bounded by typed tools and server-derived scope;
- evidence comes from executed tool results, never model-claimed citations;
- missing manufacturing data remains missing;
- humans retain responsibility for operational decisions; and
- Track A and Track B remain independent mandatory regression gates.

New ideas MAY be added as proposal types or reference cases only when they fit
these boundaries and have source data that can be evaluated. A new idea MUST
NOT silently replace the M8 objective, widen Agent authority, or create a pass
claim unsupported by the frozen datasets.

## Current baseline

Before M8:

- Agent tools are read-only and use a server-created authorized context;
- completed runs and tool calls are persisted with executed evidence;
- final evidence is rebuilt from tool results and fails closed when mandatory
  calls are incomplete;
- M7 authenticates users, resolves local tenant/factory membership, enforces
  capabilities, and records authorization decisions; and
- there is no disposition-proposal persistence, review state, approval
  capability, or execution endpoint.

M8 MUST preserve every M7 authentication, authorization, scope, resource
hiding, and audit property.

## Trust and authority boundary

```text
Untrusted question and model output
              |
              v
Completed authorized Agent run
              |
              v
Server evidence rebuilder
  |-- persisted successful tool calls only
  |-- tenant/factory/run consistency
  |-- mandatory evidence profile
              |
              v
Typed proposal validator
  |-- allowlisted proposal and action codes
  |-- server-resolved resource references
  |-- bounded text and list sizes
              |
              v
Draft -> pending human review -> approved/rejected record
                                      |
                                      +-- NO MES execution
```

The model MAY draft bounded explanatory text and choose among allowlisted
proposal/action codes. It MUST NOT choose the tenant, factory, initiator,
reviewer, status, evidence references, target IDs, timestamps, expiry,
capabilities, or audit outcome. Trusted server code derives or validates those
values from persisted state.

## Frozen proposal types and action codes

M8 begins with a deliberately small registry.

| Proposal type | Purpose | Allowed recommendation codes |
|---|---|---|
| `maintenance_investigation` | Request human investigation of equipment or downtime evidence | `inspect_machine`, `request_maintenance_review` |
| `quality_investigation` | Request human investigation of quality or yield evidence | `request_quality_review`, `review_production_records` |
| `material_discrepancy_investigation` | Request physical material and inventory reconciliation without asserting fraud | `inspect_material_identity`, `reconcile_inventory`, `request_quality_review` |

An unknown type or action MUST fail validation. None of these codes performs
the named action; each is a recommendation for a human-controlled downstream
process.

Adding a proposal type or action code requires a reviewed contract change,
typed validation, authorization analysis, and evaluation evidence. The LLM
cannot create codes dynamically.

## Conceptual typed proposal

The implementation MAY use newtypes and normalized tables, but MUST preserve
these semantics:

```rust
struct DispositionProposal {
    id: Uuid,
    version: i64,
    tenant_id: Uuid,
    factory_id: Uuid,
    initiated_by_user_id: Uuid,
    source_run_id: Uuid,
    supersedes_proposal_id: Option<Uuid>,
    proposal_type: ProposalType,
    status: ProposalStatus,
    target_refs: Vec<ResourceRef>,
    evidence_refs: Vec<EvidenceRef>,
    evidence_state: EvidenceState,
    recommended_actions: Vec<RecommendationCode>,
    summary: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    submitted_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}
```

Required properties:

- IDs, scope, actor, status, version, and timestamps are server assigned;
- `source_run_id` references one completed run in the same tenant;
- target and evidence references are server rebuilt from that run's executed
  tool results or separately trusted physical evidence records;
- proposal text and lists have explicit size/count limits;
- recommendation codes are allowlisted enums, not free-form commands;
- expiry is bounded and server assigned;
- submitted content and evidence bindings are immutable; and
- a revision creates a new version that references the superseded proposal.

The proposal MUST NOT contain executable SQL, URLs selected by the model,
shell commands, PLC instructions, arbitrary tool calls, provider credentials,
or an opaque payload intended for an external MES.

## Evidence contract

Every proposal MUST be traceable to persisted, successfully executed tool
results. Trusted code MUST rebuild `evidence_refs`; model-supplied evidence or
target identifiers are advisory input at most and cannot be persisted without
resolution against the source run.

The evidence validator MUST enforce:

1. the source run is completed and belongs to the authenticated tenant;
2. the run factory equals the proposal factory;
3. every referenced tool call was authorized and completed;
4. every target ID appears in trusted executed results or a separately trusted
   repository record in the same scope;
5. the proposal type's mandatory evidence profile is satisfied;
6. missing values remain explicit and are never imputed by the Agent;
7. untrusted text inside records is treated as data, not instructions; and
8. unsupported causal or physical-identity conclusions are rejected.

Evidence profiles for the initial proposal registry:

- `maintenance_investigation` requires relevant equipment-downtime evidence
  and a bounded production window;
- `quality_investigation` requires quality-inspection evidence and a bounded
  production window; and
- `material_discrepancy_investigation` requires material, inventory, or
  independent physical-observation records supplied by a future reviewed data
  adapter. Current AI4I, SECOM, and simulator records do not satisfy this
  profile by themselves.

When required evidence is absent, the system MAY produce a bounded
`INCONCLUSIVE` proposal whose only recommendations request missing evidence.
It MUST NOT produce a supported causal conclusion or silently substitute a
different evidence profile.

## Evidence states and material-truth reference case

The frozen evidence states are:

| State | Meaning |
|---|---|
| `SUPPORTED` | Executed evidence supports requesting the stated investigation; it does not prove root cause |
| `SUSPECTED_MISMATCH` | Multiple source records conflict, but actual physical state is not independently proven |
| `INCONCLUSIVE` | Required evidence is missing or cannot distinguish the relevant states |
| `CONFIRMED_MATCH` | Trusted independent physical evidence agrees with the declared record |
| `CONFIRMED_MISMATCH` | Trusted independent physical evidence contradicts the declared record |

“Reported material A, physically used material B” is a reference case, not a
new project direction and not a currently validated capability.

- Process telemetry, inventory imbalance, or quality drift alone can support
  at most `SUSPECTED_MISMATCH`.
- `CONFIRMED_MATCH` or `CONFIRMED_MISMATCH` requires independent physical
  evidence such as a trusted RFID or weigh-scale observation with preserved
  provenance and matching scope/time.
- The proposal MUST describe a discrepancy or investigation need, not accuse
  an operator of fraud.
- AI4I and SECOM contain no declared-versus-actual material identity ground
  truth, so they MUST NOT be used to claim material-substitution detection.

Until an eligible adapter and evaluation source are reviewed, creation of a
material proposal beyond `INCONCLUSIVE` MUST fail closed.

## Proposal lifecycle

```text
DRAFT -> PENDING_REVIEW -> REVIEW_APPROVED
  |             |-------> REVIEW_REJECTED
  |             |-------> EXPIRED
  |-------> CANCELLED
  |             |-------> CANCELLED
```

Rules:

- only trusted application code changes status;
- submission freezes proposal content and evidence bindings;
- terminal states cannot transition;
- approval records acceptance of the recommendation only;
- approval MUST NOT trigger an external or operational write;
- the reviewer MUST be different from the initiator;
- expiry uses a server timestamp and is deterministic;
- cancellation does not delete the proposal or audit history; and
- revisions are new versioned proposals rather than edits to submitted data.

## Capabilities and separation of duties

M8 explicitly introduces these capabilities:

| Capability | Meaning |
|---|---|
| `create:disposition_proposals` | Create a draft from an authorized completed run |
| `read:disposition_proposals:self` | Read proposals initiated by the current user |
| `read:disposition_proposals:tenant` | Read proposals in the current tenant and permitted factory scope |
| `review:disposition_proposals` | Approve or reject another user's pending proposal |

M8 extends, but does not weaken, the M7 role bundles:

- `viewer` receives no disposition capability;
- `production_analyst` receives create and self-read capabilities;
- `supervisor` receives create, self-read, tenant-read, and review
  capabilities; and
- `admin` receives the same disposition capabilities as `supervisor`.

Possessing `review:disposition_proposals` does not bypass tenant/factory scope
or separation of duties. No role receives an execution capability in M8.

## Planned HTTP boundary

Later M8 milestones MAY implement:

```http
POST /api/v1/agent/runs/{run_id}/disposition-proposals
GET  /api/v1/disposition-proposals/{proposal_id}
POST /api/v1/disposition-proposals/{proposal_id}/submit
POST /api/v1/disposition-proposals/{proposal_id}/reviews
```

All routes require M7 authentication. State-changing routes require an
`Idempotency-Key` and an optimistic version precondition. Reusing a key with a
different authenticated actor, resource, or request hash MUST return a
conflict. The model never calls review endpoints and never supplies reviewer
identity.

There is deliberately no `/execute` route in M8.

## Planned persistence boundary

M8.1 SHOULD model at least:

```text
disposition_proposals
disposition_proposal_targets
disposition_proposal_evidence
disposition_proposal_actions
disposition_reviews
idempotency_records
```

Required database properties:

- every row is tenant scoped and proposal factory scope is explicit;
- source runs and evidence references use foreign keys where the current data
  model permits;
- review decisions are append-only;
- submitted proposal content is immutable;
- optimistic versions prevent lost updates;
- idempotency keys are unique within their actor/operation scope;
- terminal records cannot be silently deleted; and
- historical identity deletion or suspension cannot orphan proposal audit
  history.

Original MES, simulator, and dataset records MUST remain unchanged.

## Audit contract

Successful and denied create, read, submit, cancel, expire, and review
operations MUST record:

- request ID, actor, tenant, and factory;
- proposal ID/version and source run ID when known;
- capability and authorization outcome;
- prior and resulting proposal status for successful transitions;
- sanitized denial or validation category;
- idempotency outcome and duration; and
- evidence-reference count, never an unbounded sensitive payload.

Audit logs MUST NOT contain bearer tokens, provider secrets, raw model
prompts, complete tool results, or free-form payloads that have not passed
redaction and size limits.

## Error and resource-hiding semantics

M8 retains the standard error envelope and request ID.

| Situation | HTTP status | Stable error code |
|---|---:|---|
| Missing/invalid authentication | 401 | `authentication_required` |
| Missing create/read/review capability | 403 | `authorization_denied` |
| Resource absent or outside authorized tenant/factory scope | 404 | `resource_not_found` |
| Invalid type, action, transition, or evidence profile | 422 | `invalid_disposition_proposal` |
| Stale version or incompatible idempotency-key reuse | 409 | `proposal_conflict` |
| Authentication provider unavailable | 503 | `authentication_unavailable` |

A denial MUST NOT reveal another tenant's proposal, actor, evidence, or review
state.

## Mandatory security and regression tests

M8 is not complete until automated tests cover all of the following:

1. a completed authorized run can create a typed draft;
2. an incomplete, failed, or cross-tenant run cannot create a draft;
3. unknown proposal types and recommendation codes are rejected;
4. model-supplied tenant, factory, actor, reviewer, status, target, and
   evidence IDs cannot override trusted values;
5. every persisted evidence reference resolves to authorized executed
   evidence in the source scope;
6. missing mandatory evidence produces `INCONCLUSIVE` or fails closed;
7. missing manufacturing values are not imputed;
8. maintenance and quality evidence profiles are enforced independently;
9. material telemetry alone cannot produce `CONFIRMED_MISMATCH`;
10. material proposals beyond `INCONCLUSIVE` are rejected while no eligible
    physical-evidence adapter exists;
11. prompt injection in source records cannot add types, actions, evidence, or
    authority;
12. viewer creation and review attempts return 403;
13. analyst self-read succeeds and cross-user read is hidden;
14. supervisor tenant/factory read succeeds only within scope;
15. reviewers cannot approve their own proposals;
16. users without review capability cannot submit a review;
17. valid submission freezes content and evidence;
18. valid independent review records approval without executing an action;
19. invalid and terminal-state transitions fail closed;
20. expiry and cancellation preserve history;
21. a stale optimistic version returns 409;
22. identical idempotent retries return the original result;
23. incompatible idempotency-key reuse returns 409;
24. create, denial, transition, and review audits are complete and sanitized;
25. no route, tool, capability, or record can execute MES/ERP/PLC writes;
26. existing M7 authentication and authorization suites remain PASS;
27. repository tenant/factory isolation remains PASS;
28. the deterministic simulator/Agent walkthrough remains PASS;
29. Track A remains PASS independently; and
30. Track B remains PASS independently.

Track A and Track B remain regression and grounding gates under their frozen
datasets. They do not become material-identity datasets, and passing them does
not validate material-substitution detection.

## Delivery sequence

- **M8.0:** freeze this contract and verify all existing regression gates.
- **M8.1:** add typed proposal domain values, persistence, idempotency records,
  and authorized create/read APIs without review or execution.
- **M8.2:** generate proposals from server-rebuilt Agent evidence and enforce
  per-type evidence profiles and fail-closed validation.
- **M8.3:** add submit, independent human review, expiry/cancellation, status
  transition audit, and separation-of-duties enforcement.
- **M8.4:** complete the adversarial suite, formal M8 report, and independent
  Track A/Track B regression evidence.

Each sub-milestone MUST pass local tests and both independent evaluation
tracks before it is recorded as passing. Passing one track never compensates
for the other.

## Explicit non-goals

M8 does not implement:

- MES, ERP, warehouse, QMS, or PLC writes;
- hold, release, rework, scrap, recipe, routing, scheduling, or material
  substitution execution;
- autonomous approval or model-selected reviewers;
- proof of physical root cause or operator misconduct;
- material-substitution detection from AI4I or SECOM;
- SOP-to-state-machine compilation;
- multi-agent orchestration;
- a sophisticated frontend;
- CD, production hosting, or production certification.

Any future execution milestone requires a separate explicit contract covering
downstream authorization, transaction preconditions, idempotency, before/after
state audit, compensation or reversal, external-system failure recovery, and
human accountability. Approval in M8 grants none of those authorities.
