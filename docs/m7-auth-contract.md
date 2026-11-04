# M7 trusted identity and authorization contract

Status: **FROZEN FOR IMPLEMENTATION**

Frozen on 2026-11-03 as the binding contract for M7.1 through M7.3. M7.0
defines behavior and trust boundaries only; it does not claim that production
authentication is implemented.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **SHOULD NOT** describe
requirements that later M7 code and tests must enforce.

## Objective

Replace the development-header identity shortcut with a production-capable,
fail-closed identity and authorization boundary shared by HTTP routes and every
Agent tool call.

M7 establishes who is acting, which tenant and factories they may access, and
which capabilities they hold. It does not make model output trusted and does
not add MES writes.

## Current baseline

Before M7:

- protected routes accept `x-dev-tenant-id`, `x-dev-factory-id`, and optional
  `x-dev-user-id` only when development identity is enabled;
- non-development startup refuses to serve because production authentication
  is not implemented;
- Agent tools receive a server-created `AuthorizedToolContext` containing a
  user ID, tenant ID, permitted factory IDs, run ID, and capabilities;
- model-produced tool arguments cannot supply or override those fields;
- repositories impose tenant/factory filters below the HTTP boundary.

M7 MUST preserve those lower-layer authorization properties while replacing
the source of the trusted identity.

## Trust boundaries

```text
Untrusted client
    |
    | Authorization: Bearer <access token>
    | requested tenant/factory scope
    v
Authentication provider
    |-- verify token cryptographically
    |-- validate issuer, audience and time claims
    v
Authenticated subject (issuer, subject)
    |
    v
Local identity and membership lookup
    |-- internal user
    |-- active tenant membership
    |-- roles
    |-- permitted factories
    v
AuthenticatedPrincipal
    |                 \
    v                  v
HTTP authorization     AuthorizedToolContext
    |                  (server-derived subset)
    v                  |
Tenant-scoped          v
repository calls       per-tool authorization
```

Client headers, URL parameters, request bodies, JWT claims, database text,
tool output, and model output remain untrusted input. A valid signature proves
who issued a token; it does not by itself grant a local tenant, factory, role,
or capability.

## Authentication modes and startup policy

The application MUST use an explicit authentication provider:

```dotenv
AUTH_PROVIDER=dev_headers
```

or:

```dotenv
AUTH_PROVIDER=oidc
OIDC_ISSUER=https://identity.example.com/
OIDC_AUDIENCE=agentic-mes
```

An optional `OIDC_JWKS_URL` MAY override discovery for controlled deployments
and tests. Production deployments SHOULD use issuer discovery.

The startup matrix is binding:

| `APP_ENV` | `AUTH_PROVIDER` | Required behavior |
|---|---|---|
| `development` | `dev_headers` | Allowed |
| `development` | `oidc` | Allowed |
| any non-development value | `oidc` | Allowed when configuration validates |
| any non-development value | `dev_headers` | Refuse startup |
| any value | missing or unknown | Refuse startup |

There MUST be no fallback from failed OIDC authentication to development
headers. When OIDC is selected, `x-dev-*` identity headers MUST NOT influence
the principal.

The development adapter MUST still construct the same `AuthenticatedPrincipal`
shape. Its headers may select only a development identity and membership
created by the deterministic simulator or test fixture. The API MUST NOT
auto-provision an arbitrary caller-supplied `x-dev-user-id`, and callers cannot
supply roles or capabilities. The selected factory MUST exist in the selected
tenant and in that development membership. This preserves the local workflow
without making development headers a second production authorization model.

`/healthz` remains unauthenticated and reports only process liveness.
`/readyz` remains unauthenticated, reports only non-secret readiness metadata,
and MUST fail when the configured authentication provider cannot safely serve
requests. Every `/api/v1/**` route requires authentication unless a later
contract explicitly documents an exception.

## OIDC access-token validation

M7 accepts signed JWT bearer **access tokens**, not opaque tokens, browser
sessions, or an ID token used as an access token. Opaque-token introspection is
outside M7.

For every request, the OIDC provider MUST:

1. accept exactly one syntactically valid `Authorization: Bearer` value;
2. impose a bounded token length before parsing;
3. reject `alg=none`, symmetric algorithms, and algorithms outside an explicit
   asymmetric allowlist;
4. select a trusted JWK by `kid` and verify that key type and algorithm agree;
5. verify the signature before using claims for authorization;
6. require an exact configured issuer match;
7. require the configured audience to be present;
8. require a non-expired `exp` claim;
9. enforce `nbf` when present and reject a materially future `iat`;
10. permit only a small documented clock-skew allowance;
11. require a non-empty, bounded `sub` claim; and
12. reject malformed, duplicated, unsupported, or ambiguous token data.

JWKS retrieval MUST use HTTPS outside tests. Keys MUST be cached with bounded
lifetimes. An unknown `kid` MAY cause one bounded refresh, but refresh loops or
unbounded retries are forbidden. If no currently trusted key set is available,
the API MUST fail closed rather than accepting an unverifiable token.

Raw tokens, signatures, authorization headers, and complete JWT payloads MUST
NOT be logged or persisted. Authentication error responses MUST NOT reveal
which signature, key, membership, or claim comparison failed.

## Local identity is authoritative for authorization

The stable external identity key is the exact `(issuer, subject)` pair. It maps
to one internal user. The same subject string from a different issuer is a
different identity.

Signed token claims MAY help locate an identity, but the local database is
authoritative for:

- user active/suspended status;
- tenant membership;
- role assignment;
- factory scope; and
- effective capabilities.

An authenticated subject without an active local mapping is denied. A role,
tenant, factory, group, or capability claim MUST NOT grant access unless an
explicit future mapping contract allows it and the result is still bounded by
local membership.

## Requested tenant and factory scope

M7 supports users who may belong to more than one tenant. Production clients
select their requested context with:

```text
x-tenant-id: <uuid>
x-factory-id: <uuid>
```

These are untrusted scope selectors, not identity credentials. The server MUST
verify both against active local membership before constructing a principal.
Request body, path, or query factory IDs MUST also equal or fall within the
principal's permitted scope.

Development mode continues to use the clearly separate `x-dev-*` header names.
Production scope headers MUST never enable the development adapter, and
development headers MUST never authorize an OIDC request.

Cross-tenant repository filters remain mandatory even after middleware
authorization. Absence of factory scopes means access to no factories; it MUST
NOT be interpreted as all factories. An all-factory grant requires an explicit
stored flag or grant.

## Authenticated principal

Trusted server code constructs the following conceptual value:

```rust
struct AuthenticatedPrincipal {
    user_id: Uuid,
    external_identity_id: Uuid,
    issuer: String,
    subject: String,
    tenant_id: Uuid,
    permitted_factory_ids: Vec<Uuid>,
    roles: Vec<Role>,
    capabilities: Vec<Capability>,
    auth_method: AuthMethod,
}
```

The concrete implementation MAY use newtypes and sets, but it MUST preserve
these semantics. `issuer` and `subject` are identity metadata, not values that
may be forwarded to an LLM.

`AuthorizedToolContext` MUST be derived from this principal plus the persisted
Agent run ID. It MUST be a capability-preserving or capability-reducing
projection: an Agent or request payload cannot add factories or capabilities.

## Roles and capabilities

Initial roles are tenant-scoped:

| Role | Effective M7 capabilities |
|---|---|
| `viewer` | `read:mes` |
| `production_analyst` | `read:mes`, `run:agent`, `read:agent_runs:self` |
| `supervisor` | analyst capabilities plus `read:agent_runs:tenant` |
| `admin` | supervisor capabilities plus `manage:identity` |

Roles are convenience bundles. Enforcement MUST check capabilities rather than
role-name string comparisons. If a user has multiple active roles in one
tenant, effective capabilities are the union of only those locally assigned
roles.

`read:evaluation_datasets` is reserved for the evaluation runner and MUST NOT
be included in an ordinary human role.

Future capabilities such as `approve:workflow_change` or
`approve:exception` require a later contract and do not exist merely because a
user is currently a supervisor or admin. All Agent tools remain read-only in
M7.

Endpoint policy for M7:

| Operation | Required capability and scope |
|---|---|
| MES read endpoints | `read:mes` plus tenant/factory scope |
| Create Agent run | `run:agent`, `read:mes`, requested factory scope |
| Read own Agent run/tool calls | `read:agent_runs:self`, matching user and tenant |
| Read another user's run/tool calls | `read:agent_runs:tenant`, matching tenant |
| Manage identity records | `manage:identity`; no management endpoint is required in M7 |

Agent tools MUST independently check their required capability and factory
scope. Passing the HTTP boundary is necessary but not sufficient.

## Planned persistence boundary

M7.1 migrations SHOULD model at least:

```text
users
external_identities
tenant_memberships
tenant_membership_roles
membership_factory_scopes
```

Required properties:

- internal user IDs are UUIDs;
- `(issuer, subject)` is unique;
- membership is unique per user and tenant;
- roles are constrained to the frozen role set;
- factory scopes reference a factory in the same tenant;
- active/suspended state is explicit;
- all-factory access, if supported, is explicit rather than inferred from an
  empty list; and
- deleting or suspending identity data cannot orphan historical audit records.

Existing `agent_runs.user_id` text data requires an explicit migration plan.
Historical development runs MUST remain readable, while new authenticated runs
MUST reference the internal user identity without fabricating a mapping.

## Authorization and resource-hiding semantics

The API MUST retain the standard error envelope and request ID.

| Situation | HTTP status | Stable error code |
|---|---:|---|
| Missing or invalid bearer credential | 401 | `authentication_required` |
| Valid identity without required capability/scope | 403 | `authorization_denied` |
| Resource absent or outside an already authorized scope | 404 | `resource_not_found` |
| No trustworthy keys/provider state available | 503 | `authentication_unavailable` |
| Authenticated caller exceeds rate limit | 429 | `rate_limit_exceeded` |

A 401 response MUST include an appropriate `WWW-Authenticate: Bearer` header
and a generic message. A 403 MUST NOT reveal which additional role would grant
access. Resource reads SHOULD use 404 where revealing cross-scope existence
would create an enumeration channel.

Rate-limit keys MUST use the authenticated internal user and tenant, not
unverified claims or caller-controlled headers.

## Audit contract

Successful and denied protected operations MUST be attributable to a request
ID and, when authentication succeeded, the internal user and tenant.

M7 audit records MUST support:

- internal user ID and external identity reference;
- tenant and selected factory scope;
- auth method and issuer identifier;
- role/capability snapshot relevant to the decision;
- action, resource type/ID, outcome, and duration;
- Agent run ID and per-tool authorization decision where applicable; and
- a sanitized denial category.

Audit and application logs MUST NOT contain bearer tokens, token signatures,
provider secrets, raw authorization headers, or complete sensitive payloads.
The OIDC subject SHOULD be referenced through the local external-identity row
rather than repeatedly copied into operational audit tables.

## Mandatory security and regression tests

M7 is not complete until automated tests cover all of the following:

1. valid access token succeeds within its local membership;
2. missing credential returns 401;
3. malformed token returns 401;
4. expired token returns 401;
5. premature token (`nbf`) returns 401;
6. modified signature returns 401;
7. wrong issuer returns 401;
8. wrong audience returns 401;
9. disallowed algorithm and `alg=none` return 401;
10. missing or unknown `kid` fails closed with bounded refresh behavior;
11. unavailable JWKS with no valid cache returns 503;
12. valid token without a local identity mapping returns 403;
13. suspended user or membership returns 403;
14. insufficient capability returns 403;
15. cross-tenant scope selection is denied;
16. unauthorized factory selection is denied;
17. empty factory scope grants no factory access;
18. user can read their own Agent runs but not another user's run without the
    tenant-level capability;
19. an HTTP-authorized Agent run cannot expand its scope in tool arguments;
20. non-development startup with `dev_headers` is rejected;
21. OIDC mode does not fall back to `x-dev-*` headers;
22. key rotation accepts a newly trusted key and rejects an invalid old key;
23. logs, errors, and audit rows do not contain the raw token;
24. existing tenant/factory repository isolation tests remain PASS;
25. Track A remains PASS independently; and
26. Track B remains PASS independently.

Tests MUST use local test keys and a controlled JWKS fixture or test server.
Real IdP credentials and private signing keys MUST NOT be committed.

## Delivery sequence

- **M7.0:** freeze this contract and verify documentation/regression gates.
- **M7.1:** add identity persistence, authentication provider abstraction, OIDC
  verification, and principal construction.
- **M7.2:** enforce capability and self/tenant run policies at HTTP and Agent
  tool boundaries.
- **M7.3:** complete audit fields, adversarial tests, key-rotation tests,
  documentation, and the formal M7 report.

Each sub-milestone MUST preserve the current development workflow and MUST keep
non-development serving fail-closed until its production path is complete.

## Explicit non-goals

M7 does not implement:

- an OAuth login UI or authorization-code flow;
- an identity-provider administration product;
- passwords or password reset;
- MES write tools, PLC control, or autonomous actions;
- SOP compilation, event sourcing, or exception approval;
- CD or production hosting; or
- production certification.

Those features require separate contracts and evaluations. In particular,
future approval capabilities cannot bypass the identity, local membership,
scope, audit, and fail-closed properties frozen here.
