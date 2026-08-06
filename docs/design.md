# Sayo Design Notes

English | [日本語](design.ja.md)

## Pure Host boundary

Sayo's pure core accepts only `InitialTerms × WorldSnapshot`, constructed by the Host at startup, and returns only the
residual `NormalForm<Term>` of the initial evaluation Group as a normal result. CLI rendering, SRI, and diagnostics belong
to the Host/VM observation and operations layer; they are not Sayo I/O effects. `Trap` and divergence are not output Terms.

`WorldSnapshot` does not require a physical copy. The Host must not mutate the World or referenced Sayo files while a
Runtime is alive. Concurrent mutation is outside the semantics and is treated as interference with the evaluation input.

## Bootstrapping and self-hosting Relation implementations

```text
SayoVM
    ├── Native Primitive / Native Space Checker
    └── Sayo Relation
```

Foundational Relations such as integer arithmetic live in the Rust Runtime's Native Registry. If every foundational
Relation had to be implemented in Sayo, no implementation would exist to execute the first computation, creating a
circular bootstrap problem.

```text
Ordinary Relation: locate and evaluate main.sayo in a World
Native Relation:   execute a Rust implementation inside the Runtime
```

A Native Relation is a primitive Relation implementation executed directly by SayoVM. Its meaning is determined by its
Spaces, ResultSpace, and ordinary evaluation rules; Sayo and Native are two implementation paths for that meaning.

Space and Template are conceptually separate. A Space is an externally visible classification and connection point for
Terms. A Template defines Term structure and state-transition rules. A Template may expose a public Space, and a
StateInstance participates in Relations through that Space. During bootstrap, Template and public Space names are the
same, but this is an interface mapping rather than conceptual identity.

Standard primitives are not declared in user Worlds. `main.native` is not a declaration mechanism and is rejected as a
World configuration error. The Native Registry is fixed inside SayoVM; users cannot add Native implementations.

Native Terms are addressed by `NativeTermId` and stored as typed `enum NativeTermState` values. The ID/Store boundary
keeps this representation separate from public semantics.

### Template schemas

A Template's `template.sayo` is a restricted Sayo program that validates its payload. Successful validation passes;
`Trap`, `NoStep`, and step-limit exhaustion fail. Ordinary Native Relations, external files, external input, and side
effects are prohibited. SayoVM exposes only the schema-specific `CheckSpace` and `Pass` primitives in this context.
Templates without schemas rely on statically derived payload arity and Runtime transition validation.

### Selecting Native Relations

SayoVM owns standard Native Relations, and user Worlds cannot add them. An applicable Sayo or structural-capture World
Relation takes precedence. Native is an observationally equivalent fallback used only when no World Relation exists.
Native registration APIs remain internal to Rust.

The first standard Native Relations implement integer arithmetic:

```text
Add Int : Plus       Plus Int : Int
Sub Int : Minus      Minus Int : Int
Mul Int : Times      Times Int : Int
Div Int : Quotient   Quotient Int : Int
```

Bootstrap Native `Int` is `i64`, and results are registered as ordinary Int Terms. Each first-stage Relation creates an
intermediate Native Term that captures a TermId; the intermediate receives the next Int and performs the operation.
Overflow and division by zero are Runtime errors.

State that retains input and accepts later application is represented by structural capture and StateInstance in a Sayo
World, and by the Native Term Store in Native code. Sayo Relations do not gain dedicated lambda or closure syntax. Both
paths obey the same Relation, payload, and leftmost-first Scheduler semantics.

Integer representation, arithmetic, and comparison live in Native code. Scheduler, Tick, End, Vanish, and Trap belong to
the Runtime. Ordinary state machines belong in a Sayo World. Whether a new primitive belongs in Native or Sayo is decided
with that primitive's semantics.

Sayo and Native execution are separated internally. Native Executor details are not a World-visible API. Native
optimizations must preserve observational equivalence with their Sayo implementations.

Semantic recursion in Sayo Relations is lowered into explicit evaluation frames owned by SayoVM, not Rust function
recursion. A frame stores Reducer cells, call context, return positions for Sayo Relations or Template-control payloads,
and step accounting. This separates Flat-sequence width from Relation depth and makes the bootstrap depth limit
independent of the Host thread stack.

### State management

StateInstances and Native Terms live in the in-process Term Store rather than generated source files. Persistence, garbage
collection, expiration, and sharing between Runtime instances are not currently provided.

### Test Worlds

`examples/` contains small Worlds for manual use. Automated tests create temporary Worlds in Rust tests. Tests cover Space
classification, Sayo and Native Relations, state Terms, structural capture, source references, ambiguity, World
validation, and arithmetic failures.
