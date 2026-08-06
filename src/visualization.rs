use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::model::Expression;
use crate::parser;
use crate::trace::{
    SayoTrace, TraceCellId, TraceControlGroup, TraceControlId, TraceEvent, TraceFragment,
    TraceGroupId, TraceTerm,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviDocument {
    format: &'static str,
    version: u32,
    result: SviResult,
    frames: Vec<SviFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviResult {
    status: String,
    terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviFrame {
    pub frame_id: u64,
    pub cause: SviCause,
    pub scene: SviScene,
    pub focus: SviFocus,
    pub changes: Vec<SviChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviCause {
    pub kind: String,
    pub source_event: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<TraceGroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_id: Option<TraceControlId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<usize>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviScene {
    pub rows: Vec<SviRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviRow {
    pub row_id: String,
    pub role: SviRowRole,
    pub separator: SviRowSeparator,
    pub items: Vec<SviViewItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SviRowSeparator {
    Space,
    Comma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SviRowRole {
    Main,
    Context,
    Template,
    Reduction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviViewItem {
    pub view_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_id: Option<TraceCellId>,
    pub term: SviTerm,
    #[serde(skip)]
    domain: ViewDomain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SviTerm {
    Atom {
        term_id: Option<u64>,
        source: String,
        space: String,
    },
    State {
        term_id: Option<u64>,
        source: String,
        space: String,
        head: String,
        payload: Vec<SviViewItem>,
    },
    Group {
        term_id: Option<u64>,
        source: String,
        items: Vec<SviViewItem>,
    },
    Placeholder {
        term_id: Option<u64>,
        source: String,
        name: String,
    },
}

impl SviTerm {
    pub fn source(&self) -> &str {
        match self {
            Self::Atom { source, .. }
            | Self::State { source, .. }
            | Self::Group { source, .. }
            | Self::Placeholder { source, .. } => source,
        }
    }

    fn term_id(&self) -> Option<u64> {
        match self {
            Self::Atom { term_id, .. }
            | Self::State { term_id, .. }
            | Self::Group { term_id, .. }
            | Self::Placeholder { term_id, .. } => *term_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviFocus {
    pub view_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_id: Option<TraceControlId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<SviFocusStyle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SviFocusStyle {
    Active,
    Trap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SviChange {
    pub kind: SviChangeKind,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub operation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SviChangeKind {
    Preserve,
    Replace,
    Rewrite,
    Copy,
    Remove,
}

#[derive(Clone)]
struct ReplayCell {
    cell: TraceCellId,
    term: Option<TraceTerm>,
    source: Option<String>,
}

#[derive(Clone)]
struct ReplayGroup {
    parent: Option<TraceGroupId>,
    cells: Vec<ReplayCell>,
    control: Option<TraceControlGroup>,
}

#[derive(Default)]
struct ReplayState {
    groups: HashMap<TraceGroupId, ReplayGroup>,
    controls: HashMap<TraceControlId, Vec<Option<TraceTerm>>>,
    last_attempted: HashMap<TraceGroupId, Vec<TraceCellId>>,
    root: Option<TraceGroupId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlDetail {
    Hidden,
    Exposed,
    Working,
    Assembled,
}

enum SceneLayer<'a> {
    Cells {
        row_id: String,
        role: SviRowRole,
        cells: &'a [ReplayCell],
    },
    ControlTemplate {
        row_id: String,
        control: &'a TraceControlGroup,
        assembled: bool,
    },
    ControlOverview {
        row_id: String,
        group: &'a ReplayGroup,
        control: &'a TraceControlGroup,
    },
    ControlDetail {
        row_id: String,
        cells: &'a [ReplayCell],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewDomain {
    Semantic,
    ControlProjection,
}

struct ViewIdentity<'a> {
    domain: ViewDomain,
    stable_key: &'a str,
}

struct ScenePlan<'a> {
    layers: Vec<SceneLayer<'a>>,
}

impl SviDocument {
    pub fn build(trace: &SayoTrace, status: impl Into<String>, result_terms: Vec<String>) -> Self {
        let mut state = ReplayState::default();
        let mut frames = Vec::new();
        for (source_event, event) in trace.events().iter().enumerate() {
            apply_event(&mut state, event);
            let frame = build_frame(frames.len() as u64, source_event, event, &state);
            push_visible_frame(&mut frames, frame);
            if matches!(event, TraceEvent::GroupStarted { control: Some(control), .. } if control.slot == 0)
            {
                let frame =
                    build_control_exposed_frame(frames.len() as u64, source_event, event, &state);
                push_visible_frame(&mut frames, frame);
            }
            if let TraceEvent::Yielded { group, .. } = event
                && let Some(control) = state
                    .groups
                    .get(group)
                    .and_then(|group| group.control.as_ref())
                && control.slot + 1 == control.slot_count
            {
                let frame =
                    build_control_completed_frame(frames.len() as u64, source_event, event, &state);
                push_visible_frame(&mut frames, frame);
                let frame =
                    build_control_assembled_frame(frames.len() as u64, source_event, event, &state);
                push_visible_frame(&mut frames, frame);
            }
        }
        add_changes(&mut frames);
        Self {
            format: "sayo-visualization-ir",
            version: 1,
            result: SviResult {
                status: status.into(),
                terms: result_terms,
            },
            frames,
        }
    }

    pub fn frames(&self) -> &[SviFrame] {
        &self.frames
    }
}

fn push_visible_frame(frames: &mut Vec<SviFrame>, frame: SviFrame) {
    let visible_change = frames
        .last()
        .is_none_or(|previous| !same_visible_scene(&previous.scene, &frame.scene));
    if visible_change || frame.cause.kind == "trapped" {
        frames.push(frame);
    }
}

fn same_visible_scene(left: &SviScene, right: &SviScene) -> bool {
    left.rows.len() == right.rows.len()
        && left.rows.iter().zip(&right.rows).all(|(left, right)| {
            left.role == right.role
                && left.separator == right.separator
                && left.items.len() == right.items.len()
                && left
                    .items
                    .iter()
                    .zip(&right.items)
                    .all(|(left, right)| left.term.source() == right.term.source())
        })
}

fn apply_event(state: &mut ReplayState, event: &TraceEvent) {
    match event {
        TraceEvent::GroupStarted {
            group,
            parent,
            cells,
            sources,
            control,
        } => {
            state.root.get_or_insert(*group);
            if let Some(control) = control {
                state
                    .controls
                    .entry(control.control_id)
                    .or_insert_with(|| vec![None; control.slot_count]);
            }
            state.groups.insert(
                *group,
                ReplayGroup {
                    parent: *parent,
                    cells: cells
                        .iter()
                        .enumerate()
                        .map(|(index, cell)| ReplayCell {
                            cell: *cell,
                            term: None,
                            source: sources.get(index).cloned(),
                        })
                        .collect(),
                    control: control.clone(),
                },
            );
        }
        TraceEvent::Materialized { group, cell, term } => {
            if let Some(target) = find_cell_mut(state.groups.get_mut(group), *cell) {
                target.term = Some(term.clone());
            }
        }
        TraceEvent::Attempted {
            group,
            left_cell,
            right_cell,
            ..
        } => {
            state
                .last_attempted
                .insert(*group, vec![*left_cell, *right_cell]);
        }
        TraceEvent::Rewritten {
            group,
            removed_left,
            removed_right,
            inserted,
            ..
        } => {
            replace_cells(
                state.groups.get_mut(group),
                &[*removed_left, *removed_right],
                inserted,
            );
        }
        TraceEvent::StateTransition {
            group,
            removed,
            inserted,
            ..
        }
        | TraceEvent::EndTransition {
            group,
            removed,
            inserted,
            ..
        } => {
            replace_cells(state.groups.get_mut(group), &[*removed], inserted);
        }
        TraceEvent::Yielded { group, cell, term } => {
            if let Some(target) = find_cell_mut(state.groups.get_mut(group), *cell) {
                target.term = Some(term.clone());
            }
            if let Some(control) = state
                .groups
                .get(group)
                .and_then(|group| group.control.as_ref())
                && let Some(slots) = state.controls.get_mut(&control.control_id)
            {
                slots[control.slot] = Some(term.clone());
            }
        }
        TraceEvent::NotApplicable { .. }
        | TraceEvent::CursorMoved { .. }
        | TraceEvent::Trapped { .. } => {}
    }
}

fn find_cell_mut(group: Option<&mut ReplayGroup>, cell: TraceCellId) -> Option<&mut ReplayCell> {
    group?.cells.iter_mut().find(|entry| entry.cell == cell)
}

fn replace_cells(
    group: Option<&mut ReplayGroup>,
    removed: &[TraceCellId],
    inserted: &[crate::trace::TraceInsertion],
) {
    let Some(group) = group else { return };
    let insertion_index = removed
        .iter()
        .filter_map(|cell| group.cells.iter().position(|entry| entry.cell == *cell))
        .min()
        .unwrap_or(group.cells.len());
    group.cells.retain(|entry| !removed.contains(&entry.cell));
    group.cells.splice(
        insertion_index..insertion_index,
        inserted.iter().map(|entry| ReplayCell {
            cell: entry.cell,
            term: Some(entry.term.clone()),
            source: Some(entry.term.source.clone()),
        }),
    );
}

fn build_frame(
    frame_id: u64,
    source_event: usize,
    event: &TraceEvent,
    state: &ReplayState,
) -> SviFrame {
    let group_id = event_group(event).or(state.root);
    let group = group_id.and_then(|id| state.groups.get(&id));
    let control = group.and_then(|group| group.control.as_ref());
    let control_id = control
        .map(|control| control.control_id)
        .or_else(|| event_control_id(event));
    let control_detail = if matches!(
        event,
        TraceEvent::GroupStarted {
            control: Some(control),
            ..
        } if control.slot == 0
    ) {
        ControlDetail::Hidden
    } else {
        ControlDetail::Working
    };
    let scene = build_scene(frame_id, group_id, group, state, control_detail);
    let focus_cells = focus_cells(event, state);
    let focus_ids = scene
        .rows
        .iter()
        .flat_map(|row| row.items.iter())
        .filter(|item| item.cell_id.is_some_and(|cell| focus_cells.contains(&cell)))
        .map(|item| item.view_id.clone())
        .collect::<Vec<_>>();
    SviFrame {
        frame_id,
        cause: SviCause {
            kind: if event_control_id(event).is_some() {
                "control_returned".into()
            } else if matches!(event, TraceEvent::Yielded { .. }) && control.is_some() {
                "control_slot_resolved".into()
            } else {
                cause_kind(event).into()
            },
            source_event,
            group_id,
            control_id,
            slot: control.map(|control| control.slot),
            summary: event_summary(event),
        },
        scene,
        focus: SviFocus {
            view_ids: focus_ids,
            control_id,
            style: if matches!(event, TraceEvent::Trapped { .. }) {
                Some(SviFocusStyle::Trap)
            } else if focus_cells.is_empty() {
                None
            } else {
                Some(SviFocusStyle::Active)
            },
        },
        changes: Vec::new(),
    }
}

fn build_control_exposed_frame(
    frame_id: u64,
    source_event: usize,
    event: &TraceEvent,
    state: &ReplayState,
) -> SviFrame {
    let mut frame = build_frame(frame_id, source_event, event, state);
    frame.cause.kind = "control_payload_exposed".into();
    frame.cause.summary = "control payload exposed".into();
    let group_id = frame.cause.group_id;
    let group = group_id.and_then(|id| state.groups.get(&id));
    frame.scene = build_scene(frame_id, group_id, group, state, ControlDetail::Exposed);
    frame.focus.view_ids.clear();
    frame.focus.style = None;
    frame
}

fn build_control_completed_frame(
    frame_id: u64,
    source_event: usize,
    event: &TraceEvent,
    state: &ReplayState,
) -> SviFrame {
    let mut frame = build_frame(frame_id, source_event, event, state);
    frame.cause.kind = "control_completed".into();
    frame.cause.summary = "control completed".into();
    let group_id = frame.cause.group_id;
    let group = group_id.and_then(|id| state.groups.get(&id));
    frame.scene = build_scene(frame_id, group_id, group, state, ControlDetail::Exposed);
    frame.focus.view_ids = frame
        .scene
        .rows
        .last()
        .into_iter()
        .flat_map(|row| row.items.iter().map(|item| item.view_id.clone()))
        .collect();
    frame
}

fn build_control_assembled_frame(
    frame_id: u64,
    source_event: usize,
    event: &TraceEvent,
    state: &ReplayState,
) -> SviFrame {
    let mut frame = build_frame(frame_id, source_event, event, state);
    frame.cause.kind = "control_assembled".into();
    frame.cause.summary = "resolved payload assembled into control result".into();
    let group_id = frame.cause.group_id;
    let group = group_id.and_then(|id| state.groups.get(&id));
    frame.scene = build_scene(frame_id, group_id, group, state, ControlDetail::Assembled);
    frame.focus.view_ids.clear();
    frame.focus.style = None;
    frame
}

fn build_scene(
    frame_id: u64,
    group_id: Option<TraceGroupId>,
    group: Option<&ReplayGroup>,
    state: &ReplayState,
    control_detail: ControlDetail,
) -> SviScene {
    let Some(group) = group else {
        return SviScene { rows: Vec::new() };
    };
    let plan = ScenePlan::for_group(group_id.unwrap_or(0), group, state, control_detail);
    SviScene {
        rows: plan.render(frame_id, state),
    }
}

impl<'a> ScenePlan<'a> {
    fn for_group(
        group_id: TraceGroupId,
        group: &'a ReplayGroup,
        state: &'a ReplayState,
        control_detail: ControlDetail,
    ) -> Self {
        let mut layers = Vec::new();
        Self::push_parent_context(&mut layers, group, state);
        if let Some(control) = &group.control {
            Self::push_control_layers(&mut layers, group_id, group, control, control_detail);
        } else {
            let (prefix, role) = if group.parent.is_some() {
                ("reduction", SviRowRole::Reduction)
            } else {
                ("main", SviRowRole::Main)
            };
            layers.push(SceneLayer::Cells {
                row_id: format!("{prefix}-{group_id}"),
                role,
                cells: &group.cells,
            });
        }
        Self { layers }
    }

    fn push_parent_context(
        layers: &mut Vec<SceneLayer<'a>>,
        group: &'a ReplayGroup,
        state: &'a ReplayState,
    ) {
        let Some(parent_id) = group.parent else {
            return;
        };
        let Some(parent) = state.groups.get(&parent_id) else {
            return;
        };
        layers.push(SceneLayer::Cells {
            row_id: format!("context-{parent_id}"),
            role: SviRowRole::Context,
            cells: &parent.cells,
        });
    }

    fn push_control_layers(
        layers: &mut Vec<SceneLayer<'a>>,
        group_id: TraceGroupId,
        group: &'a ReplayGroup,
        control: &'a TraceControlGroup,
        detail: ControlDetail,
    ) {
        layers.push(SceneLayer::ControlTemplate {
            row_id: format!("reduction-{group_id}"),
            control,
            assembled: detail == ControlDetail::Assembled,
        });
        if matches!(detail, ControlDetail::Exposed | ControlDetail::Working) {
            layers.push(SceneLayer::ControlOverview {
                row_id: format!("control-overview-{}", control.control_id),
                group,
                control,
            });
        }
        if detail == ControlDetail::Working && control.slots[control.slot].starts_with('(') {
            layers.push(SceneLayer::ControlDetail {
                row_id: format!("control-detail-{}-{}", control.control_id, control.slot),
                cells: &group.cells,
            });
        }
    }

    fn render(self, frame_id: u64, state: &ReplayState) -> Vec<SviRow> {
        self.layers
            .into_iter()
            .map(|layer| layer.render(frame_id, state))
            .collect()
    }
}

impl SceneLayer<'_> {
    fn render(self, frame_id: u64, state: &ReplayState) -> SviRow {
        match self {
            Self::Cells {
                row_id,
                role,
                cells,
            } => row_from_cells(frame_id, row_id, role, cells),
            Self::ControlTemplate {
                row_id,
                control,
                assembled,
            } => SviRow {
                row_id,
                role: SviRowRole::Template,
                separator: SviRowSeparator::Space,
                items: vec![control_item(frame_id, control, state, assembled)],
            },
            Self::ControlOverview {
                row_id,
                group,
                control,
            } => SviRow {
                items: control_overview_items(frame_id, &row_id, group, control, state),
                row_id,
                role: SviRowRole::Reduction,
                separator: SviRowSeparator::Comma,
            },
            Self::ControlDetail { row_id, cells } => SviRow {
                items: items_from_cells(frame_id, &row_id, cells),
                row_id,
                role: SviRowRole::Reduction,
                separator: SviRowSeparator::Space,
            },
        }
    }
}

fn row_from_cells(frame_id: u64, row_id: String, role: SviRowRole, cells: &[ReplayCell]) -> SviRow {
    let items = items_from_cells(frame_id, &row_id, cells);
    SviRow {
        row_id,
        role,
        separator: SviRowSeparator::Space,
        items,
    }
}

fn items_from_cells(frame_id: u64, row_id: &str, cells: &[ReplayCell]) -> Vec<SviViewItem> {
    cells
        .iter()
        .map(|cell| {
            let view_id = format!("f{frame_id}-{row_id}-c{}", cell.cell);
            let term = cell.term.as_ref().map_or_else(
                || {
                    structured_term(
                        cell.source.as_deref().unwrap_or(""),
                        None,
                        "",
                        &[],
                        &view_id,
                    )
                },
                |term| {
                    structured_term(
                        &term.source,
                        term.term_id,
                        &term.space,
                        &term.fragments,
                        &view_id,
                    )
                },
            );
            SviViewItem {
                view_id,
                cell_id: Some(cell.cell),
                term,
                domain: ViewDomain::Semantic,
            }
        })
        .collect()
}

fn control_item(
    frame_id: u64,
    control: &TraceControlGroup,
    state: &ReplayState,
    assembled: bool,
) -> SviViewItem {
    let view_id = format!("f{frame_id}-control-{}", control.control_id);
    let resolved = state.controls.get(&control.control_id);
    if assembled {
        let payload = control
            .slots
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let slot_view_id = format!(
                    "f{frame_id}-control-overview-{}-slot-{index}",
                    control.control_id
                );
                let resolved_term = resolved
                    .and_then(|slots| slots.get(index))
                    .and_then(Option::as_ref);
                let mut term = resolved_term.map_or_else(
                    || structured_term(source, None, "", &[], &slot_view_id),
                    |term| {
                        structured_term(
                            &term.source,
                            term.term_id,
                            &term.space,
                            &term.fragments,
                            &slot_view_id,
                        )
                    },
                );
                set_term_domain(&mut term, ViewDomain::ControlProjection);
                SviViewItem {
                    view_id: slot_view_id,
                    cell_id: None,
                    term,
                    domain: ViewDomain::ControlProjection,
                }
            })
            .collect::<Vec<_>>();
        let source = format!(
            "{}{{{}}}",
            control.result_template,
            payload
                .iter()
                .map(|item| item.term.source())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return SviViewItem {
            view_id,
            cell_id: None,
            term: SviTerm::State {
                term_id: None,
                source,
                space: control.result_template.clone(),
                head: control.result_template.clone(),
                payload,
            },
            domain: ViewDomain::ControlProjection,
        };
    }
    let source = format!(
        "{}{{{}}}",
        control.result_template,
        control.slots.join(", ")
    );
    let mut term = structured_term(&source, None, &control.result_template, &[], &view_id);
    set_term_domain(&mut term, ViewDomain::ControlProjection);
    SviViewItem {
        view_id: view_id.clone(),
        cell_id: None,
        term,
        domain: ViewDomain::ControlProjection,
    }
}

fn control_overview_items(
    frame_id: u64,
    row_id: &str,
    group: &ReplayGroup,
    control: &TraceControlGroup,
    state: &ReplayState,
) -> Vec<SviViewItem> {
    let resolved = state.controls.get(&control.control_id);
    control
        .slots
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let view_id = format!("f{frame_id}-{row_id}-slot-{index}");
            let resolved_term = resolved
                .and_then(|slots| slots.get(index))
                .and_then(Option::as_ref);
            let current_term = (index == control.slot && !source.starts_with('('))
                .then(|| group.cells.first())
                .flatten()
                .and_then(|cell| cell.term.as_ref());
            let term = current_term.or(resolved_term).map_or_else(
                || structured_term(source, None, "", &[], &view_id),
                |term| {
                    structured_term(
                        &term.source,
                        term.term_id,
                        &term.space,
                        &term.fragments,
                        &view_id,
                    )
                },
            );
            let mut term = term;
            set_term_domain(&mut term, ViewDomain::ControlProjection);
            SviViewItem {
                view_id: view_id.clone(),
                cell_id: None,
                term,
                domain: ViewDomain::ControlProjection,
            }
        })
        .collect()
}

fn structured_term(
    source: &str,
    term_id: Option<u64>,
    space: &str,
    fragments: &[TraceFragment],
    view_id: &str,
) -> SviTerm {
    match parser::parse(source) {
        Ok(expression) => {
            term_from_expression(&expression, source, term_id, space, fragments, view_id, &[])
        }
        Err(_) => SviTerm::Atom {
            term_id,
            source: source.into(),
            space: space.into(),
        },
    }
}

fn term_from_expression(
    expression: &Expression,
    source: &str,
    term_id: Option<u64>,
    space: &str,
    fragments: &[TraceFragment],
    view_id: &str,
    path: &[usize],
) -> SviTerm {
    match expression {
        Expression::PayloadRef(index) => SviTerm::Placeholder {
            term_id,
            source: source.into(),
            name: format!("%{index}"),
        },
        Expression::State { template, payload } => SviTerm::State {
            term_id,
            source: source.into(),
            space: if space.is_empty() {
                template.clone()
            } else {
                space.into()
            },
            head: template.clone(),
            payload: payload
                .iter()
                .enumerate()
                .map(|(index, expression)| {
                    let mut child_path = path.to_vec();
                    child_path.push(index);
                    nested_item(expression, fragments, view_id, &child_path)
                })
                .collect(),
        },
        Expression::Group(inner) => SviTerm::Group {
            term_id,
            source: source.into(),
            items: expression_terms(inner)
                .iter()
                .enumerate()
                .map(|(index, expression)| {
                    let mut child_path = path.to_vec();
                    child_path.push(index);
                    nested_item(expression, fragments, view_id, &child_path)
                })
                .collect(),
        },
        Expression::Apply(_, _) => SviTerm::Group {
            term_id,
            source: source.into(),
            items: expression_terms(expression)
                .iter()
                .enumerate()
                .map(|(index, expression)| {
                    let mut child_path = path.to_vec();
                    child_path.push(index);
                    nested_item(expression, fragments, view_id, &child_path)
                })
                .collect(),
        },
        Expression::Atom(_) | Expression::StringLiteral(_) => SviTerm::Atom {
            term_id,
            source: source.into(),
            space: space.into(),
        },
    }
}

fn nested_item(
    expression: &Expression,
    fragments: &[TraceFragment],
    parent_view: &str,
    path: &[usize],
) -> SviViewItem {
    let view_id = format!(
        "{parent_view}-p{}",
        path.iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("-")
    );
    let term_id = fragments
        .iter()
        .find(|fragment| fragment.path == path)
        .map(|fragment| fragment.origin_id);
    let source = expression.render_source();
    SviViewItem {
        view_id: view_id.clone(),
        cell_id: None,
        term: term_from_expression(expression, &source, term_id, "", fragments, &view_id, path),
        domain: ViewDomain::Semantic,
    }
}

fn set_term_domain(term: &mut SviTerm, domain: ViewDomain) {
    let children = match term {
        SviTerm::State { payload, .. } => payload,
        SviTerm::Group { items, .. } => items,
        SviTerm::Atom { .. } | SviTerm::Placeholder { .. } => return,
    };
    for child in children {
        child.domain = domain;
        set_term_domain(&mut child.term, domain);
    }
}

fn expression_terms(expression: &Expression) -> Vec<&Expression> {
    fn collect<'a>(expression: &'a Expression, output: &mut Vec<&'a Expression>) {
        if let Expression::Apply(left, right) = expression {
            collect(left, output);
            output.push(right);
        } else {
            output.push(expression);
        }
    }
    let mut terms = Vec::new();
    collect(expression, &mut terms);
    terms
}

fn event_group(event: &TraceEvent) -> Option<TraceGroupId> {
    match event {
        TraceEvent::GroupStarted { group, .. }
        | TraceEvent::Materialized { group, .. }
        | TraceEvent::Attempted { group, .. }
        | TraceEvent::NotApplicable { group, .. }
        | TraceEvent::Rewritten { group, .. }
        | TraceEvent::StateTransition { group, .. }
        | TraceEvent::EndTransition { group, .. }
        | TraceEvent::CursorMoved { group, .. }
        | TraceEvent::Yielded { group, .. } => Some(*group),
        TraceEvent::Trapped { group, .. } => *group,
    }
}

fn event_control_id(event: &TraceEvent) -> Option<TraceControlId> {
    match event {
        TraceEvent::StateTransition { control_id, .. }
        | TraceEvent::EndTransition { control_id, .. } => *control_id,
        _ => None,
    }
}

fn focus_cells(event: &TraceEvent, state: &ReplayState) -> Vec<TraceCellId> {
    match event {
        TraceEvent::Materialized { cell, .. } | TraceEvent::Yielded { cell, .. } => vec![*cell],
        TraceEvent::Attempted {
            left_cell,
            right_cell,
            ..
        }
        | TraceEvent::NotApplicable {
            left_cell,
            right_cell,
            ..
        } => vec![*left_cell, *right_cell],
        TraceEvent::Rewritten { inserted, .. }
        | TraceEvent::StateTransition { inserted, .. }
        | TraceEvent::EndTransition { inserted, .. } => {
            inserted.iter().map(|entry| entry.cell).collect()
        }
        TraceEvent::CursorMoved { to, .. } => to.iter().copied().collect(),
        TraceEvent::Trapped { group, .. } => group
            .and_then(|group| state.last_attempted.get(&group).cloned())
            .unwrap_or_default(),
        TraceEvent::GroupStarted { .. } => Vec::new(),
    }
}

fn cause_kind(event: &TraceEvent) -> &'static str {
    match event {
        TraceEvent::GroupStarted {
            control: Some(control),
            ..
        } if control.slot == 0 => "control_started",
        TraceEvent::GroupStarted {
            control: Some(_), ..
        } => "control_slot_started",
        TraceEvent::GroupStarted { .. } => "evaluation_started",
        TraceEvent::Materialized { .. } => "materialized",
        TraceEvent::Attempted { .. } => "relation_attempted",
        TraceEvent::NotApplicable { .. } => "relation_missed",
        TraceEvent::Rewritten { .. } => "relation_applied",
        TraceEvent::StateTransition { .. } => "state_changed",
        TraceEvent::EndTransition { .. } => "end_changed",
        TraceEvent::CursorMoved { .. } => "cursor_moved",
        TraceEvent::Yielded { .. } => "yielded",
        TraceEvent::Trapped { .. } => "trapped",
    }
}

fn event_summary(event: &TraceEvent) -> String {
    match event {
        TraceEvent::GroupStarted { group, .. } => format!("group {group} started"),
        TraceEvent::Materialized { cell, term, .. } => {
            format!("cell {cell} became {}", term.source)
        }
        TraceEvent::Attempted { left, right, .. } => {
            format!("{} acts on {}", left.source, right.source)
        }
        TraceEvent::NotApplicable { reason, .. } => reason.clone(),
        TraceEvent::Rewritten { relation, .. } => format!(
            "{} {} -> {}",
            relation.left_space, relation.right_space, relation.result_space
        ),
        TraceEvent::StateTransition { .. } => "state changed".into(),
        TraceEvent::EndTransition { .. } => "End changed state".into(),
        TraceEvent::CursorMoved { .. } => "cursor moved".into(),
        TraceEvent::Yielded { term, .. } => format!("{} remains", term.source),
        TraceEvent::Trapped {
            category, message, ..
        } => format!("{category}: {message}"),
    }
}

fn add_changes(frames: &mut [SviFrame]) {
    for index in 1..frames.len() {
        let (previous, current) = frames.split_at_mut(index);
        current[0].changes = changes_between(&previous[index - 1], &current[0]);
    }
}

fn changes_between(previous: &SviFrame, current: &SviFrame) -> Vec<SviChange> {
    let before = flat_items(&previous.scene);
    let after = flat_items(&current.scene);
    let mut used = HashSet::new();
    let mut changes = Vec::new();
    let mut removed = Vec::new();
    let mut preserved_sources = HashSet::new();
    for from in &before {
        let target = preserve_source_is_visible(current, from)
            .then(|| {
                after
                    .iter()
                    .find(|to| {
                        !used.contains(&to.view_id)
                            && same_item_identity_for_frame(current, from, to)
                    })
                    .copied()
            })
            .flatten();
        if let Some(to) = target {
            used.insert(to.view_id.clone());
            preserved_sources.insert(from.view_id.clone());
            changes.push(SviChange {
                kind: SviChangeKind::Preserve,
                from: vec![from.view_id.clone()],
                to: vec![to.view_id.clone()],
                operation: current.cause.kind.clone(),
            });
        } else {
            removed.push(from.view_id.clone());
        }
    }
    let mut copied = HashSet::new();
    for from in &before {
        if !preserved_sources.contains(&from.view_id) || from.term.term_id().is_none() {
            continue;
        }
        let targets = after
            .iter()
            .filter(|to| !used.contains(&to.view_id) && from.term.term_id() == to.term.term_id())
            .map(|to| to.view_id.clone())
            .collect::<Vec<_>>();
        if !targets.is_empty() {
            copied.extend(targets.iter().cloned());
            changes.push(SviChange {
                kind: SviChangeKind::Copy,
                from: vec![from.view_id.clone()],
                to: targets,
                operation: current.cause.kind.clone(),
            });
        }
    }
    let created = after
        .iter()
        .filter(|to| !used.contains(&to.view_id) && !copied.contains(&to.view_id))
        .map(|to| to.view_id.clone())
        .collect::<Vec<_>>();
    if !removed.is_empty() || !created.is_empty() {
        changes.push(SviChange {
            kind: match current.cause.kind.as_str() {
                "relation_applied" => SviChangeKind::Rewrite,
                "state_changed"
                | "end_changed"
                | "control_payload_exposed"
                | "control_slot_resolved"
                | "control_completed"
                | "control_assembled"
                | "control_returned" => SviChangeKind::Replace,
                _ if created.is_empty() => SviChangeKind::Remove,
                _ => SviChangeKind::Replace,
            },
            from: removed,
            to: created,
            operation: current.cause.kind.clone(),
        });
    }
    changes
}

fn preserve_source_is_visible(frame: &SviFrame, item: &SviViewItem) -> bool {
    frame.cause.kind != "control_returned"
        || view_identity(item).domain != ViewDomain::ControlProjection
}

fn same_item_identity(from: &SviViewItem, to: &SviViewItem) -> bool {
    let same_cell = from.cell_id.is_some() && from.cell_id == to.cell_id;
    let same_term = from.term.term_id().is_some() && from.term.term_id() == to.term.term_id();
    same_cell || same_term || same_projection_identity(from, to)
}

fn same_item_identity_for_frame(frame: &SviFrame, from: &SviViewItem, to: &SviViewItem) -> bool {
    if frame.cause.kind == "control_slot_resolved"
        && stable_view_key(&from.view_id).contains("control-overview")
        && from.term.source() != to.term.source()
    {
        return false;
    }
    same_item_identity(from, to)
}

fn same_projection_identity(from: &SviViewItem, to: &SviViewItem) -> bool {
    let from = view_identity(from);
    let to = view_identity(to);
    from.domain == ViewDomain::ControlProjection
        && to.domain == ViewDomain::ControlProjection
        && from.stable_key == to.stable_key
}

fn view_identity(item: &SviViewItem) -> ViewIdentity<'_> {
    let stable_key = stable_view_key(&item.view_id);
    ViewIdentity {
        domain: item.domain,
        stable_key,
    }
}

fn stable_view_key(view_id: &str) -> &str {
    view_id
        .split_once('-')
        .map_or(view_id, |(_, suffix)| suffix)
}

fn collect_items<'a>(item: &'a SviViewItem, output: &mut Vec<&'a SviViewItem>) {
    output.push(item);
    let children = match &item.term {
        SviTerm::State { payload, .. } => payload,
        SviTerm::Group { items, .. } => items,
        SviTerm::Atom { .. } | SviTerm::Placeholder { .. } => return,
    };
    for child in children {
        collect_items(child, output);
    }
}

fn flat_items(scene: &SviScene) -> Vec<&SviViewItem> {
    let mut items = Vec::new();
    for item in scene.rows.iter().flat_map(|row| row.items.iter()) {
        collect_items(item, &mut items);
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{TraceInsertion, TraceRelation};

    #[test]
    fn builds_complete_frames_without_requiring_reader_replay() {
        let trace = SayoTrace::from_events_for_test(vec![
            TraceEvent::GroupStarted {
                group: 0,
                parent: None,
                cells: vec![0, 1],
                sources: vec!["add".into(), "5".into()],
                control: None,
            },
            TraceEvent::Rewritten {
                group: 0,
                removed_left: 0,
                removed_right: 1,
                inserted: vec![TraceInsertion {
                    cell: 2,
                    term: TraceTerm {
                        term_id: Some(7),
                        source: "Plus{5}".into(),
                        space: "Plus".into(),
                        implementation: "native".into(),
                        fragments: Vec::new(),
                    },
                }],
                relation: TraceRelation {
                    left_space: "Add".into(),
                    right_space: "Int".into(),
                    result_space: "Plus".into(),
                    executor: "native".into(),
                },
            },
        ]);
        let svi = SviDocument::build(&trace, "normal", vec!["Plus{5}".into()]);
        assert_eq!(svi.frames.len(), 2);
        assert_eq!(svi.frames[0].scene.rows[0].items.len(), 2);
        assert_eq!(
            svi.frames[1].scene.rows[0].items[0].term.source(),
            "Plus{5}"
        );
        assert!(!svi.frames[1].changes.is_empty());
    }
}
