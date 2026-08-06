# Sayo Reduction IR JSON — version 1

English | [日本語](sri-json.ja.md)

SRI JSON is SayoVM's internal format for recording and diagnosing Reducer events and passing them to the Rust SVI
Builder. It is not the public rendering contract sent directly to a Visualizer, nor is it a language NormalForm or a
Sayo Term.

The public Visualizer contract is the complete frame sequence defined in `svi-json.md`. A Visualizer does not replay SRI
directly.

## Document

One evaluation is represented by one JSON document.

```json
{
  "format": "sayo-reduction-ir",
  "version": 1,
  "events": []
}
```

- `format` is always `"sayo-reduction-ir"`.
- `version` is the integer `1` for this contract.
- `events` preserves observation order.
- An unknown version must not be interpreted as version 1.

CLI `--trace-format json` writes only this document to stderr. On success, the NormalForm is written to stdout. On Trap,
stdout remains empty; stderr receives valid JSON containing a Trap event, and the process exits with failure status.
Tracing cannot be combined with `--show-space` or `--show-kind`, which add other stderr presentations.

## IDs

Group, Cell, and Term IDs are non-negative JSON integers without display prefixes.

- A Group ID is unique within one Trace.
- A Cell ID is unique within one Trace and is never reused after a rewrite removes it.
- A Term ID refers to a Runtime Term and remains stable when the same Term moves to another Cell.
- `term_id` is `null` for Terms that do not yet have an identity, such as pre-materialization source.

## Term and Relation

A Term is embedded as follows:

```json
{
  "term_id": 1,
  "source": "add",
  "space": "Add",
  "implementation": "atom",
  "fragments": []
}
```

`source` is a stable trace display string. Native states also receive forms such as `Plus{5}`. In version 1,
`implementation` is one of `atom`, `sayo`, or `native`.

For a state Term, optional `fragments` describe the origin of Terms retained inside its display.

```json
{
  "term_id": 12,
  "source": "EachApply{Plus{9}, 1}",
  "space": "Each",
  "implementation": "native",
  "fragments": [
    { "path": [0], "origin_id": 8 },
    { "path": [0, 0], "origin_id": 3 },
    { "path": [1], "origin_id": 10 }
  ]
}
```

- `path` is a zero-based index sequence that walks state payloads in `source` from the outside inward.
- `origin_id` is the Term ID retained at that position.
- The same `origin_id` before and after a transition means the same Term continues, even if its display or position changes.
- Empty `fragments` are omitted from JSON. A version 1 Reader may ignore the field when it does not recognize it.

A Relation is embedded as follows:

```json
{
  "left_space": "Add",
  "right_space": "Int",
  "result_space": "Plus",
  "executor": "native"
}
```

## Events

Every event has a string `event` tag. Version 1 defines the following tags:

| event | primary fields | meaning |
|---|---|---|
| `group_started` | `group`, `parent`, `cells`, `sources` | Create an evaluation Group and its unevaluated initial source sequence. |
| `materialized` | `group`, `cell`, `term` | Evaluate a lazy Cell into a Term. |
| `attempted` | `group`, `left_cell`, `right_cell`, `left`, `right` | Attempt application on two adjacent Terms. |
| `not_applicable` | `group`, `left_cell`, `right_cell`, `reason` | Advance right after `NoStep`. |
| `rewritten` | `group`, `removed_left`, `removed_right`, `inserted`, `relation` | Replace two Cells with a Relation result. |
| `state_transition` | `group`, `removed`, `inserted`, `control_id?` | Apply a Tick transition to one Cell. |
| `end_transition` | `group`, `removed`, `inserted`, `control_id?` | Apply an End transition at a Group boundary. |
| `cursor_moved` | `group`, `from`, `to`, `reason` | Move the explicit Scheduler cursor. |
| `yielded` | `group`, `cell`, `term` | Retain a Term in the Group's NormalForm. |
| `trapped` | `group`, `category`, `message` | Stop evaluation with failure. |

`parent`, `from`, `to`, and `group` when a Trap occurs outside a Group may be `null`.

A `group_started` that evaluates control-Relation payload includes an additional `control` object:

```json
{
  "control_id": 3,
  "transition": "tick",
  "result_template": "EachEmit",
  "implementation_source": "%0 (%0 %1)",
  "slots": ["%0", "(%0 %1)"],
  "slot": 0,
  "slot_count": 2
}
```

`control_id` is unique within one Trace and is shared by every child Group that evaluates a slot for one control
execution. This lets the SVI Builder recognize that child Groups build payload positions of one target Template rather
than independent state changes. Ordinary Groups omit `control`.

`inserted` is ordered, and every element has this form:

```json
{
  "cell": 3,
  "term": {
    "term_id": 8,
    "source": "8",
    "space": "Int",
    "implementation": "atom"
  }
}
```

Array order is left-to-right Cell order after the rewrite. An empty array can represent disappearance of a state and its
payload.

## Replay contract

An SRI Reader or SVI Builder can reconstruct state by applying `events` exactly once from the beginning.

1. `group_started` creates the Group and its initial Cell order. `sources` has the same length as `cells`; each entry is
   the canonical Sayo source initially held by the Cell at the same index. The SVI Builder can therefore construct the
   initial sequence before materialization.
2. `materialized` associates a Term with a Cell.
3. `rewritten`, `state_transition`, and `end_transition` remove the declared Cells and insert `inserted` at that position
   in order.
4. `yielded` confirms final residual Cells.
5. `trapped` preserves the state at that point as a failed state.

Version 1 does not permit omitted events, event reordering, or Cell ID reuse. If streaming becomes necessary, JSON Lines
may be considered as a separate version or transport while this single-document form remains canonical.
