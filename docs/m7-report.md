# Formal M7 authentication and authorization report

Status: **PASS**

Contract: [M7 authentication contract](m7-auth-contract.md)  
Completed: 2026-11-03

M7 is accepted only because every frozen mandatory case below has automated
evidence and both dataset evaluation tracks passed independently.

## Mandatory-case matrix

| # | Frozen gate | Automated evidence | Result |
|---:|---|---|---|
| 1 | Valid token and local membership | Runtime-generated RSA OIDC principal test | PASS |
| 2 | Missing credential → 401 | Protected-endpoint Bearer-challenge test | PASS |
| 3 | Malformed token → 401 | Malformed JWT rejection plus frozen HTTP error mapping | PASS |
| 4 | Expired token → 401 | OIDC time-attack test | PASS |
| 5 | Premature `nbf` token → 401 | OIDC time-attack test | PASS |
| 6 | Modified signature → 401 | Signature-mutation test | PASS |
| 7 | Wrong issuer → 401 | OIDC issuer-attack test | PASS |
| 8 | Wrong audience → 401 | OIDC wrong-audience test | PASS |
| 9 | Disallowed algorithm and `alg=none` → 401 | HS256 and unsigned-token tests | PASS |
| 10 | Missing/unknown `kid` fails closed | Missing-`kid` and bounded unknown-`kid` refresh tests | PASS |
| 11 | JWKS unavailable without valid cache → 503 | Expired-cache/unavailable-JWKS test plus HTTP mapping | PASS |
| 12 | No local identity mapping → 403 | Unknown-subject repository test | PASS |
| 13 | Suspended user or membership → 403 | PostgreSQL API integration test | PASS |
| 14 | Insufficient capability → 403 | Viewer/zero-role route tests | PASS |
| 15 | Cross-tenant selection denied | PostgreSQL API integration test | PASS |
| 16 | Unauthorized factory denied | Membership-scope repository/API tests | PASS |
| 17 | Empty factory scope grants nothing | Explicit empty-scope API test | PASS |
| 18 | Self/tenant Agent-run visibility | Analyst/self and supervisor/tenant tests | PASS |
| 19 | Agent tool cannot expand HTTP scope | Strict tool arguments and server-derived-context tests | PASS |
| 20 | Non-development `dev_headers` rejected | Authentication startup-policy unit test | PASS |
| 21 | OIDC does not fall back to development headers | OIDC fallback adversarial test | PASS |
| 22 | Key rotation trusts new key, rejects old key | JWKS replacement/key-rotation test | PASS |
| 23 | No raw token in errors/audit | Response/debug redaction and audit-schema tests | PASS |
| 24 | Repository tenant/factory isolation | Ignored PostgreSQL integration suite | PASS |
| 25 | Track A independent gate | UCI AI4I evaluation job and report | PASS |
| 26 | Track B independent gate | UCI SECOM evaluation job and report | PASS |

## Security properties delivered

M7 provides explicit authentication-provider startup policy, RS256 OIDC/JWKS
verification, database-authoritative local membership and factory scope,
capability enforcement at HTTP and Agent-tool boundaries, self/tenant Agent
run visibility, resource hiding, bounded rate limiting, and durable sanitized
authorization decisions. Repository tenant/factory filters remain mandatory
below the authorization layer.

## Limits of this PASS

This report does not claim real production-IdP interoperability, a hosted
OAuth login flow, penetration testing, HA/DR, regulatory certification,
production deployment, MES writes, PLC control, approval workflows, SOP
compilation, event sourcing, or autonomous actions. Those require later,
separately frozen milestones and evidence.
