# Sayo Language Specification

English | [日本語](language-spec.ja.md)

This document consolidates the Sayo language semantics and Runtime policy decided so far.

Status labels have the following meanings:

- **Implemented**: works in the current `SayoVM`.
- **Deferred**: intentionally left undecided.

## 1. Overview

Sayo is a language centered on the application of Terms.

It is also a **discrete dynamical-system language**: an ordered Term sequence is state, Relations and Runtime-owned states
are transition rules, and SayoVM is the transition engine. Program meaning includes not only Relations but also the
leftmost-first Scheduler, which restarts its scan from the beginning after every transition.

| Dynamical-system role | Sayo construct |
|---|---|
| State | Ordered Term sequence and Group control frames |
| Transition rules | Relations and Runtime-owned state transitions |
| Time-evolution law | Leftmost-first Scheduler |
| Boundary condition | Internal `End`, attempted only at the end of a Group |
| Observation | NormalForm, CLI rendering, and SRI trace |

A NormalForm is not necessarily one value. It is an equilibrium in which no `Tick`, binary transition, or applicable
Group-end `End` transition remains. Infinite transition is not a language error; it is a non-converging trajectory. The
bootstrap SayoVM resource limits are operational guards that stop such trajectories in the initial implementation.

```text
M N
```

means “apply Term `N` to Term `M`.”

Sayo does not expose functions, methods, or Callable as surface-language concepts. Higher-order behavior is expressed by
a Relation producing a Term that can accept a later application.

```text
Add 5
```

may produce a Term that retains `5` and can receive another Term later.

Sayo follows four core principles:

1. Everything is a Term.
2. Semantic data interpreted by the core is String.
3. Runtime owns state.
4. Everything evolves by transitions.

Here, String means data observed and exchanged by Sayo programs. `TermId`, Native state, Group, and `End` are internal
control information used by SayoVM to manage transitions, not user-semantic values.
Opaque external references can be transported as whole Terms; their referenced contents are interpreted only by the Host.

## 2. Term

### 2.1 Basic syntax and String Terms — Implemented

```text
Term ::= Atom
       | StringLiteral
       | ExternalTerm
       | Term Term
       | (Term)

StringLiteral ::= `"` UTF-8 string `"`
```

Application is left-associative:

```text
a b c
```

is equivalent to:

```text
(a b) c
```

Whitespace separates Terms. Newlines currently behave exactly like other whitespace and have no independent meaning.

A quoted string is one immutable Term in the `String` Space. Strings are UTF-8 and recognize `\"`, `\\`, `\n`, `\r`,
and `\t`. Unknown escapes and literal newlines inside a string are syntax errors. Unquoted Atoms and internal source
strings are not language-level `String` values.

An Atom ending in `.sayo` is a Sayo source-file reference. Relative paths resolve from the referring Sayo file. Cyclic
references are Runtime errors.

### 2.2 Relation input — Bootstrap implemented

Ordinary binary Relations expose neither `input` nor `%input` syntax for referring to the right Term. When SayoVM finds an
applicable right Term, it appends that Term to the current payload and transitions to a new StateInstance of the Result
Template named by the Relation path. This automatic capture transition requires no Sayo implementation file.

```text
State{p0, ..., pn} x
→ Result{p0, ..., pn, x}
```

A source-backed left Term is treated as having a zero-Term payload. Computation after capture is expressed by a `Tick`
Relation on the Result Template.

### 2.3 State Terms — Bootstrap implemented

The canonical state-Term notation pairs a Template name with a payload of zero or more Sayo Terms:

```text
<Template>{<Term> (, <Term>)*}
```

`Template{}` is a zero-payload StateInstance and differs from the Atom `Template`.

Examples:

```sayo
Plus{5}
For{(Add 9)}
```

Inside `{...}`, commas separate payload Terms while whitespace retains ordinary application meaning. Commas are required
between multiple payloads; `Pair{left right}` is invalid. Wrap an application in a Group, as in `For{(Add 9)}`, when it
must occupy one payload position.

A payload is an ordered Term sequence owned by one StateInstance. A nested state Term remains one payload Term from the
outer Template's perspective; it does not create several independent state machines inside one `{}`. The Template
interprets payload count, positional meaning, and required Spaces. Runtime stores payloads as Term ID sequences and has
no semantic knowledge of Template-specific field or type names.

This differs from ordinary application:

```sayo
Plus 5
```

`Plus(5)` already means `Plus 5` under the existing grammar and is therefore not state notation.

Runtime can create state Terms through automatic capture and internal transitions. Users may also write
`Template { payload... }` as the formal state-construction syntax. A payload may be empty. Every evaluation constructs a
new StateInstance; this syntax does not deserialize or restore an existing State ID.

### 2.4 External references — Prototype implemented

```text
ExternalTerm ::= Role `<` Kind `:` ID `>`
Kind         ::= `blob` | `handle`
```

`Role` is an ASCII identifier beginning with a letter or `_`. `ID` is a nonempty sequence of ASCII letters, digits,
`_`, `-`, or `.`. `<>` contains one reference, never a Sayo expression or raw contents. `blob` denotes a Host-owned
immutable value; `handle` denotes a connection to a potentially mutable external target. Neither is a capability.

The role must be a non-primitive user Space declared with `space.sayo`. Explicit construction chooses this Space without
running a source checker. The core does not dereference, compare contents, or change external values. External Terms
have no semantic payload or automatic Tick/End behavior; Relations use their outer Space and can transport them whole.
For example, `Request{Agent<handle:coder>, Document<blob:input>}` has two semantic payload Terms, each with one opaque
reference. `%0` returns the whole Agent Term, not its external ID or contents.

Reference metadata is available to the Rust Host through `Term::external`. IDs are displayed in source and traces.
The Host owns lookup, reference validation, and blob immutability. Schemas and source Space checkers cannot construct
external Terms. Combined `{}`/`<>` syntax and reference-identity operations are deferred. See
[External References](external-boundary.md) for the complete prototype contract and executable mock Host example.
The optional [native Host](host-effects.md) executes registered real operations at this boundary.

## 3. Space

A Space is a set to which a Term belongs. It is not a static type. Membership is decided dynamically by a Runtime Space
checker or a known Native rule, and one Term may belong to more than one Space.

```text
Int
Add
Plus
```

A directory at the World root represents a Space and may contain a membership checker.

Space and Template are conceptually separate. A Space is an externally visible Term classification and connection point;
a Template defines Term structure and state transitions. A Template may provide zero or more public Spaces, and a
StateInstance participates in Relations through those public Spaces. A Template with no public Space exists only as an
internal state.

For compatibility with the current World structure, bootstrap convention gives a Template and its public Space the same
name. This means “the Template implements this Space,” not that the two concepts are identical. Existing World Spaces used
by payload positions and future Template-local Spaces may be referenced by Template validation rules.

```text
world/
    Hello/
        space.sayo
```

SayoVM supplies the candidate Term's source string to the checker.

Native Registry provides checkers for standard primitive Spaces, including `i64` classification for `Int` and exact
matches for standard Atoms such as `add` and `fold`.

A user World defines a Sayo Space Checker at `<Space>/space.sayo`. The candidate's raw source becomes a `String` Term in
payload `%0`. If the checker NormalForm contains exactly one Term, it is a Match unless that Term is `Miss`. `Miss` is a
checker-only non-membership outcome and cannot remain as an ordinary Term. `Trap` stops evaluation as a checker failure.
An empty or multi-Term NormalForm is also a checker implementation error.

Checkers are pure, closed classification rules. They may access only `%0`, may not load external `.sayo` files or external
input, and do not recursively classify their Atoms through other user Space Checkers. They resolve only through the
embedded standard World and Native layer. Native Space membership is used when the World finds no membership. If multiple
Spaces Match, classification is ambiguous and traps.

## 4. Relation

A Relation defines application between Spaces:

```text
LeftSpace RightSpace : ResultSpace
```

Examples:

```text
Add Int : Plus
Plus Int : Int
```

The meaning is:

> Applying a Term in RightSpace to a Term in LeftSpace produces a Term in ResultSpace.

A Relation may produce a Term that accepts later application, expressing partial application and higher-order behavior
without introducing functions or Callable:

```text
Add 5 : Plus
Plus{5} 3 : Int
```

## 5. World and Relation storage

### 5.1 World structure — Implemented

```text
world/
├── <Space>/
│   └── space.sayo
├── <LeftSpace>/<RightSpace>/<ResultSpace>/
│   └── (no implementation file for automatic capture)
└── <LeftSpace>/<RightSpace>/<ResultSpace>/main.sayo
```

The following represents `Add Int : Plus`:

```text
world/
    Add/
        Int/
            Plus/
                main.sayo
```

For Relations declared in a World, Space, RightSpace, and ResultSpace must be known in that World. Private Native
Relations supplied by SayoVM are outside this constraint and may use Runtime-internal Result Spaces such as `Each` and
`Sieve`.

Relation Left, Right, and Result names refer to **Term Template names** in the World. A Space is a materializable Template;
an internal state is a capture-backed Template instance created by Runtime. A Relation result must return its declared
Result Template as the rightmost anchor, including for multi-Term replacement.

The reserved Result name `.any` defines a dynamic-Result Relation whose rightmost anchor may be any Template. A fixed
Result and `.any` Result may not coexist for the same Left/Right pair; World validation and Runtime lookup both enforce
this exclusion. When no concrete RightSpace applies, ordinary lookup falls back to `Left/.any/Result`.
`Left/Right/.any/main.sayo` is available only to Sayo implementations, and `.any` has the same meaning for Template-control
Relations.

#### Relation candidate resolution — Implemented

Candidates are selected in this order:

1. Concrete Right with fixed Result
2. Concrete Right with dynamic `.any` Result
3. `.any` Right with fixed Result
4. `.any` Right with dynamic `.any` Result
5. Native wildcard

Multiple candidates at one priority trap as ambiguity. If the chosen candidate returns `NoStep`, lookup does not fall
through to lower priorities. This ordering is applied inside the one World Component selected by Component-level shadowing.

### 5.2 Relation implementations — Implemented

| Kind | Location | Executor |
|---|---|---|
| Sayo | `main.sayo` | SayoVM |
| Native | outside the World | Rust inside SayoVM |

For an ordinary binary Relation, the existence of the Result directory itself defines automatic capture. Empty Result
directories are formal Relations in the bootstrap specification, while leaving room for a future declaration file.
`main.native` is forbidden in Worlds. SayoVM manages standard Native Relations and Native Space Checkers in its Registry;
user Worlds need not declare them.

## 6. Execution boundary

A Relation implementation is a `main.sayo` evaluated inside SayoVM, structural capture represented by an empty directory,
or a SayoVM built-in Native implementation. There is no external-process protocol. An inapplicable Relation becomes
Scheduler `NoStep`; it does not depend on a process exit status.

## 7. Runtime-owned Terms

State Terms and Native Terms live in the in-process Term Store and are not written to files. A Native state lives exactly
as long as its Runtime. Individual garbage collection, expiration, and sharing between Runtime instances are not provided.

### 7.1 State payload — Bootstrap implemented

A state Term distinguishes payload retained by its Template from a following right Term:

```text
Plus{5} 3

payload: [5]
input:   3
```

Native arithmetic Terms and general StateInstances retain captured values as `TermId` values in Runtime memory:

```text
StateInstance {
  state_id: StateId
  template: TemplateId
  payload: Vec<TermId>
}
```

Template position references use zero-based `%0`, `%1`, `%2`, and so on. They refer to internal Template payload, not
variables or functions in an ordinary user expression. Space/type decorations such as `Add{9}[Plus]` are outside Runtime
semantics and may be added by a Visualizer or debugger. Runtime does not require a higher-level type name for a payload.

Sayo Relations currently gain no capture syntax, lambda syntax, or arbitrary source-generation facility. Runtime owns the
general mechanism that creates state Terms.

## 8. Evaluation

### 8.0 Host boundary — Implemented

Sayo semantics touch the outside world at exactly two points:

1. At startup, the Host gives the initial evaluation Group an initial ordered Term sequence and a logically immutable
   World snapshot.
2. The Host observes the ordered Term sequence that remains when the initial Group can no longer transition.

Sayo provides neither `Print` nor an operation that obtains more external input during evaluation. The result is a
`NormalForm<Term>`, not a String, and may contain zero, one, or many Terms. CLI rendering, structured Rust API access, and
Visualizer rendering are different Host observations of the same remaining sequence.

A Host may observe residual Request Terms, process them outside Sayo, and provide fixed response Terms as input to a new
evaluation interval. `Runtime::payload_terms` inspects semantic Template payloads; `Runtime::evaluate_terms` accepts
unmodified Terms from the same Runtime without reparsing display text. The evaluator does not execute Requests,
automatically resume pending work, or expose nested Group NormalForms as external execution boundaries.

`Trap` is not an output Term. It means evaluation did not reach NormalForm. Divergence has no result, while a bootstrap
limit is a diagnostic guard against divergence. SRI is an execution observation, not Sayo output.

```text
evaluate : InitialTerms × WorldSnapshot
        → NormalForm
        | Trap
        | Divergence
```

The same initial Terms and World snapshot reach the same NormalForm or the same Trap. SayoVM does not physically copy a
World. Its Host contract requires World directories and referenced Sayo files to remain unchanged for the Runtime's
lifetime. Concurrent mutation is an unsupported Host race, not a Sayo state transition.

### 8.1 Leftmost-first transitions — Implemented

Runtime scans the parser's Term sequence from the left. When an adjacent pair can transition, it performs only the first
such transition and restarts at the beginning. A sequence with no transition is NormalForm; NormalForm need not contain
exactly one Term.

```text
add 5 3
→ Plus{5} 3
→ 8
```

`NoStep` is not an error; scanning continues to the right. Ambiguous Relations, Space mismatches in declared results, and
implementation failures Trap. Bootstrap defaults limit Relation depth to 256 and transitions to 100,000. Reaching a
limit is a bootstrap diagnostic, not a rule that self-transitions are invalid. A visible `A → A` transition is valid and
may diverge in an unbounded Runtime.

### 8.2 Groups and sequence expansion — Implemented

`(...)` is an evaluation Group, not only parse precedence. Runtime normalizes its contents and expands the resulting Term
sequence into the parent sequence.

1. Normalize and expand a nested Group first.
2. Try a right-less `Tick` transition on the current Term.
3. If Tick returns `NoStep`, try the current Term against its right neighbor.
4. On success, replace the target with one or more Terms and restart from the left.
5. On `NoStep`, move right.
6. When the whole sequence has no step, try `End` on its final Term.
7. If End also returns `NoStep`, return the complete sequence as NormalForm.

A Relation may produce multiple Terms. For a fixed Result, the rightmost produced Term—the anchor—must belong to the
declared Result Space. Runtime-owned structural capture and standard-World control Relations create internal states.

```text
Duplicate 5
→ EmitTwice{5}
→ EmitPair{5, 5}
→ 5 EmitOnce{5}
→ 5 5 duplicate
```

The internal names above are Runtime display labels, not user Atoms or Spaces. Runtime `display` is distinct from
executable `source`.

### 8.3 Relation outcomes — Implemented

| Outcome | Meaning | Reducer action |
|---|---|---|
| `Success` | transition to one or more Terms | restart from the left |
| `NoStep` | the current candidate cannot act | continue right |
| `Trap` | implementation or semantic failure | stop immediately |

A `Success` whose display is unchanged remains a real transition. Only bootstrap limits stop it. There is no `Pending`
outcome. Division by zero, integer overflow, invalid state transitions, cyclic file references, and depth exhaustion are
examples of Trap.

### 8.4 Scheduler contract — Implemented

The Scheduler is part of Sayo semantics:

1. Scan every evaluation Group from the left.
2. Normalize a nested Group before expanding it into its parent.
3. At each cell, try Tick before binary action.
4. If Tick yields `NoStep`, try the cell and its right neighbor.
5. After any success, restart at the beginning of the replaced sequence.
6. If every Tick and binary action yields `NoStep`, try End only on the final Term.
7. If End yields `NoStep`, the sequence is NormalForm.

Semantic Relation recursion is allowed. SayoVM represents Relation calls, Template payload evaluation, and return
positions as heap evaluation frames rather than Rust call-stack depth. A wide flat sequence is not Relation depth. A
state produced by a Relation carries the increased depth into later Tick and End evaluations.

Consequently, if `A Tick → A` diverges in `A B`, then `A` never consumes `B`. Sayo does not provide fair parallel
evaluation or right-first evaluation. An optimization may shorten a valid transition chain but must not move a rightward
transition earlier or reorder Terms emitted to the left. SRI may record the optimized trajectory.

Relations may be re-evaluated. Every Relation must be pure: the same input gives the same result and changes no external
state. Native optimization and any future parallelism rely on this contract. Bootstrap limits are not semantic halting
rules.

### 8.5 Tick Relations — Implemented

A right-less Relation is an internal `Tick` event on one Term:

```text
Left : Result
State Tick → Result
```

Tick consumes no right Term and cannot be written as a user Term or Space. Runtime tries it before binary action. A
successful Tick replaces the current cell with zero or more Terms and restarts the Scheduler from the left.

Tick is self-change without external input. Payload arity may increase, stay equal, or decrease; for example,
`One{%0} → Two{%0, %0}` is valid. Every ordinary Template nevertheless has one statically determined arity, and each
control implementation must produce exactly the destination Template's arity. A zero-payload fixed destination is an
empty Result directory with no `main.sayo`.

Tick differs from End: Tick is tried at every position, while End is a boundary event tried only on the final Term after
ordinary transitions are exhausted. Standard-World transitions such as `EachApply → EachEmit` and
`FoldApply → FoldNext` are Tick Relations.

```text
world/
  Left/
    .tick/
      Result/
        main.sayo
```

`.tick` is a reserved right slot. Likewise, `.end` defines a Group-end transition. More than one applicable Result at the
same control slot is ambiguous and Traps. A fixed Result directory without `main.sayo`, such as
`Before/.tick/After/`, creates `After{}`; all unreferenced old payload is emitted to its left. This empty form is invalid
for `Trap` and dynamic `.any`, while reserved `Vanish` retains its disappearance semantics.

Users may directly construct a declared state Template. Runtime assigns a fresh State ID and retains payload by Term ID.
Equal-looking instances are still distinct. State GC, persistence, and source serialization are absent in the bootstrap
VM. Only Templates in the effective user or standard World can be constructed. `Trap` and `Vanish` are reserved and
cannot be constructed directly. Internal standard Templates are currently visible because modules do not yet exist, but
depending on them is unstable. A standard internal Template removed by whole-Component shadowing is not constructible.

State transitions may produce an empty sequence. Reserved `Vanish{payload...}` transitions on its next Tick to the empty
sequence. Empty source Groups are invalid, but empty intermediate and final NormalForms are valid.

### 8.6 Term Templates and automatic capture — Implemented

A general state Term is a Runtime-owned `StateInstance`:

```text
StateInstance {
  state_id: StateId
  template: TemplateId
  payload: Vec<TermId>
}
```

A Template defines structure, right-Term capture, Tick, and End transitions. It may expose a public Space through which
its instances participate in Relations. A StateInstance is an immutable runtime individual, distinct from the Template
and from a Space checker.

Structural binary capture must transition to a Template whose arity is exactly one greater. Tick and End consume no
external input and may change arity freely. Runtime derives every Template arity uniquely from zero-payload roots,
structural capture, and control Relation bodies. Conflicting derivations, unreachable Templates, and positive capture
cycles are World validation errors.

#### Payload validation

The bootstrap schema primitive is limited to `CheckSpace`. Composite predicates and user-defined validation primitives
remain deferred. `Pass{}` is currently a schema-only temporary state, not a normal user value.

If a Template has `template.sayo`, Runtime evaluates it in a restricted schema context. Success passes; Trap, NoStep, or
the schema limit fails. Schema evaluation permits only internal `CheckSpace` and `Pass`, and forbids normal Native
Relations, external files, external input, and effects. Without a schema, Runtime applies known bootstrap rules and
checks otherwise unknown payload at transition time. Each payload expression must normalize to exactly one Term.

Within a schema, a World-root Space name such as `Int` is a Space-name literal used by
`CheckSpace{%0, Int}`. It is not an ordinary Atom value. A successful CheckSpace Tick becomes internal `Pass{}`; failure
Traps.

#### Capture and control bodies

A structural binary Relation has no Sayo body. Runtime appends the right Term, creates a fresh StateInstance, and uses the
Result path to choose its Template. The Result arity must be exactly the current arity plus one.

Tick and End bodies construct only destination payload. Their scope contains payload projections `%0`, `%1`, and so on;
there is no input binding because Runtime already performed capture, and End is not a Term. Old payload not referenced by
the destination expressions is emitted, in original order, to the destination state's left. A projection used inside a
nested expression or referenced Sayo file counts as used; payload is never silently discarded.

Each top-level expression in a control `main.sayo` is a separate destination payload slot. Slots do not act on one
another. Parentheses put a compound action into one slot:

```text
%0 (%0 %1)
```

This retains old `%0` in slot zero and places the result of `%0 %1` in slot one. Ordinary user programs and ordinary
Relation bodies remain one left-associative sequence.

`%n` is a zero-based projection from immutable payload, not a variable, method, or object field. Negative,
non-integer, and out-of-range projections Trap and are rejected statically where arity is known. World validation checks
projections in normal Relations, Tick, End, schemas, and referenced Sayo files. Schemas may not reference external Sayo
files.

Reserved `Vanish` accepts any payload arity. A transition to fixed Result `Vanish` explicitly references payload that
must disappear; referenced values move into Vanish and disappear on its next Tick, while unreferenced values are emitted.
An empty Vanish Result emits every old payload and removes only the state shell.

```text
DropOne{discarded, kept}
→ kept Vanish{discarded}
→ kept

SumEmit{6}
→ 6 Vanish{}
→ 6
```

Native state machines are optimized executions of the same protocol. `Template{payload...}` is formal state-construction
syntax and always creates a fresh StateInstance.

Every World-root directory shares one namespace for Templates and Relation Left/Right/Result names. `template.sayo` is a
restricted payload schema, not a Relation body or field declaration. A Component may also contain `space.sayo`; only
such source-backed Spaces can materialize a source String as a Term. An internal Template without `space.sayo` can arise
only through capture, a control transition, or explicit state construction.

```text
world/
  Each/
    template.sayo
    .any/
      EachApply/
  EachApply/
    template.sayo
    .tick/
      EachEmit/
        main.sayo
```

`.any` in the Right position matches any right Term. A concrete Right Space takes priority; `NoStep` from that concrete
candidate does not fall back to `.any`. Multiple candidates within either set are ambiguous. `.any` in the Result
position permits any rightmost anchor and still requires a nonempty result. `.tick`, `.end`, and `.any` are reserved
control slots, not Template names.

SayoVM owns a private standard `Trap`. If Atom `Trap` occurs in a Relation result, evaluation stops with a Semantic Trap
and records an SRI trap event. A World cannot override Trap, and Trap has no fields.

#### 8.6.1 Standard-World For Template

`For` can be expressed entirely through structural capture and Tick:

```text
world/
  For/.any/Each/
  Each/.any/EachApply/
  EachApply/.tick/EachEmit/main.sayo
  EachEmit/.tick/Each/main.sayo
```

```text
# EachApply/.tick/EachEmit/main.sayo
%0 (%0 %1)

# EachEmit/.tick/Each/main.sayo
%0
```

`EachApply` holds `[transform, input]` and produces `[transform, transformed]`. The next Tick retains only transform, so
the unreferenced transformed value is emitted left. No Template has `.end`; For is an open transformer.

```text
For (Add 9) 1
→ Each{Plus{9}} 1
→ EachApply{Plus{9}, 1}
→ EachEmit{Plus{9}, 10}
→ 10 Each{Plus{9}}
```

#### 8.6.2 Standard-World Fold Template

Fold additionally demonstrates accumulator updates and Collector closure through End:

```text
Fold Add 0 1
→ FoldInit{Add} 0 1
→ FoldAccum{Add, 0} 1
→ FoldApply{Add, 0, 1}
→ FoldNext{Add, 1}
→ FoldAccum{Add, 1}
→ FoldClose{Add, 1}       ; End
→ 1 Vanish{Add}           ; Tick
→ 1                        ; Tick
```

Its essential bodies are:

```text
FoldInit/.end/Trap/main.sayo             Trap
FoldApply/.tick/FoldNext/main.sayo       %0 (%0 %1 %2)
FoldNext/.tick/FoldAccum/main.sayo       %0 %1
FoldAccum/.end/FoldClose/main.sayo       %0 %1
FoldClose/.tick/Vanish/main.sayo         %0
```

`FoldClose` holds `[transform, accumulator]`. Its final Tick moves only transform into Vanish, emits accumulator, and then
removes transform and Vanish. Ending before Fold receives an initial value Traps; only `FoldAccum` closes successfully.

### 8.7 Reduction events and Sayo Visualization IR — Implemented

SayoVM records Reducer progress as structured internal SRI events. SRI exists for diagnostics and as input to the Rust SVI
Builder; it is not exposed directly to the Visualizer. Runtime Terms, sequence Cells, evaluation Groups, and Template
control work receive stable execution IDs. Event kinds include:

```text
group            begin an evaluation Group
materialize      resolve a lazy Cell to a Term
attempt          try action on an adjacent pair
not_applicable   continue after NoStep
rewrite          replace two Cells with one or more Cells
state            transition one internal state Cell
end              accept internal End at Group boundary
yield            contribute a Term to Group NormalForm
trap             fail evaluation
```

`sayo --trace-format json` writes one diagnostic SRI JSON document to stderr and NormalForm to stdout. Rust exposes
`Runtime::evaluate_with_trace` and `Runtime::evaluate_file_with_trace`. The version-1 internal contract is defined in
[SRI JSON](sri-json.md).

The Web API converts SRI to a complete sequence of SVI frames in Rust. Every Frame contains its complete Scene,
structured Terms, Focus, and explicit `preserve`, `replace`, `rewrite`, `copy`, or `remove` changes. SRI events that do
not change the visible sequence are omitted from SVI, except Trap. The Visualizer parses no Sayo source and replays no
semantics; it only renders the decided frames. The public contract is [SVI JSON](svi-json.md).

### 8.8 Evaluation Group termination — Implemented

Sayo does not introduce `;`. End of input for a Term that accepts many right operands is expressed by an existing Group
boundary: the entire CLI input, an entire Sayo file, or the inside of `(...)`.

Runtime may try internal End on the final state at that boundary. End is not a user Term or Space. Sayo does not add
Collection, Stream, or Spread semantics: multiple values remain a flat Term sequence. A Collector can retain input and
close to a result on End.

For example, `Sum 1 2 3` could capture each right Term and transition on End to `SumEmit{6}`. That state emits `6` left
and then becomes `Vanish{}`, which disappears on its next Tick. Emission and state disappearance are separate
transitions.

## 9. Standard library and standard Native Relations

The embedded standard World and user World are separate roots. At startup, SayoVM builds the standard World as an
independent `MemoryWorld`; standard files are neither mixed into nor generated inside the user World. The embedded World
contains Capture, Tick, and End Relations for `For`, `Filter`, `Fold`, `Reduce`, and `Duplicate`.

Component lookup order is user World, embedded standard World, then standard Native. A user Component completely shadows
the same standard Component, including checker, schema, ordinary Relations, Capture, Tick, and End. Missing files within
the user Component do not fall back individually. Ambiguity within one Component Traps.

Standard internal state Components made unreachable by shadowing their entry Component are removed from the effective
World, preventing a standard state machine from being mixed with Native fallback halfway through. Unreachable user
Templates remain validation errors; the standard World must also validate independently.

### 9.1 Standard Native Relations — Implemented

SayoVM provides hidden standard primitives. Native `Duplicate`, `For`, `Filter`, `Fold`, and `Reduce` are
semantics-compatible fallbacks when their standard-World definitions are unavailable. Both Sayo bodies and empty
structural-capture Result directories take priority over Native. A capture signature matching Native is not an implicit
Native declaration.

Native may be expanded as an observationally equivalent optimization. Equivalence covers applicable inputs, Result
Space, final sequence, NoStep/Trap behavior, left-emission order, Component shadowing, and StateInstance identity. Native
may compress several Sayo transitions, so exact transition counts and fine SRI trajectories need not match; SRI records
the Native transitions actually executed.

```text
Add Int : Plus       Plus Int : Int
Sub Int : Minus      Minus Int : Int
Mul Int : Times      Times Int : Int
Div Int : Quotient   Quotient Int : Int
Compare Int : CompareInt       CompareInt Int : Ordering
Compare String : CompareString CompareString String : Ordering
Duplicate Int : Duplicate
For * : Each                  Each Int : Each
Filter * : Sieve              Sieve * : Sieve
Odd Int : Keep | Drop
Fold * : FoldInit             FoldInit * : FoldAccum
FoldAccum * : FoldAccum
Reduce * : ReduceInit         ReduceInit * : ReduceAccum
ReduceAccum * : ReduceAccum
```

Bootstrap `Int` is `i64`. Arithmetic accepts whitespace-trimmed `i64` text and returns canonical decimal text;
overflow and division by zero Trap.

Native `Compare left right` supports Int/Int and String/String, producing immutable `Less`, `Equal`, or `Greater` in
Space `Ordering`, through `CompareInt{left}` or `CompareString{left}`. This is a provisional comparison protocol, not a
Boolean system or `if`. General comparison for user Spaces is deferred.

The standard World also provides `equal-to expected actual` for Space checkers. It returns the input String on equality
and transitions to `Miss` otherwise. Only the final String equality operation is a restricted Native primitive, not
directly callable in ordinary evaluation.

`For` accepts any transform Term. `For (Add 9) 1 2 3` creates `Each{Plus{9}}` and transitions through `EachApply` and
`EachEmit` to `10 11 12 Each{Plus{9}}`. It is an **open transformer**: it does not react to End, and the remaining Each
state can accept more input after expansion into an outer sequence.

`Filter` applies its predicate once to each input. The predicate must produce one Term in distinct standard Space `Keep`
or `Drop`; these are ordinary Spaces used for dispatch, not Boolean values. Native predicates return those Spaces, Sayo
Relations declare them as ResultSpace, and Sayo bodies may use primitive Atoms `keep` and `drop`. Any other result Traps.
Filter is also open.

```text
Filter Odd 1 2 3
→ Sieve{Odd} 1 2 3
→ SieveApply{Odd, 1} 2 3
→ SieveChoose{Odd, 1} Keep 2 3
→ 1 Sieve{Odd} 2 3
→ 1 SieveApply{Odd, 2} 3
→ 1 SieveChoose{Odd, 2} Drop 3
→ 1 Sieve{Odd} 3
→ 1 3 Sieve{Odd}
```

`SieveApply` uses dynamic Result `.any`, returning SieveChoose to the left and the predicate result as the anchor.
Ordinary Relations then dispatch `SieveChoose Keep` and `SieveChoose Drop`.

`Fold` requires an initial value and closes on End. `Fold Add 0 1 2 3` produces `6`; `Fold Add 0` produces `0`.
`Reduce` is Fold without an explicit initial value: the first input becomes its accumulator, one input returns itself,
and no input Traps on End. Every combining action must produce exactly one Term.

Thus standard state Terms are either **open transformers** (`For`, `Filter`), which emit values and retain a continuation
that returns NoStep on End, or **Collectors** (`Fold`, `Reduce`), which retain an accumulator and close to one result on
End. The current language has no closed Collection value.

## 10. World validation — Implemented

`sayo check --world <path>` validates at least:

- the World path is a directory;
- every source-backed Space has `space.sayo`;
- implementation-free internal Templates are distinguished from Spaces;
- Right and Result names are known Spaces, Templates, or reserved slots;
- empty Result directories are valid structural capture Relations;
- Tick and End bodies are Sayo, and body-less fixed Results consistently denote zero payload (`Trap` and `.any` require
  bodies; `Vanish` remains reserved);
- Sayo implementations and `template.sayo` schemas parse;
- only Components with `space.sayo` materialize source-backed Terms;
- Relation Right names are Templates or `.any`, `.tick`, or `.end`;
- no `main.native` exists in a World;
- effective Template arities are uniquely derivable after complete standard-Component shadowing; and
- the Runtime can be constructed with all remaining standard connections.

Validation covers both the user World's own syntax and structure and the effective World composed with the standard
World. Success counts report only user-owned Spaces, Templates, Relations, and implementations, excluding the standard
World and Native fallback.

## 11. Bootstrap Runtime guards — Implemented

Defaults are:

- Relation evaluation depth: 256
- state transitions: 100,000

These are bootstrap protections, not final language resource limits. `RuntimeLimits` can configure them per Runtime and a
future Runtime may remove them. Relation depth counts semantic Relation frames, never the width of a flat sequence.

## 12. Current implementation scope

SayoVM currently provides:

- CLI expression and `.sayo` file evaluation;
- Sayo Relations, structural right capture, and `%n` payload projection;
- quoted UTF-8 String Terms;
- user Space checking through `space.sayo` (`Match`, `Miss`, or `Trap`);
- Native integer arithmetic and Int/String comparison with Ordering;
- standard `equal-to` for Space checkers;
- Native fallback state machines for Duplicate, For, Filter, Fold, and Reduce;
- embedded standard-World state machines for the same operations;
- World validation and classified diagnostics;
- internal SRI JSON through `--trace-format json`; and
- complete SVI frames through the Web API and a render-only Visualizer.

An empty user-World overlay can use Native primitives and the standard Sayo World. REPL remains deferred until stable
operation. Collection and Sequence values are absent; multiple results and inputs are flat Term sequences. General state
Terms are expressible through structural capture and Tick/End Relations.

## 13. Deferred decisions

### Conditional branching

Sayo intentionally has no dedicated `if`, Boolean type, or truth-value evaluation rule. Branching is expressed through
Space membership and Relation applicability. A nonmatching candidate does not apply, and resolution proceeds to the next
candidate.

Potential growth order is:

1. one condition through Space membership;
2. alternate Relations for nonmatching conditions;
3. conditions represented by multiple Spaces;
4. schema-only or Native predicate primitives.

Space composition such as `And`, `Or`, and `Not` remains deferred. The REPL, modules and packages, and concrete
self-hosting migration are reconsidered only after the bootstrap SayoVM is stable. User-extensible Native Relations are
not planned; Native implementations remain SayoVM built-ins.
