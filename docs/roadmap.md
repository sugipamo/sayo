# Sayo Roadmap

English | [日本語](roadmap.ja.md)

This file tracks work that remains relative to the current implementation. Completed semantics and implementation scope
belong in `language-spec.md`; design rationale belongs in `design.md`.

## Candidates to implement after their semantics are decided

- Visibility and shadowing between Template-local Spaces and World-root Spaces
- Generalizing comparison to user-defined Spaces
- Composite Space conditions such as `And`, `Or`, and `Not`
- Expanding the validation primitives available to schemas
- Residual, application, and termination rules if internal `Pass{}` becomes a public primitive
- Configuring or disabling bootstrap resource limits from the CLI

## Revisit after the Runtime is stable

- REPL
- Modules and packages
- A concrete self-hosting migration
- StateInstance garbage collection, persistence, and sharing between Runtime instances
- Publishing `sayo-lang` on crates.io if a stable Rust embedding API or meaningful `cargo install` demand emerges

Features intentionally excluded and completed functionality are recorded only in `language-spec.md`.
