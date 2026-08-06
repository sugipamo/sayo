# Getting Started with Sayo

English | [日本語](getting-started.ja.md)

This guide covers building SayoVM, running programs, creating a minimal World, using the browser Playground locally, and
running the test suites. For exact language semantics, see the [Language Specification](language-spec.md).

## Requirements

Sayo supports the latest stable Rust toolchain and does not declare a fixed MSRV.

## Build and run

Build SayoVM and evaluate an expression against the arithmetic example World:

```sh
cargo build
cargo run -- --world examples/native_arithmetic/world add 5 3
```

```text
8
```

Validate and execute the identity example:

```sh
cargo run -- check --world examples/identity_relation/world
cargo run -- --world examples/identity_relation/world examples/identity_relation/main.sayo
```

```text
42
```

Once the GitHub repository is public, install the `sayo` command directly from it:

```sh
cargo install --git https://github.com/sugipamo/sayo
```

The Cargo package is named `sayo-lang`, but the installed command is `sayo`. Sayo 0.1.0 is not published to crates.io.

## Source files

Sayo evaluates a complete `.sayo` file as one expression:

```sh
sayo --world examples/native_arithmetic/world examples/native_arithmetic/main.sayo
```

Newlines currently behave like whitespace. An Atom ending in `.sayo` references another source file relative to the file
that contains the reference. Cyclic source references are errors.

## Diagnostics and traces

Show each result Term's Space and implementation kind:

```sh
sayo --world examples/native_arithmetic/world --show-space --show-kind add 5 3
```

Write the versioned internal Sayo Reduction IR (SRI) to stderr while preserving NormalForm on stdout:

```sh
sayo --world examples/native_arithmetic/world --trace-format json add 5 3 2> trace.json
```

The internal format is documented in [SRI JSON](sri-json.md). The public Visualizer frame contract is documented in
[SVI JSON](svi-json.md).

## World validation

Validate a World before execution:

```sh
sayo check --world examples/native_arithmetic/world
```

`check` validates World structure, Sayo syntax, Component shadowing, static payload arity, and composition with the
embedded standard World. It does not require self-hosting. Reported counts cover declarations owned by the user World.

## Minimal World layout

```text
world/
├── <Space>/
│   └── space.sayo
├── <Left>/<Right>/<Result>/
│   └── main.sayo
└── <Template>/template.sayo
```

- `space.sayo` is a Sayo Space checker.
- `main.sayo` is a Relation body evaluated by SayoVM.
- `template.sayo` is an optional restricted payload schema.
- An empty Result directory defines structural capture.
- `.tick`, `.end`, and `.any` are reserved Template control slots.

Native Space checkers and primitive Relations live in SayoVM. Standard state machines live in the embedded standard
World, so an empty user World overlay is valid. User Components completely shadow standard Components with the same
name; lookup does not fall back to individual files inside a hidden Component.

Inside Tick and End control bodies, `%0`, `%1`, and later positions refer to the current immutable payload. Each top-level
expression defines one destination payload slot; use a Group for compound application, as in `%0 (%0 %1)`.

See the [Language Specification](language-spec.md) for Result anchors, emission of unreferenced payload, Group expansion,
Scheduler order, and NormalForm rules. See [Design Notes](design.md) for the Host boundary and Native execution rationale.

## Standard operations

The embedded standard World and Native fallback currently support arithmetic, comparison, duplication, open
transformers, and collectors.

```sh
sayo --world examples/native_arithmetic/world duplicate 5 6
# 5 5 6 6 duplicate

sayo --world examples/native_arithmetic/world for '(add 9)' 1 2 3
# 10 11 12 Each{Plus{9}}

sayo --world examples/native_arithmetic/world filter odd 1 2 3 4
# 1 3 Sieve{odd}

sayo --world examples/native_arithmetic/world fold add 0 1 2 3
# 6

sayo --world examples/native_arithmetic/world reduce add 1 2 3
# 6

sayo --world examples/native_arithmetic/world compare 5 9
# Less
```

Bootstrap `Int` values are `i64`; overflow and division by zero Trap. Comparison produces `Less`, `Equal`, or `Greater`
in the `Ordering` Space rather than a Boolean.

## Browser and WebAssembly

The Playground runs Wasm SayoVM in a Web Worker. It snapshots the user World at the start of each run and receives one
versioned JSON project document. Semantic empty directories are listed separately from files.

Building Wasm requires the `wasm32-unknown-unknown` target and the `wasm-bindgen-cli` version used by the repository:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
./scripts/build-wasm.sh
```

Generated bindings are written to the ignored `playground/pkg/` directory. Serve `playground/index.html` over HTTP to
run source, animate SVI frames, edit an in-memory user World, import or export a project, and cancel execution.

Shared URLs contain only the gzip-compressed input World in their fragment. Results and SVI are excluded, and URLs longer
than 8,000 characters are rejected in favor of JSON import and export.

Cloudflare Pages is the planned public host. Its reproducible build and later publication procedure are documented in
[Playground Deployment](deployment.md).

## Testing

Run Rust checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Run the browser suite in Chromium:

```sh
npm ci
npm run test:browser
```

The browser command builds Wasm and exercises SayoVM, the Web Worker boundary, project sharing, traps, and Visualizer
playback. Automated Rust tests create temporary Worlds; manual examples live under `examples/`.
