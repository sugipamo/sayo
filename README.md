# Sayo

English | [日本語](README.ja.md)

Repository: <https://github.com/sugipamo/sayo>

Sayo is a language built around the application of terms. The Rust-based SayoVM owns evaluation,
runtime state, and native primitives, while standard state machines run as an embedded Sayo World.
Programs execute through Sayo Relations and SayoVM-native Relations.

Sayo can also be understood as a discrete dynamical system: an ordered sequence of Terms is its
state, Relations are transition rules, and SayoVM is the transition engine. A NormalForm is not
merely a return value; it is an equilibrium in which no transition remains applicable.

The language touches the outside world at two boundaries. The Host supplies the initial Term
sequence and an immutable World snapshot, then observes the ordered Terms left in the initial
evaluation Group once it reaches equilibrium. Sayo has no `Print` operation; CLI output is a Host
observation. Mutating a World or referenced Sayo source while its Runtime is alive is unsupported.

Term Templates, `.tick`, `.end`, `.any`, and the reserved `Trap` Template are part of the current
language model. SayoVM implements StateTerms, payloads, zero-based payload references such as `%0`,
structural capture of right-hand Terms, and Template control Relations.

Users may construct state explicitly with `Template { payload, ... }`. Commas are required between
payload Terms because whitespace denotes ordinary Term application. `Template {}` is a valid
zero-payload StateInstance. Evaluating the same constructor twice creates two distinct instances;
it does not restore an existing identity.

The complete language specification, design notes, IR contracts, and roadmap live in
[`docs/`](docs/README.md).

## Build and run

Sayo currently supports the latest stable Rust toolchain and does not declare a fixed MSRV.

```sh
cargo build
cargo run -- --world examples/native_arithmetic/world add 5 3
```

Output:

```text
8
```

A minimal Relation implemented in SayoVM can be checked and executed as follows:

```sh
sayo check --world examples/identity_relation/world
sayo --world examples/identity_relation/world examples/identity_relation/main.sayo
```

Output:

```text
42
```

Sayo can evaluate an entire source file as one expression:

```sh
sayo --world examples/native_arithmetic/world examples/native_arithmetic/main.sayo
```

Enable diagnostic output to inspect each result Term's Space and implementation kind:

```sh
sayo --world examples/native_arithmetic/world --show-space --show-kind add 5 3
```

Use the versioned internal SRI JSON trace when investigating Reducer behavior:

```sh
sayo --world examples/native_arithmetic/world --trace-format json add 5 3 2> trace.json
```

The internal trace contract is documented in [`docs/sri-json.md`](docs/sri-json.md). The public
Visualizer contract is the complete frame sequence described by
[`docs/svi-json.md`](docs/svi-json.md). The Visualizer does not reconstruct Reducer state or infer
structure by parsing Sayo source.

The NormalForm is written to stdout and internal SRI JSON to stderr, so tracing does not break
pipelines that consume the result.

Newlines currently behave like whitespace and have no separate meaning. An Atom ending in `.sayo`
references another Sayo source file and is resolved relative to the referring file. Cyclic source
references are errors.

Validate a World before execution with:

```sh
sayo check --world examples/native_arithmetic/world
```

`check` does not require self-hosting. It validates Space and Relation structure, Sayo syntax,
Component shadowing, static payload arity, and whether the user World can be composed with the
embedded standard World. Reported counts include declarations owned by the user World only.

Install the command directly from the GitHub repository with:

```sh
cargo install --git https://github.com/sugipamo/sayo
sayo --world examples/native_arithmetic/world add 5 3
```

The Cargo package is named `sayo-lang`; the installed command is named `sayo`. Sayo 0.1.0 is not published to
crates.io. The Playground and GitHub repository are the primary distribution channels while the Rust embedding API is
still evolving.

## Browser and WebAssembly

The browser adapter snapshots the user World into a `MemoryWorld` at the beginning of each run and
keeps it separate from the embedded standard World. The Wasm boundary accepts one versioned JSON
document. Empty Relation directories are semantically meaningful, so they are listed explicitly in
`directories` rather than inferred from `files`.

```json
{
  "version": 1,
  "entry": "main.sayo",
  "directories": ["Add/Int/Plus"],
  "files": {
    "main.sayo": "add 5 3"
  },
  "options": {
    "visualization": true,
    "relation_depth": 256,
    "max_steps": 100000,
    "max_trace_events": 100000
  }
}
```

Building Wasm requires the `wasm32-unknown-unknown` target and the `wasm-bindgen-cli` version that
matches the Cargo dependency:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
./scripts/build-wasm.sh
```

Generated bindings are written to `playground/pkg/`. `playground/sayo-worker.js` initializes SayoVM
inside a Web Worker and returns a single JSON response containing the NormalForm and complete SVI
frame sequence. Semantic replay and frame construction happen in Rust; the Visualizer renders the
resulting frames and their declared changes.

Cloudflare Pages is the planned public host. The repository contains a reproducible Pages build, while project creation
and the first deployment remain a later publication task. See
[`docs/deployment.md`](docs/deployment.md) for the exact build settings.

## World layout

```text
world/
├── <Space>/
│   └── space.sayo              # Sayo Space checker
├── <Left>/<Right>/<Result>/
│   └── main.sayo               # Relation implementation evaluated by SayoVM
└── <Template>/template.sayo    # Optional payload schema
```

Native Space checkers and primitive Relations live in SayoVM's Native Registry. Standard state
machines live in the embedded Sayo World. An empty user World overlay is therefore valid.

An empty Result directory defines a structural capture Relation. SayoVM appends the right-hand Term
to the payload of a new StateInstance. Inside `.tick` and `.end` implementations, current payload
Terms are available through zero-based references such as `%0` and `%1`.

An empty fixed Result directory under `.tick` or `.end` constructs a zero-payload Result instance.
Current payload Terms not referenced by the next payload are emitted to the left in their original
order. A `Tick` is a self-transition that consumes no external input; depending on the target
Template, it may increase, preserve, or decrease payload arity. SayoVM derives each Template's arity
statically from the World.

The bootstrap Runtime limits semantic Relation depth to 256 and reduction steps to 100,000. Relation
calls are managed with explicit SayoVM evaluation frames rather than the Rust call stack. The width
of a Flat Term sequence does not count toward Relation depth. Rust callers can supply different
limits through `RuntimeLimits`.

A `main.sayo` Relation does not generate source. SayoVM normalizes its body and splices the resulting
Term sequence into the caller. A result may contain zero, one, or many Terms. For example, when no
Relation applies to `5 3`, the sequence `5 3` itself is a valid NormalForm. Parentheses create an
explicit Group that is normalized before being expanded into its parent sequence.

`Trap`, divergence, traces, and CLI rendering are not output Terms. An empty NormalForm is valid.

A Space checker lives at `<Space>/space.sayo` and receives a candidate's source as a String in `%0`.
For example, `equal-to "hello" %0` accepts only the Atom `hello`. A normal single-Term result means
Match, `Miss` means non-membership, and `Trap` means the checker failed. Empty or multi-Term checker
results are implementation errors. Checkers cannot access payload positions other than `%0` or load
external Sayo files.

The rationale for Host boundaries and Native Relations is in
[`docs/design.md`](docs/design.md). Deferred implementation candidates are in
[`docs/roadmap.md`](docs/roadmap.md).

Native primitives are not declared in user Worlds, and `main.native` is rejected by `sayo check`.
The lookup order is user World, embedded standard World, then standard Native fallback. A user
Component completely shadows a standard Component with the same name; lookup never falls back to
individual files inside the hidden standard Component.

Template control `main.sayo` files treat each top-level expression as one next-payload slot. Wrap an
application in a Group when it must produce one slot, as in the second slot of `%0 (%0 %1)`.

The current Native Registry provides these Relations:

```text
Add Int : Plus       Plus Int : Int
Sub Int : Minus      Minus Int : Int
Mul Int : Times      Times Int : Int
Div Int : Quotient   Quotient Int : Int
Compare Int : CompareInt   CompareInt Int : Ordering
Compare String : CompareString   CompareString String : Ordering
Duplicate Int : Duplicate
For * : Each            Each Int : Each
Filter * : Sieve        Sieve * : Sieve
Odd Int : Keep | Drop
Fold * : FoldInit       FoldInit * : FoldAccum
FoldAccum * : FoldAccum
Reduce * : ReduceInit   ReduceInit * : ReduceAccum
ReduceAccum * : ReduceAccum
```

Bootstrap `Int` values are `i64`. Overflow and division by zero produce a Runtime error.

`Compare` produces the immutable `Ordering` Terms `Less`, `Equal`, and `Greater`. These are not
Booleans, and Sayo currently has no dedicated `if` syntax or Boolean evaluation rules.

The embedded standard World implements `Duplicate` as a state machine:

```text
EmitTwice{5} → EmitPair{5, 5} → 5 EmitOnce{5} → 5 5 duplicate
```

`For` applies its transform once to every following Term. `Filter` requires its predicate to produce
one Term in either the `Keep` or `Drop` Space; any other result traps. `Fold` and `Reduce` are
Collectors that close at the end of their evaluation Group.

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

## Testing

```sh
cargo test --all-targets
```

The browser Playground runs Wasm SayoVM in a Web Worker. Serve `playground/index.html` over HTTP to
run source, animate complete SVI frames, edit the user World, and cancel execution. Shared URLs store
only the gzip-compressed input World in the fragment; execution results and SVI are excluded. URLs
longer than 8,000 characters are rejected in favor of JSON project import and export.

```sh
npm run test:browser
```

This command builds Wasm and exercises the Playground/Web Worker boundary in a real headless
browser. Manual examples live under `examples/`; automated tests create temporary Worlds for each
feature. World Relations, including structural capture through empty directories, take precedence
over Native fallback.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). User-visible changes are tracked in
[`CHANGELOG.md`](CHANGELOG.md). By participating, you agree to follow the
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).
