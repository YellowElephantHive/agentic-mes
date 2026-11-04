# Deterministic simulator scenarios

Canonical command:

```bash
factory-simulator seed --seed 42 --days 30 --factory demo
```

The fixed UTC window is 2026-01-01 through 2026-01-30. IDs are UUID v5 values derived from seed, factory slug, and record identity.

Expected cardinalities:

| Entity | Count |
|---|---:|
| Tenants | 1 |
| Factories | 1 |
| Lines | 3 |
| Machines | 6 |
| Products | 8 |
| Work orders | 120 |
| Production records | 180 |
| Quality inspections | 90 |
| Downtime events | 2 |

Ground-truth scenarios:

| Scenario ID | Ground truth |
|---|---|
| `SCN-LINE-A-MACHINE-FAILURE` | Line A machine 1, 2026-01-30 08:00–14:00 UTC, 6 hours |
| `SCN-LINE-A-MATERIAL-SHORTAGE` | Line A machine 2, 2026-01-30 10:00–12:00 UTC, 2 hours |
| `SCN-QUALITY-SPIKE-LINE-B` | Line B, 2026-01-15, 20 defects in a sample of 100 |

Line A good output is exactly 1,000 units on 2026-01-29 and 600 units on 2026-01-30. The two downtime intervals overlap; future line-level downtime aggregation must union intervals rather than sum eight machine-hours as eight line-hours.
