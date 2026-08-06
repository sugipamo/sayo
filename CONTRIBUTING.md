# Contributing to Sayo

Issues, design proposals, implementations, and documentation improvements are welcome.

## Before you start

For changes that affect language semantics, please open an Issue before implementation and describe the motivation and
example state transitions. The current contract is in [`docs/language-spec.md`](docs/language-spec.md), and deferred work
is tracked in [`docs/roadmap.md`](docs/roadmap.md).

Small bug fixes, tests, and typo corrections may be submitted directly as Pull Requests.

## Local checks

Use the latest stable Rust toolchain. Sayo does not currently declare a fixed MSRV.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

If you change the Playground, prepare the Wasm toolchain described in the README and also run:

```sh
npm ci
npm run test:browser
```

## Pull Requests

- Keep each Pull Request focused on one purpose where practical.
- Update the specification and tests in the same Pull Request when public semantics change.
- Test observational equivalence with the Sayo implementation when adding a Native optimization.
- Do not commit generated `target/`, `playground/pkg/`, or `node_modules/` content.

Contributions are provided under the same MIT License as the project.
