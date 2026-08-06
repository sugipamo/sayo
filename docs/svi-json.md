# Sayo Visualization IR JSON — version 1

English | [日本語](svi-json.ja.md)

SVI is a Host-facing intermediate representation that lets a Visualizer render SayoVM evaluation without inferring
semantics. It is not an optimization or JIT IR. The Rust SVI Builder converts internal SRI events into a complete sequence
of display frames, and the Visualizer renders each frame without reconstructing state.

```text
SayoVM -> internal Reducer events -> Rust SVI Builder -> SVI JSON -> Visualizer
```

Web API version 1 returns this SVI JSON. SRI JSON remains SayoVM's internal diagnostic Reducer-event format.

## Document

```json
{
  "format": "sayo-visualization-ir",
  "version": 1,
  "result": {
    "status": "normal",
    "terms": ["10", "11", "12", "Each{Plus{9}}"]
  },
  "frames": []
}
```

`result.status` is one of `normal`, `trap`, or `limit`. Every Frame must independently reconstruct the complete display;
a Reader must not derive its Scene from the previous Frame. The SVI Builder omits Reducer events whose Scene is visually
identical to the previous Frame. Exploration, classification, and Relation attempts remain in SRI, while SVI playback
length scales with visible sequence changes. A Trap Frame is retained even if its Scene is unchanged.

## Frame

```json
{
  "frame_id": 14,
  "cause": {},
  "scene": { "rows": [] },
  "focus": { "view_ids": [] },
  "changes": []
}
```

- `frame_id` is a unique sequential ID within the document.
- `cause` explains why the Frame exists and identifies its source Reducer event.
- `scene` is the complete display state at that point.
- `focus` names views of interest for auto-scroll or Trap presentation.
- `changes` explicitly maps the previous Frame to this one and is empty in the first Frame.

## Scene rows

```json
{
  "rows": [
    { "row_id": "context-3", "role": "context", "separator": "space", "items": [] },
    { "row_id": "reduction-4", "role": "reduction", "separator": "space", "anchor_view_id": "f8-context-3-c12", "items": [] }
  ]
}
```

Version 1 defines four `role` values:

- `main`: an ordinary evaluation sequence
- `context`: an ancestor sequence retained while a child Group is evaluated
- `template`: the fixed control Template shown during internal evaluation
- `reduction`: the child Group or control payload currently being evaluated

Nested Group evaluation includes every ancestor `context` Row, ordered from the initial outer sequence to the immediate
caller, followed by the active `reduction` Row. This keeps the originating expression visible while evaluation descends.
Row order is top-to-bottom display order. A Visualizer does not infer Rows from Group parentage. `separator` is either
`space` or `comma`: ordinary Sayo sequences and Group expansion use `space`; state-payload overviews use `comma`. The
Visualizer does not infer punctuation from Row names or roles.

`anchor_view_id` is optional. When present, the Row starts beneath that item in an ancestor Row. SayoVM uses it to keep
Template-control work visually attached to the state Term that initiated the transition, even in a long sequence.

Template-control evaluation can show caller `context`, control Template, and current payload in separate Rows. Multiple
Rows may share the `reduction` role and must not be flattened into one line. During slot evaluation, the Template Row
retains its original placeholder form; value changes appear only in work Rows.

After all slots resolve, Frames fold the detail into the overview with `control_completed`, assemble the overview into a
completed Template with `control_assembled`, and return the completed state to the caller with `control_returned`. Work
Rows consist of an `overview` holding every slot and a `detail` showing only a Group slot undergoing internal expansion.
A simple slot is replaced directly in the overview. A Group slot adds a detail Row while leaving the overview fixed, then
returns its result to the overview when complete.

## View item and identities

```json
{
  "view_id": "view-28",
  "cell_id": 12,
  "term": {}
}
```

- `term_id` identifies a Term in SayoVM.
- `cell_id` identifies a Cell in a Reducer sequence.
- `view_id` identifies one on-screen occurrence.

When one Term appears in multiple places, those occurrences share `term_id` but have distinct `view_id` values. `focus`
and `changes` refer to `view_id` so presentation remains unambiguous.

## Structured terms

A Visualizer does not parse `source`. Terms are recursively structured as `atom`, `state`, `group`, or `placeholder`.
`source` remains the canonical representation for copying, logs, and accessibility.

### Atom

```json
{ "kind": "atom", "term_id": 8, "source": "9", "space": "Int" }
```

### State

```json
{
  "kind": "state",
  "term_id": 20,
  "source": "Plus{9}",
  "space": "Plus",
  "head": "Plus",
  "payload": [
    {
      "view_id": "view-20-payload-0",
      "term": { "kind": "atom", "term_id": 8, "source": "9", "space": "Int" }
    }
  ]
}
```

### Group

```json
{
  "kind": "group",
  "term_id": null,
  "source": "(%0 %1)",
  "items": [
    { "view_id": "group-4-0", "term": { "kind": "placeholder", "term_id": null, "source": "%0", "name": "%0" } },
    { "view_id": "group-4-1", "term": { "kind": "placeholder", "term_id": null, "source": "%1", "name": "%1" } }
  ]
}
```

## Cause and control identity

```json
{
  "kind": "control_slot_resolved",
  "source_event": 14,
  "group_id": 4,
  "control_id": 3,
  "slot": 0
}
```

`control_id` is a unique Runtime-issued ID and must not be inferred from the parent Group, Template name, or implementation
source. One Reducer event may create multiple display Frames, so multiple Frames may share `source_event`.

Control processing can express at least `control_started`, `control_payload_exposed`, `control_slot_started`,
`control_slot_resolved`, `control_completed`, `control_assembled`, and `control_returned`.

## Focus

```json
{
  "view_ids": ["view-28", "view-29"],
  "control_id": 3,
  "style": "active"
}
```

`style` is `active` or `trap`. `active` marks an item of interest; `trap` marks failure position. Presentation decides the
lighting and colors. The Visualizer does not infer targets from the Scene.

## Changes

```json
{
  "kind": "rewrite",
  "from": ["view-plus-9", "view-int-1"],
  "to": ["view-int-10"],
  "operation": "relation_applied"
}
```

Version 1 `kind` values are `preserve`, `replace`, `rewrite`, `copy`, and `remove`. `from` and `to` are explicit; a
Visualizer does not infer Term continuation, duplication, merging, or disappearance from string equality or position.

## Presentation boundary

SVI determines what happened, not how to display it. It contains no coordinates, colors, CSS classes, durations, easing,
or movement directions. A Visualizer chooses presentation without changing Frames, Focus, or Changes.

## Builder architecture

The Rust SVI Builder does not assemble Scenes through direct branching. It pushes `Context`, `Template`, `Overview`, and
`Detail` Layers into an ordered `ScenePlan`, then projects that plan into complete Rows. Layer selection and Term
structuring are separate responsibilities so deeper Detail can gain Row-generation rules without changing semantic replay.

Internally, the Builder distinguishes semantic Views originating from the Reducer from display-only Views duplicated into
Template, Overview, or Detail Rows. This attribute is not serialized. Public SVI contains only the already-classified
`changes`, preventing a display copy from being misrepresented as a semantic `copy`.
