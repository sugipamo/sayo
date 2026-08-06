use serde::{Serialize, Serializer};

use crate::model::{Term, TermId, TermImplementation};

pub type TraceGroupId = u64;
pub type TraceCellId = u64;
pub type TraceControlId = u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceTerm {
    pub term_id: Option<TermId>,
    pub source: String,
    pub space: String,
    pub implementation: String,
    /// Stable identities of payload terms embedded in a displayed state.
    /// `path` addresses zero-based payload positions, recursively.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<TraceFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceFragment {
    pub path: Vec<usize>,
    pub origin_id: TermId,
}

impl From<&Term> for TraceTerm {
    fn from(term: &Term) -> Self {
        let implementation = match &term.implementation {
            TermImplementation::None => "atom",
            TermImplementation::Sayo(_) => "sayo",
            TermImplementation::Native(_) => "native",
        };
        Self {
            term_id: term.term_id,
            source: term.display.clone(),
            space: term.space.clone(),
            implementation: implementation.into(),
            fragments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceRelation {
    pub left_space: String,
    pub right_space: String,
    pub result_space: String,
    pub executor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceControlGroup {
    pub control_id: TraceControlId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_cell: Option<TraceCellId>,
    pub transition: String,
    pub result_template: String,
    pub implementation_source: String,
    pub slots: Vec<String>,
    pub slot: usize,
    pub slot_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorReason {
    Start,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceInsertion {
    pub cell: TraceCellId,
    pub term: TraceTerm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TraceEvent {
    GroupStarted {
        group: TraceGroupId,
        parent: Option<TraceGroupId>,
        cells: Vec<TraceCellId>,
        sources: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        control: Option<TraceControlGroup>,
    },
    Materialized {
        group: TraceGroupId,
        cell: TraceCellId,
        term: TraceTerm,
    },
    Attempted {
        group: TraceGroupId,
        left_cell: TraceCellId,
        right_cell: TraceCellId,
        left: TraceTerm,
        right: TraceTerm,
    },
    NotApplicable {
        group: TraceGroupId,
        left_cell: TraceCellId,
        right_cell: TraceCellId,
        reason: String,
    },
    Rewritten {
        group: TraceGroupId,
        removed_left: TraceCellId,
        removed_right: TraceCellId,
        inserted: Vec<TraceInsertion>,
        relation: TraceRelation,
    },
    StateTransition {
        group: TraceGroupId,
        removed: TraceCellId,
        inserted: Vec<TraceInsertion>,
        #[serde(skip_serializing_if = "Option::is_none")]
        control_id: Option<TraceControlId>,
    },
    EndTransition {
        group: TraceGroupId,
        removed: TraceCellId,
        inserted: Vec<TraceInsertion>,
        #[serde(skip_serializing_if = "Option::is_none")]
        control_id: Option<TraceControlId>,
    },
    CursorMoved {
        group: TraceGroupId,
        from: Option<TraceCellId>,
        to: Option<TraceCellId>,
        reason: CursorReason,
    },
    Yielded {
        group: TraceGroupId,
        cell: TraceCellId,
        term: TraceTerm,
    },
    Trapped {
        group: Option<TraceGroupId>,
        category: String,
        message: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SayoTrace {
    events: Vec<TraceEvent>,
}

impl Serialize for SayoTrace {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SriDocument {
            format: "sayo-reduction-ir",
            version: 1,
            events: &self.events,
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
struct SriDocument<'a> {
    format: &'static str,
    version: u32,
    events: &'a [TraceEvent],
}

impl SayoTrace {
    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }

    #[cfg(test)]
    pub(crate) fn from_events_for_test(events: Vec<TraceEvent>) -> Self {
        Self { events }
    }

    pub fn render_json(&self) -> String {
        let document = SriDocument {
            format: "sayo-reduction-ir",
            version: 1,
            events: &self.events,
        };
        let mut output = serde_json::to_string_pretty(&document)
            .expect("SRI document contains only JSON-serializable values");
        output.push('\n');
        output
    }
}

pub(crate) struct TraceRecorder {
    trace: SayoTrace,
    next_group_id: TraceGroupId,
    next_cell_id: TraceCellId,
    next_control_id: TraceControlId,
    recorded_trap: bool,
    max_events: Option<usize>,
    overflowed: bool,
}

impl TraceRecorder {
    pub(crate) fn new() -> Self {
        Self::with_limit(None)
    }

    pub(crate) fn with_event_limit(max_events: usize) -> Self {
        Self::with_limit(Some(max_events))
    }

    fn with_limit(max_events: Option<usize>) -> Self {
        Self {
            trace: SayoTrace::default(),
            next_group_id: 0,
            next_cell_id: 0,
            next_control_id: 0,
            recorded_trap: false,
            max_events,
            overflowed: false,
        }
    }

    pub(crate) fn begin_group(
        &mut self,
        parent: Option<TraceGroupId>,
        sources: Vec<String>,
    ) -> (TraceGroupId, Vec<TraceCellId>) {
        let group = self.next_group_id;
        self.next_group_id += 1;
        let cells = (0..sources.len())
            .map(|_| self.new_cell())
            .collect::<Vec<_>>();
        self.record(TraceEvent::GroupStarted {
            group,
            parent,
            cells: cells.clone(),
            sources,
            control: None,
        });
        (group, cells)
    }

    pub(crate) fn begin_control_group(
        &mut self,
        parent: Option<TraceGroupId>,
        sources: Vec<String>,
        control: TraceControlGroup,
    ) -> (TraceGroupId, Vec<TraceCellId>) {
        let group = self.next_group_id;
        self.next_group_id += 1;
        let cells = (0..sources.len())
            .map(|_| self.new_cell())
            .collect::<Vec<_>>();
        self.record(TraceEvent::GroupStarted {
            group,
            parent,
            cells: cells.clone(),
            sources,
            control: Some(control),
        });
        (group, cells)
    }

    pub(crate) fn new_cell(&mut self) -> TraceCellId {
        let cell = self.next_cell_id;
        self.next_cell_id += 1;
        cell
    }

    pub(crate) fn new_control(&mut self) -> TraceControlId {
        let control = self.next_control_id;
        self.next_control_id += 1;
        control
    }

    pub(crate) fn record(&mut self, event: TraceEvent) {
        if self
            .max_events
            .is_some_and(|limit| self.trace.events.len() >= limit)
        {
            self.overflowed = true;
            return;
        }
        if matches!(event, TraceEvent::Trapped { .. }) {
            self.recorded_trap = true;
        }
        self.trace.events.push(event);
    }

    pub(crate) fn has_recorded_trap(&self) -> bool {
        self.recorded_trap
    }

    pub(crate) fn overflowed(&self) -> bool {
        self.overflowed
    }

    pub(crate) fn finish(self) -> SayoTrace {
        self.trace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_versioned_single_json_document() {
        let trace = SayoTrace {
            events: vec![TraceEvent::Rewritten {
                group: 2,
                removed_left: 4,
                removed_right: 5,
                inserted: vec![TraceInsertion {
                    cell: 6,
                    term: TraceTerm {
                        term_id: Some(9),
                        source: "line\n\"quoted\"".into(),
                        space: "String".into(),
                        implementation: "atom".into(),
                        fragments: Vec::new(),
                    },
                }],
                relation: TraceRelation {
                    left_space: "Left".into(),
                    right_space: "Right".into(),
                    result_space: "Result".into(),
                    executor: "sayo".into(),
                },
            }],
        };

        let output = trace.render_json();
        let document: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(document["format"], "sayo-reduction-ir");
        assert_eq!(document["version"], 1);
        assert_eq!(document["events"][0]["event"], "rewritten");
        assert_eq!(document["events"][0]["group"], 2);
        assert_eq!(document["events"][0]["inserted"][0]["cell"], 6);
        assert_eq!(
            document["events"][0]["inserted"][0]["term"]["source"],
            "line\n\"quoted\""
        );
        assert!(output.ends_with('\n'));
    }
}
