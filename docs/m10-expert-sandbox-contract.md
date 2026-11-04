# M10 Expert Sandbox and Feedback Contract

Status: **FROZEN FOR M10.1–M10.4 IMPLEMENTATION**

## Objective

Turn the M9.9 single-case local console into a recoverable expert-evaluation
sandbox. M10 helps invited manufacturing specialists inspect cases, vary the
deterministic demo scope, and attach clearly labeled human annotations. It
does not make the service a production MES or grant manufacturing execution.

## Frozen boundaries

- Track A and Track B remain mandatory, independent release gates.
- Original manufacturing and dataset records remain immutable and provenance
  preserving; missing values remain missing.
- Work-queue reads are tenant/factory scoped and capability filtered.
- Feedback is an annotation, never manufacturing ground truth and never an
  automatic training example.
- Persona selection is permitted only through the development identity adapter
  and demonstrates workflow, not production separation of duties.
- Non-development serving continues to require OIDC and ignores `x-dev-*`.
- No `/execute`, MES/ERP/QMS/WMS/PLC write, ticket export, or equipment command
  is introduced.

## Delivery

- **M10.0:** freeze this contract.
- **M10.1:** paginated case queue API with status and record-type filters.
- **M10.2:** recoverable Work Queue UI and case deep links.
- **M10.3:** authorized factory/line scope selection plus deterministic scenario
  presets. Sandbox data mutation and drift-trigger controls require a separate
  future contract with reset provenance and production-route absence tests.
- **M10.4:** structured expert feedback with server-rebuilt trace metadata,
  explicit research consent, and no automatic model-training path.

## Acceptance

Every sub-milestone must pass formatting, Clippy, unit and PostgreSQL tests,
the M9 no-execution/adversarial suite, the browser workflow check, Track A,
Track B, and the aggregate gate before it is recorded as passing.
