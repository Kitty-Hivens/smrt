# smrt documentation

The map. Each page stands alone; together they cover the mirror end to end.

| Page | What it covers |
|---|---|
| [architecture.md](architecture.md) | What the system is: components, storage tree, data flows. Start here. |
| [concepts.md](concepts.md) | The domain model: packs, versions and channels, sources, the mod registry, side/presence classification, the dependency graph. |
| [api.md](api.md) | The public HTTP API as a client author needs it: the launcher update flow, ordering and channel rules, download resolution, schema versioning. Normative. |
| [operations.md](operations.md) | Running and curating: environment, deploy, the authoring workflow, harvest, registry curation, takedowns, failure modes. |
| [development.md](development.md) | Working on the code: gates, TS bindings, the corpus harness, migrations, adding endpoints. |
| [side-required-audit.md](side-required-audit.md) | Historical: the 2026-07 audit that produced the side/required/presence model, with the acceptance record. Background reading, not a contract. |

Two generated references complement these pages:

- `/docs` on a running mirror -- the Scalar UI over `/openapi.json`, generated from the handler annotations. The endpoint-by-endpoint truth: paths, parameters, response schemas.
- `web/src/lib/bindings/` -- TypeScript types generated from the Rust wire structs (`cargo test` regenerates them). What the panel (and any TS client) compiles against.

The rule of thumb: **prose here, shapes there.** These pages explain semantics the schema cannot carry (ordering rules, invariants, workflows); the OpenAPI reference carries the exact field lists so they never drift from the code.
