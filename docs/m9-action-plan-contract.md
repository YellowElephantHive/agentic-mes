# M9 deterministic disposition planning and dry-run contract

Status: **FROZEN FOR IMPLEMENTATION**

Frozen on 2026-11-04 as the binding contract for M9.1 through M9.6. M9.0
defines behavior, authority, lifecycle, drift detection, simulation isolation,
and evaluation boundaries only. It does not claim that action plans or dry-run
endpoints are implemented.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **SHOULD NOT** describe
requirements that later M9 code and tests must enforce.

## Objective

Transform one independently reviewed M8 disposition proposal into a typed,
deterministically validated, reproducibly simulated action plan that a human
may approve as a record.

M9 extends the existing flow:

```text
read-only diagnosis
    -> evidence-bound M8 proposal
    -> independent M8 review
    -> typed M9 action plan
    -> deterministic validation and state-drift check
    -> isolated dry-run
    -> independent human plan review
    -> approved or rejected plan record
```

Plan approval means that a human accepted the proposed investigation plan. It
MUST NOT execute a step, create a real maintenance or quality ticket, mutate
MES/ERP/QMS/WMS data, hold or release production, change inventory or
genealogy, contact equipment, or actuate a PLC.

## Roadmap alignment guard

M9 preserves the direction established by M0 through M8:

- deterministic code owns rules, scope, lifecycle, and authorization;
- model output is untrusted and cannot create authority;
- evidence comes from persisted executed tool results and reviewed M8 records;
- missing manufacturing values remain missing;
- original records and provenance remain unchanged;
- humans retain responsibility for operational decisions;
- Track A and Track B remain independent mandatory regression gates; and
- no new idea may silently introduce production execution.

Material discrepancy remains a restricted reference case. M9 may plan a
physical identity inspection or inventory reconciliation, but AI4I, SECOM,
simulator telemetry, or an M8 `inconclusive` proposal cannot establish that
material substitution occurred.

## Current baseline

Before M9:

- M8 creates evidence-bound maintenance, quality, and bounded material
  investigation proposals;
- only a different authorized human can approve or reject a pending proposal;
- proposal content and evidence are immutable after submission;
- approval creates no operational side effect;
- M7 authentication, local membership, tenant/factory scope, RBAC, resource
  hiding, and sanitized audits remain authoritative; and
- there is no action-plan schema, validator, state snapshot, simulation,
  action-plan review, or execution endpoint.

M9 MUST preserve every M7 and M8 security, isolation, audit, idempotency,
optimistic-concurrency, evidence, and no-execution property.

## Trust and authority boundary

```text
Untrusted question and model output
              |
              v
M8 review-approved proposal
  |-- immutable proposal version
  |-- independent review record
  |-- executed-evidence bindings
              |
              v
Deterministic plan compiler
  |-- frozen plan and step registry
  |-- server-derived scope and targets
  |-- bounded lists and text
              |
              v
Deterministic validator
  |-- current trusted target state
  |-- canonical snapshot hash
  |-- preconditions and freshness
              |
              v
Isolated simulator
  |-- copied or synthetic state only
  |-- deterministic seed and input hash
  |-- no operational table mutation
              |
              v
Independent human plan review
              |
              +-- approved plan record
              +-- NO EXECUTION AUTHORITY
```

The model MAY draft bounded explanatory text. It MUST NOT choose plan type,
steps, tenant, factory, creator, reviewer, status, source proposal or review,
targets, evidence, preconditions, snapshot inputs, simulation outcome,
timestamps, expiry, capabilities, or audit outcome.

## Frozen action-plan and step registry

M9 begins with three plan types.

| Plan type | Required M8 proposal type | Allowed declarative step codes |
|---|---|---|
| `maintenance_inspection_plan` | `maintenance_investigation` | `machine_inspection_check`, `downtime_history_review`, `maintenance_evidence_collection`, `maintenance_role_notification` |
| `quality_investigation_plan` | `quality_investigation` | `quality_history_review`, `quality_sample_collection`, `production_records_review`, `quality_role_notification` |
| `inventory_reconciliation_plan` | `material_discrepancy_investigation` | `material_identity_inspection`, `inventory_records_reconciliation`, `physical_observation_collection`, `quality_role_notification` |

Step codes describe a proposed future human-controlled investigation. Their
presence in a plan does not perform the named activity.

Unknown plan types, cross-type steps, duplicate steps, model-created codes,
and opaque action payloads MUST fail validation. Adding a type or step requires
a reviewed contract change, typed validation, authorization analysis, and
evaluation evidence.

## Conceptual typed action plan

The implementation MAY normalize storage, but MUST preserve these semantics:

```rust
struct ActionPlan {
    id: Uuid,
    version: i64,
    tenant_id: Uuid,
    factory_id: Uuid,
    created_by_user_id: Uuid,
    source_proposal_id: Uuid,
    source_proposal_version: i64,
    source_review_id: Uuid,
    plan_type: ActionPlanType,
    status: ActionPlanStatus,
    target_refs: Vec<ResourceRef>,
    evidence_refs: Vec<EvidenceRef>,
    steps: Vec<ActionPlanStep>,
    preconditions: Vec<PlanPrecondition>,
    source_snapshot_hash: Option<String>,
    latest_simulation_id: Option<Uuid>,
    summary: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    validated_at: Option<DateTime<Utc>>,
    submitted_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}
```

Required properties:

- IDs, scope, actor, versions, status, timestamps, and expiry are server owned;
- the source proposal is `review_approved`, in the same tenant/factory, and has
  exactly one persisted independent review;
- source proposal version, review ID, targets, and evidence are server rebuilt;
- plan type is deterministically derived from proposal type;
- steps come only from the frozen registry;
- text, targets, evidence, steps, preconditions, findings, and simulation
  output have explicit size/count bounds;
- submitted plan content, bindings, snapshot, and simulation reference are
  immutable; and
- a revised plan is a new record linked to the superseded plan.

An action plan MUST NOT contain executable SQL, provider-selected URLs, shell
commands, arbitrary HTTP requests, arbitrary tool calls, PLC instructions,
credentials, external-system payloads, or model-selected reviewers.

## Preconditions and source eligibility

Plan creation and every later mutation MUST fail closed unless:

1. the source proposal exists in the authenticated tenant/factory;
2. its status is `review_approved`;
3. its independent M8 review exists and matches the proposal version history;
4. the caller can read the source proposal and create the derived plan;
5. every target and evidence reference remains resolvable in scope;
6. the plan type and steps match the frozen registry;
7. missing values remain explicit and are never imputed;
8. no source record or model text is interpreted as an instruction; and
9. material evidence state is not upgraded by plan creation or simulation.

Rejected, cancelled, expired, draft, or pending M8 proposals MUST NOT produce
an M9 plan. Approval of an inconclusive M8 proposal permits only an
investigation plan that explicitly retains the inconclusive evidence state.

## Canonical snapshot and state-drift contract

Validation MUST read current trusted state through tenant/factory-scoped
repositories and construct a canonical snapshot containing, at minimum:

- source proposal ID and immutable version;
- source review ID and decision;
- ordered target references and their current trusted state/version;
- ordered evidence references and stable provenance identifiers;
- relevant work-order, line, equipment, quality, or inventory state when that
  state is available through a reviewed adapter;
- explicit missingness markers; and
- validation-policy version.

The server computes `source_snapshot_hash` as SHA-256 over a versioned,
canonical serialization. Raw sensitive payloads are not embedded in the hash
record.

Simulation, submission, and review MUST rebuild the current snapshot. If it
differs from the validated snapshot, the operation MUST NOT continue and the
plan transitions to `invalidated_by_drift`. A new plan or explicit revision is
required. The model cannot waive drift or select which changed fields to
ignore.

Examples of drift include a changed work-order state, new quality result,
closed maintenance condition, removed target, altered evidence provenance, or
changed source proposal/review binding.

## Dry-run isolation contract

Simulation MUST be deterministic and isolated from operational state.

It MAY use:

- a copied in-memory state;
- a transaction that is guaranteed to roll back and is independently checked;
- dedicated simulation tables; or
- the deterministic factory simulator with a recorded seed.

It MUST record:

- plan ID/version and source snapshot hash;
- simulator/policy version and deterministic seed when applicable;
- canonical input hash;
- bounded affected-resource references;
- precondition findings and missing-data findings;
- required human roles;
- a bounded impact preview;
- output hash and completion status; and
- start/end timestamps and duration.

It MUST NOT modify operational MES tables, dataset raw records, source
proposals, source reviews, evidence records, inventory, genealogy, routing,
recipes, quality decisions, or equipment state. Tests MUST compare protected
row counts and content hashes before and after simulation.

A simulation result is an impact preview, not a prediction guarantee and not
evidence that a physical action occurred.

## Action-plan lifecycle

```text
DRAFT -> VALIDATED -> SIMULATED -> PENDING_APPROVAL -> PLAN_APPROVED
   |         |            |               |----------> PLAN_REJECTED
   |         |            |               |----------> EXPIRED
   |         |            |-----------> INVALIDATED_BY_DRIFT
   |         |------------------------> INVALIDATED_BY_DRIFT
   |----------------------------------> INVALIDATED_BY_DRIFT
   |----------------------------------> CANCELLED
             |------------------------> CANCELLED
                          |-----------> CANCELLED
```

Rules:

- only trusted application code changes status;
- validation is required before simulation;
- one completed simulation of the current validated snapshot is required
  before submission;
- snapshot equality is rechecked before simulation, submission, and review;
- submission freezes plan content, bindings, snapshot, and simulation result;
- the reviewer MUST differ from the plan creator;
- the M8 proposal reviewer MAY review the M9 plan if they did not create it;
- the Agent cannot submit, approve, reject, cancel, or select a reviewer;
- expiry uses the server clock;
- cancellation and drift invalidation preserve history;
- terminal states cannot transition; and
- `plan_approved` grants no execution permission.

## Capabilities and separation of duties

M9 introduces these capabilities:

| Capability | Meaning |
|---|---|
| `create:action_plans` | Create a draft from an authorized M8 approved proposal |
| `read:action_plans:self` | Read plans created by the current user |
| `read:action_plans:tenant` | Read plans within permitted tenant/factory scope |
| `validate:action_plans` | Run deterministic validation |
| `simulate:action_plans` | Run the isolated deterministic dry-run |
| `submit:action_plans` | Submit one's simulated plan for review |
| `review:action_plans` | Approve or reject another user's submitted plan |

Recommended role bundles:

- `viewer` receives no M9 capability;
- `production_analyst` receives create, self-read, validate, simulate, and
  submit capabilities;
- `supervisor` and `admin` additionally receive tenant-read and review; and
- no role receives action-plan execution, MES write, ERP/QMS/WMS write, or PLC
  control capability.

Possessing a review capability never bypasses tenant/factory scope,
source-resource visibility, or separation of duties.

## Planned HTTP boundary

Later M9 milestones MAY implement:

```http
POST /api/v1/disposition-proposals/{proposal_id}/action-plans
GET  /api/v1/action-plans/{plan_id}
POST /api/v1/action-plans/{plan_id}/validate
POST /api/v1/action-plans/{plan_id}/simulate
POST /api/v1/action-plans/{plan_id}/submit
POST /api/v1/action-plans/{plan_id}/reviews
POST /api/v1/action-plans/{plan_id}/cancel
```

All routes require M7 authentication. State-changing routes require an
`Idempotency-Key` and positive optimistic `If-Match` version. Reusing a key
with a different actor, operation, resource, expected version, or request hash
returns a conflict.

There is deliberately no `/execute` route and no external-system export route
in M9.

## Planned persistence boundary

M9.1 SHOULD model at least:

```text
action_plans
action_plan_targets
action_plan_evidence
action_plan_steps
action_plan_preconditions
action_plan_snapshots
action_plan_simulations
action_plan_reviews
action_plan_transition_audits
```

Required database properties:

- every row is tenant scoped and plan factory scope is explicit;
- source proposal and review references use scoped foreign keys;
- steps are constrained to the typed registry;
- reviews, simulations, snapshots, and transition audits are append-only;
- submitted plan content is immutable;
- optimistic versions prevent lost updates;
- idempotency keys are actor/operation scoped;
- terminal records cannot be silently deleted; and
- historical identity suspension or deletion cannot orphan audit history.

Original MES, simulator, evaluation-dataset, M8 proposal, and M8 review records
MUST remain unchanged.

## Audit contract

Successful and denied create, read, validate, simulate, submit, cancel,
expire, drift-invalidate, and review operations MUST record:

- request ID, actor, tenant, and factory;
- plan ID/version and source proposal/review IDs when known;
- capability and authorization outcome;
- prior/resulting status and versions for completed transitions;
- validation policy version and snapshot hash when available;
- simulation ID, input/output hashes, and outcome when available;
- sanitized denial, validation, or drift category;
- idempotency outcome and duration; and
- bounded evidence, target, step, and finding counts.

Audits MUST NOT contain bearer tokens, provider secrets, raw prompts, complete
tool results, unbounded source records, or unredacted free-form payloads.

## Error and resource-hiding semantics

M9 retains the standard error envelope and request ID.

| Situation | HTTP status | Stable error code |
|---|---:|---|
| Missing/invalid authentication | 401 | `authentication_required` |
| Missing capability | 403 | `authorization_denied` |
| Resource outside authorized scope | 404 | `resource_not_found` |
| Invalid plan, step, transition, precondition, or simulation input | 422 | `invalid_action_plan` |
| Detected state drift | 409 | `action_plan_drifted` |
| Stale version or incompatible idempotency reuse | 409 | `action_plan_conflict` |
| Authentication provider unavailable | 503 | `authentication_unavailable` |

A denial MUST NOT reveal another tenant's proposal, plan, creator, reviewer,
snapshot, simulation, target, evidence, or status.

## Mandatory security and regression tests

M9 is not complete until automated tests cover all of the following:

1. an authorized `review_approved` M8 proposal creates one typed draft;
2. draft, pending, rejected, cancelled, expired, or absent M8 proposals cannot
   create a plan;
3. cross-tenant and cross-factory source proposals are hidden;
4. plan type is deterministically derived and cannot be overridden;
5. unknown and cross-type step codes are rejected;
6. model-supplied tenant, factory, actor, reviewer, status, targets, evidence,
   preconditions, snapshot, simulation outcome, and expiry cannot override
   trusted values;
7. targets and evidence resolve to the source proposal and executed evidence;
8. missing manufacturing values remain explicit and are not imputed;
9. source-record prompt injection cannot add steps, authority, or conclusions;
10. maintenance, quality, and inventory-reconciliation profiles remain
    independent;
11. material plans preserve `inconclusive` unless eligible independent physical
    evidence exists;
12. AI4I and SECOM cannot produce a confirmed material-mismatch claim;
13. viewer creation, validation, simulation, submission, and review return 403;
14. analyst self-read succeeds and cross-user read is hidden;
15. supervisor tenant/factory reads remain scope bounded;
16. validation produces a canonical versioned SHA-256 snapshot;
17. identical trusted inputs produce the same snapshot hash;
18. changed target state or evidence produces a different snapshot hash;
19. drift before simulation, submission, or review invalidates the plan;
20. drift cannot be waived by the model, creator, reviewer, or admin role;
21. simulation requires a currently validated plan;
22. identical simulation inputs and seed produce identical output hashes;
23. dry-run changes no protected operational, dataset, proposal, review, or
    evidence records;
24. simulation output is bounded and contains no executable payload;
25. valid submission freezes plan content, bindings, snapshot, and simulation;
26. plan creators cannot review their own plans;
27. users without review capability cannot submit a review;
28. valid independent approval records acceptance without executing a step;
29. rejection, cancellation, expiry, and invalidation preserve history;
30. invalid and terminal-state transitions fail closed;
31. stale optimistic versions return 409;
32. identical idempotent retries return the original result;
33. incompatible idempotency-key reuse returns 409;
34. authorization, validation, drift, simulation, transition, and review
    audits are complete and sanitized;
35. no route, tool, capability, record, or simulation can execute or export an
    MES/ERP/QMS/WMS/PLC write;
36. existing M7 authentication and authorization suites remain PASS;
37. existing M8 proposal and review suites remain PASS;
38. repository tenant/factory isolation remains PASS;
39. the deterministic simulator/Agent walkthrough remains PASS;
40. Track A remains PASS independently; and
41. Track B remains PASS independently.

Track A may ground maintenance-plan regression cases. Track B may ground
quality-plan regression cases. Neither track becomes a material-identity
dataset, and passing either track does not validate material-substitution
detection or compensate for the other track.

## Delivery sequence

- **M9.0:** freeze this contract and verify all existing regression gates.
- **M9.1:** add typed plan persistence, source bindings, idempotency, and
  authorized create/read APIs.
- **M9.2:** add the deterministic proposal-to-plan compiler and frozen step
  profiles.
- **M9.3:** add validation, canonical snapshots, freshness rules, and drift
  invalidation.
- **M9.4:** add deterministic isolated simulation and protected-state checks.
- **M9.5:** add submission, independent plan review, cancellation, expiry,
  optimistic versions, and transition audit.
- **M9.6:** complete the adversarial suite, formal M9 report, and independent
  Track A/Track B regression evidence.

Each sub-milestone MUST pass local regression, GitHub CI, and both independent
evaluation tracks before being recorded as passing. Passing one track never
compensates for the other.

## Explicit non-goals

M9 does not implement:

- an `/execute` endpoint or execution capability;
- MES, ERP, QMS, WMS, warehouse, inventory, genealogy, or PLC writes;
- hold, release, rework, scrap, substitution, recipe, routing, scheduling, or
  equipment actuation;
- real ticket creation, notification delivery, or external-system export;
- autonomous approval or model-selected reviewers;
- proof of physical root cause, material identity, or operator misconduct;
- SOP-to-state-machine compilation or deployment;
- multi-agent orchestration;
- production hosting, CD, or production certification.

Any future controlled-execution milestone requires a new frozen contract that
covers downstream authorization, least-privilege connectors, transaction and
state preconditions, idempotency, before/after audit, partial failure,
compensation or reversal, reconciliation, emergency stop, external-system
availability, and accountable human authorization. M9 approval grants none of
those authorities.
