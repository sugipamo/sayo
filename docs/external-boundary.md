# External references and the Host boundary

English | [日本語](external-boundary.ja.md)

This is the minimal prototype contract on the external-effects branch. It separates implemented reference transport
from deferred external execution policy. Sayo still performs only pure evaluation; no Relation calls an Agent or Host
processor. See [the language specification](language-spec.md) for the core evaluation contract.

## Opaque values

```sayo
Document<blob:input>
Agent<handle:coder>
Request{Agent<handle:coder>, Document<blob:input>}
```

- `(...)` evaluates a Group.
- `{...}` holds an ordered payload of Sayo Terms. `%0`, `%1`, etc. refer to these whole Terms.
- `<...>` holds exactly one typed external reference. It is not a Sayo expression or inline raw data.
- `[...]` has no new meaning in this prototype.

Reference kinds are Rust `ExternalKind::Blob` and `ExternalKind::Handle`, not new language Spaces:

| Kind | Host contract |
| --- | --- |
| `blob` | An immutable value in a Host-managed namespace. Processing creates a new blob; the old value remains unchanged. |
| `handle` | A connection to an external target that may change. Keeping a handle does not preserve that target's past state. |

IDs are nonempty ASCII letters, digits, `_`, `-`, or `.`. There is no escaping, embedded expression, tuple, or raw-data
syntax inside `<>`. IDs are not content hashes, credentials, capabilities, or proof that a target exists. They are
visible in source, CLI output, SRI, and SVI. The Host defines lookup and lifetime, validates requests, and owns contents;
the VM neither validates target existence nor dereferences, compares, modifies, or interprets referenced contents.
Blob immutability is a Host obligation, not something an evaluator without access to the contents can enforce.

The role is an ASCII identifier beginning with a letter or `_`. It must name a non-primitive user Space declared with
`space.sayo`. Explicit external construction selects that role without running its source-string checker: a checker
must not inspect the external ID or contents to choose membership. Ordinary Atoms continue to use the checker. Native
primitive and reserved roles are rejected. Reference kinds are checked by the Host, not inferred from the role name.
For example, the test Host requires an Agent handle and a Document blob.

External Terms have no Sayo payload and do not execute Template Tick/End rules. Binary Relations can consume or forward
them by their declared Space, and structural capture preserves a right-hand external Term whole. Like an ordinary
zero-payload left Term, an external left Term is consumed by capture; its reference is not implicitly appended. Use an
explicit semantic envelope to retain every required reference. The combined form `Agent{Ready}<handle:coder>` is deferred.
Schemas and source Space checkers cannot construct external Terms. SVI renders them as atomic Terms, not decomposable states.

## Request, processing, and fixed response

The checked-in [example World](../examples/external_requests/world) uses ordinary capture Relations:

```text
ask Agent<handle:coder> Document<blob:input> accept
→ WithAgent{Agent<handle:coder>} Document<blob:input> accept
→ Request{Agent<handle:coder>, Document<blob:input>} accept
```

Only the complete initial Group's NormalForm is available to the Host. A nested Group's intermediate NormalForm is not
a dispatch boundary. Requests that disappear before the final NormalForm are not dispatched. Trap or limit exhaustion
does not return a usable partial result. The evaluator never dispatches any Request automatically.

The test Host reads the blob, uppercases text prefixed with `upper:`, and otherwise copies the bytes. That content-based
branch happens outside Sayo. It creates a new blob and supplies already-separated fixed observations:

```text
receive Agent<handle:coder> Code<blob:output-1>
→ Response{Agent<handle:coder>, Code<blob:output-1>}
→ Code<blob:output-1> Agent<handle:coder>
```

The Response's outer structure is Sayo payload, so `%1 %0` can forward the whole Code and Agent Terms. This does not split
an opaque blob. The Agent Term is new but the handle still denotes the same connection; this is not an immutable Agent
snapshot. The Host then removes the serviced Request, retains `accept`, and places the response after it:

```text
accept Code<blob:output-1> Agent<handle:coder>
→ Completed{Code<blob:output-1>} Agent<handle:coder>
```

Run the deterministic mock, which needs no credentials or network:

```sh
cargo run --example external_roundtrip
cargo run --example external_roundtrip -- 'plain text'
```

The default Host output is `HELLO`, with one Host call. The second command exercises the copy branch. CLI evaluation of
`examples/external_requests/main.sayo` stops at Request; it does not execute the mock Host.

## Embedding API and replay

`Term::external` exposes typed reference metadata to the Host. `Runtime::payload_terms` returns the semantic payload of
a Template, or `None` for an external/atomic Term. It does not expose external contents.
`Runtime::evaluate_terms` starts a new pure interval from already-materialized Terms: retained state and fixed responses
can be combined without converting display text back into source. Terms must be unmodified and belong to the same
Runtime; internal ownership and IDs are checked. An empty input is an empty NormalForm.

Reusing a fixed response repeats internal computation, not the external operation. With identical input Terms, World,
and observations, pure transitions remain reproducible; internal allocation IDs need not be identical. Persisting or
moving state between Runtime instances is deferred. A saved handle alone is not a saved observation or Agent history.

## Deferred policy

The mock services one Request occurrence exactly once and inserts its response at an explicit position. This is a test
Host's policy, not an automatic language rule. Equal reference IDs or Request contents do not deduplicate occurrences.
Scheduler order does not prescribe Host dispatch order.

Real Agent integration, request/response correlation across multiple pending operations, retries, cancellation, concurrent
execution, immutable Agent snapshots, reference identity operations, role-specific kind schemas, raw data literals,
combined `{}`/`<>` syntax, durable replay storage, and linear IO capabilities are deferred. An execution failure does not
prove that no external effect happened. IO tokens, if added later, must define their own authorization and consumption
rules; opacity alone provides neither linearity nor atomicity.
