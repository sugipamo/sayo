# Sayo

English | [日本語](README.ja.md)

Sayo is an experimental language that makes computation visible as the application and transition of Terms.

```text
Add 5 3
→ Plus{5} 3
→ 8
```

There are no functions, methods, or callable objects in Sayo's core model. An ordered sequence of Terms is the current
state, Relations are transition rules, and SayoVM repeatedly applies the leftmost available transition. Evaluation ends
at a **NormalForm**: an equilibrium where no transition remains applicable.

## Why Sayo?

Ordinary programs are also dynamical systems: code supplies rules and internal state evolves. Sayo brings that process to
the surface.

- **Transitions are the language.** Computation is expressed by Terms changing through Relations.
- **State is explicit.** A state Term such as `Plus{5}` retains Terms without introducing closures or objects.
- **Results are residual structure.** A result is the ordered Term sequence left at equilibrium, not a hidden return
  channel.
- **The core stays pure.** The Host supplies initial Terms and a World snapshot, then observes the resulting NormalForm.
- **Execution can be watched.** SayoVM emits structured visualization frames, allowing reduction to be animated without
  reconstructing semantics in the browser.

SayoVM is written in Rust. Foundational operations may use Native implementations, while ordinary state machines live in
an embedded Sayo World. Both follow the same observable semantics.

## A small example

`For` receives a transform and applies it to each Term that follows:

```text
For (Add 9) 1 2 3
→ Each{Plus{9}} 1 2 3
→ 10 Each{Plus{9}} 2 3
→ 10 11 Each{Plus{9}} 3
→ 10 11 12 Each{Plus{9}}
```

The remaining `Each{Plus{9}}` is not discarded. It is an open continuation that can accept more Terms if the surrounding
sequence later supplies them.

## Current status

Sayo is at version **0.1.0**. The current implementation includes:

- a Rust CLI and embeddable SayoVM;
- `.sayo` source files and directory-backed Worlds;
- Spaces, Relations, Templates, structural capture, Tick, End, and state payloads;
- Native arithmetic and comparison;
- standard Sayo state machines for duplication, transformation, filtering, folding, and reduction;
- World validation and versioned reduction/visualization IR;
- a WebAssembly Playground with animated evaluation.

The language is intentionally small and still experimental. REPL, modules, packages, and a concrete self-hosting path are
deferred. Sayo 0.1.0 is distributed through GitHub and the Playground rather than crates.io.

## Try it

Try Sayo immediately in the [WebAssembly Playground](https://sayo-lang.pages.dev/), where each reduction is animated.

Prebuilt `sayo` CLI archives for Linux, macOS, and Windows are available from
[GitHub Releases](https://github.com/sugipamo/sayo/releases). Each release includes `SHA256SUMS` for verification.

Build and run from this repository with the latest stable Rust toolchain:

```sh
cargo run -- --world examples/native_arithmetic/world add 5 3
```

```text
8
```

After the repository becomes public, the command can also be installed directly from GitHub:

```sh
cargo install --git https://github.com/sugipamo/sayo
```

## Documentation

- [Getting Started](docs/getting-started.md) — build, CLI, Worlds, WebAssembly, and tests
- [Language Specification](docs/language-spec.md) — canonical language semantics
- [Design Notes](docs/design.md) — Host boundary, Native execution, and state design
- [Sayo Reduction IR](docs/sri-json.md) — internal Reducer event contract
- [Sayo Visualization IR](docs/svi-json.md) — complete Visualizer frame contract
- [Roadmap](docs/roadmap.md) — deliberately deferred work

The documentation index is available in [`docs/README.md`](docs/README.md).

## Contributing

Issues, semantic proposals, implementations, and documentation improvements are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) and follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Sayo is distributed under the [MIT License](LICENSE).
