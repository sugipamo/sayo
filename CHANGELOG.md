# Changelog

User-visible changes to Sayo are recorded here.

## Unreleased

### Added

- Typed opaque references using `Role<blob:ID>` and `Role<handle:ID>`
- Host inspection of semantic payloads and pure reevaluation of retained Terms with fixed responses
- A deterministic Host roundtrip example, with no Agent integration or automatic external dispatch
- A Playground example that builds a Request from opaque Agent and Document references

### Changed

- Angle brackets now delimit external references instead of being ordinary Atom characters
- Rust `Term` contains external metadata and an internal ownership marker; use `Term::atom` rather than struct literals

### Fixed

- Playground event tracking scrolls only the Event log, preserving the page position during loading and playback

## 0.1.0 - 2026-08-06

### Added

- Rust-based SayoVM and embedded standard World
- Sayo Relations, Space Checkers, Templates, and structural capture
- SRI JSON and SVI JSON contracts
- WebAssembly Playground

### Changed

- Named the standard sequential-application states `Each`, `EachApply`, and `EachEmit`
- Made GitHub and the Playground the initial distribution channels; crates.io publication remains disabled
