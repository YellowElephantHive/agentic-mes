# Formal M8 evidence-bound disposition report

Status: **PASS**

Contract: [M8 evidence-bound disposition contract](m8-disposition-contract.md)  
Completed: 2026-11-04

M8 is accepted only because all frozen mandatory cases below have automated
evidence and Track A and Track B passed independently. Approval remains a
recorded human decision; it grants no execution authority.

## Mandatory-case matrix

| # | Frozen gate | Automated evidence | Result |
|---:|---|---|---|
| 1 | Completed authorized run creates typed draft | PostgreSQL API create/read lifecycle | PASS |
| 2 | Incomplete, failed, and cross-tenant runs fail closed | Empty-evidence, failed-run, and cross-tenant API cases | PASS |
| 3 | Unknown types and actions rejected | API unknown-type case and domain registry tests | PASS |
| 4 | Untrusted scope, actors, status, targets, and evidence cannot override trusted values | Unknown-field injection request plus server-derived persisted values | PASS |
| 5 | Persisted evidence resolves to authorized executed evidence | Foreign-key join and authorized-tool-call assertions | PASS |
| 6 | Missing mandatory evidence is inconclusive or rejected | Independent profile fail-closed unit tests and empty-run rejection | PASS |
| 7 | Missing manufacturing values are not imputed | AI4I/SECOM adapter tests and disposition missing-data profile tests | PASS |
| 8 | Maintenance and quality profiles are independent | Profile-specific generation and evidence-count assertions | PASS |
| 9 | Telemetry cannot assert confirmed material mismatch | Material proposal remains `inconclusive` | PASS |
| 10 | Unsupported material proposal beyond inconclusive is rejected | Untrusted confirmed-mismatch/action injection case | PASS |
| 11 | Source-record prompt injection cannot expand authority | Malicious downtime reason-code generation test | PASS |
| 12 | Viewer creation and review return 403 | PostgreSQL API viewer cases | PASS |
| 13 | Analyst self-read succeeds; cross-user read is hidden | Self-read and cross-user 404 cases | PASS |
| 14 | Supervisor tenant/factory read is scope bounded | Supervisor success and out-of-scope factory denial | PASS |
| 15 | Reviewer cannot approve own proposal | Separation-of-duties API case | PASS |
| 16 | User without review capability cannot review | Viewer and analyst review denials | PASS |
| 17 | Submission freezes content and evidence | Summary/evidence preservation across submit/review | PASS |
| 18 | Independent review records decision without action execution | Separate reviewer approval/rejection plus no-execution checks | PASS |
| 19 | Invalid and terminal transitions fail closed | Missing precondition and terminal-review cases | PASS |
| 20 | Expiry and cancellation preserve history | Status/version and transition-audit assertions | PASS |
| 21 | Stale optimistic version returns 409 | Stale submit case | PASS |
| 22 | Identical idempotent retry returns original result | Create, submit, and review replay cases | PASS |
| 23 | Incompatible idempotency reuse returns 409 | Conflicting create request case | PASS |
| 24 | Create, denial, transition, and review audits are sanitized | Required action/outcome checks, safe schema checks, and marker leak scan | PASS |
| 25 | No route, tool, capability, or record executes MES/ERP/PLC writes | Missing `/execute`, role-capability, and source authority gates | PASS |
| 26 | M7 authn/authz remains passing | Full OIDC/JWKS, startup-policy, and authorization regression suites | PASS |
| 27 | Repository tenant/factory isolation remains passing | PostgreSQL repository and API integration suites | PASS |
| 28 | Deterministic simulator/Agent walkthrough remains passing | `scripts/reproduce-mvp.sh` M5 walkthrough | PASS |
| 29 | Track A remains independently passing | UCI AI4I job and report | PASS |
| 30 | Track B remains independently passing | UCI SECOM job and report | PASS |

## Delivered security and control properties

M8 provides typed allowlisted investigation proposals derived from persisted
executed tool results, server-owned tenant/factory/actor scope, independent
human approval or rejection, separation of duties, optimistic versions,
idempotent transitions, deterministic expiry, immutable submitted content,
and sanitized authorization and transition evidence.

The two dataset tracks remain grounding and regression gates. They are not
material-identity datasets and do not validate substitution detection.

## Verification record

- Implementation commit: `58870aa69c68513d8729a728c070744ca22f114c`.
- [GitHub Rust CI run 41777359996](https://github.com/YellowElephantHive/agentic-mes/actions/runs/41777359996):
  **PASS**.
- [GitHub dual-track run 41777359916](https://github.com/YellowElephantHive/agentic-mes/actions/runs/41777359916):
  **PASS**.
- Track A: **PASS**.
- Track B: **PASS**.
- Aggregate dual-track gate: **PASS**.

## Limits of this PASS

M8 does not implement MES, ERP, warehouse, QMS, inventory, material-genealogy,
or PLC writes. It does not perform hold, release, rework, scrap, substitution,
routing, scheduling, or equipment actuation. It does not validate physical
material identity, prove root cause or misconduct, provide production hosting,
or constitute production-security, regulatory, or MES certification.

Any future execution milestone requires a new frozen contract and independent
authorization, transaction, failure-recovery, compensation, and audit evidence.
