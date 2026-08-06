use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};
use crate::model::{Expression, NativeTermId, NormalForm, Term, TermId, TermImplementation};
use crate::parser;
use crate::standard_world::embedded_standard_world;
use crate::trace::{
    SayoTrace, TraceCellId, TraceControlGroup, TraceControlId, TraceEvent, TraceFragment,
    TraceGroupId, TraceInsertion, TraceRecorder, TraceRelation, TraceTerm,
};
use crate::world::{FsWorld, MemoryWorld, VirtualPath, WorldHandle, WorldPath, WorldView};

pub struct Runtime {
    world: VirtualPath,
    standard_world: VirtualPath,
    limits: RuntimeLimits,
    terms: Mutex<HashMap<u64, Term>>,
    next_term_id: AtomicU64,
    native_terms: Mutex<HashMap<NativeTermId, NativeTermState>>,
    template_arities: HashMap<String, usize>,
    inactive_standard_components: HashSet<String>,
    next_native_term_id: AtomicU64,
    native_registry: NativeRegistry,
}

/// Bootstrap resource bounds for one Runtime.
///
/// These protect the initial Rust implementation only; they are not
/// restrictions in Sayo's semantics.  A future long-lived SayoVM can choose
/// different bounds or no bounds at all.
#[derive(Debug, Clone)]
pub struct RuntimeLimits {
    pub relation_depth: usize,
    pub reduction_steps: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            relation_depth: 256,
            reduction_steps: 100_000,
        }
    }
}

#[derive(Clone, Copy)]
struct RelationContext {
    input_term_id: u64,
    depth: usize,
    trace_parent_group: Option<TraceGroupId>,
    state_native_id: Option<NativeTermId>,
    schema: bool,
    space_checker: bool,
}

/// The result and reduction trace of one evaluation.
///
/// A trace is intentionally returned even when evaluation traps.  This makes
/// the event stream useful to both a textual debugger and a future visual
/// reducer without changing normal evaluation semantics.
pub struct TracedEvaluation {
    pub result: Result<NormalForm>,
    pub trace: SayoTrace,
}

/// Evaluation-local trace state.
///
/// `group` is the application group currently being reduced.  A nested
/// application uses it as its parent when it allocates its own group.
struct TraceContext<'a> {
    recorder: &'a mut Option<TraceRecorder>,
    group: Option<TraceGroupId>,
}

#[derive(Clone)]
struct Relation {
    result_space: String,
    implementation: RelationImplementation,
    priority: u8,
}

impl Relation {
    fn implementation_label(&self) -> &'static str {
        match &self.implementation {
            RelationImplementation::Capture => "capture",
            RelationImplementation::Sayo(_) => "sayo",
            RelationImplementation::Native(_) => "native",
        }
    }

    fn trace(&self, left: &Term, right: &Term) -> TraceRelation {
        let executor = match &self.implementation {
            RelationImplementation::Capture => "capture",
            RelationImplementation::Native(_) => "native",
            RelationImplementation::Sayo(_) => match &left.implementation {
                TermImplementation::Sayo(_) => "sayo",
                _ => self.implementation_label(),
            },
        };
        TraceRelation {
            left_space: left.space.clone(),
            right_space: right.space.clone(),
            result_space: self.result_space.clone(),
            executor: executor.into(),
        }
    }
}

#[derive(Clone)]
enum RelationImplementation {
    Capture,
    Sayo(VirtualPath),
    Native(NativeRelationId),
}

type NativeRelationId = u64;

pub(crate) const NATIVE_PRIMITIVE_SPACES: &[&str] = &[
    "Int",
    "Add",
    "Sub",
    "Mul",
    "Div",
    "Compare",
    "EqualString",
    "EqualTo",
    "Duplicate",
    "For",
    "Filter",
    "Odd",
    "Fold",
    "Reduce",
    "Keep",
    "Drop",
];

#[derive(Clone, Debug, Eq, PartialEq)]
enum SourcePath {
    Host(PathBuf),
    World(VirtualPath),
}

impl SourcePath {
    fn host(path: impl Into<PathBuf>) -> Self {
        Self::Host(path.into())
    }

    fn world(path: VirtualPath) -> Self {
        Self::World(path)
    }

    fn join(&self, reference: impl AsRef<Path>) -> Result<Self> {
        let reference = reference.as_ref();
        match self {
            Self::Host(base) => Ok(Self::Host(if reference.is_absolute() {
                reference.to_path_buf()
            } else {
                base.join(reference)
            })),
            Self::World(base) => base.resolve(reference).map(Self::World),
        }
    }

    fn parent(&self) -> Option<Self> {
        match self {
            Self::Host(path) => path.parent().map(|path| Self::Host(path.to_path_buf())),
            Self::World(path) => path.parent().map(Self::World),
        }
    }

    fn read(&self) -> Result<String> {
        match self {
            Self::Host(path) => fs::read_to_string(path).map_err(Error::Io),
            Self::World(path) => path.read()?.ok_or_else(|| {
                Error::Source(format!("cannot read Sayo source {}", path.display()))
            }),
        }
    }

    fn is_file(&self) -> Result<bool> {
        match self {
            Self::Host(path) => Ok(path.is_file()),
            Self::World(path) => path.is_file(),
        }
    }

    fn canonicalize(&self) -> Result<Self> {
        match self {
            Self::Host(path) => fs::canonicalize(path).map(Self::Host).map_err(Error::Io),
            Self::World(path) => Ok(Self::World(path.clone())),
        }
    }

    fn display(&self) -> String {
        match self {
            Self::Host(path) => path.display().to_string(),
            Self::World(path) => path.display().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum NativeSpaceMatcher {
    Exact(&'static str),
    I64,
    GeneratedOnly,
}

#[derive(Debug)]
struct NativeSpace {
    name: &'static str,
    matcher: NativeSpaceMatcher,
}

impl NativeSpace {
    fn matches(&self, source: &str) -> bool {
        match self.matcher {
            NativeSpaceMatcher::Exact(expected) => source == expected,
            NativeSpaceMatcher::I64 => source.parse::<i64>().is_ok(),
            NativeSpaceMatcher::GeneratedOnly => false,
        }
    }
}

#[derive(Debug)]
struct NativeRelation {
    id: NativeRelationId,
    left_space: &'static str,
    right_space: &'static str,
    result_space: &'static str,
    handler: NativeHandler,
}

struct NativeRegistry {
    spaces: Vec<NativeSpace>,
    relations: Vec<NativeRelation>,
    next_id: NativeRelationId,
}

impl NativeRegistry {
    fn bootstrap() -> Self {
        let mut registry = Self {
            spaces: Vec::new(),
            relations: Vec::new(),
            next_id: 1,
        };
        register_native_spaces(&mut registry);
        register_integer_primitives(&mut registry);
        register_transition_primitives(&mut registry);
        registry
    }

    fn register_space(&mut self, name: &'static str, matcher: NativeSpaceMatcher) {
        assert!(
            !self.spaces.iter().any(|space| space.name == name),
            "bootstrap Native Space must be unique: {name}"
        );
        self.spaces.push(NativeSpace { name, matcher });
    }

    fn spaces_for<'a>(&'a self, source: &'a str) -> impl Iterator<Item = &'static str> + 'a {
        self.spaces
            .iter()
            .filter(move |space| space.matches(source))
            .map(|space| space.name)
    }

    fn has_space(&self, name: &str) -> bool {
        self.spaces.iter().any(|space| space.name == name)
    }

    fn register(
        &mut self,
        left_space: &'static str,
        right_space: &'static str,
        result_space: &'static str,
        handler: NativeHandler,
    ) -> Result<NativeRelationId> {
        if self.relations.iter().any(|relation| {
            relation.left_space == left_space
                && relation.right_space == right_space
                && relation.result_space == result_space
        }) {
            return Err(Error::Implementation(format!(
                "duplicate Native Relation: {left_space} {right_space} : {result_space}"
            )));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.relations.push(NativeRelation {
            id,
            left_space,
            right_space,
            result_space,
            handler,
        });
        Ok(id)
    }

    fn candidates(&self, left: &str, right: &str) -> impl Iterator<Item = &NativeRelation> {
        self.relations.iter().filter(move |relation| {
            relation.left_space == left
                && (relation.right_space == right || relation.right_space == "*")
        })
    }

    fn find(&self, id: NativeRelationId) -> Option<&NativeRelation> {
        self.relations.iter().find(|relation| relation.id == id)
    }
}

fn register_native_spaces(registry: &mut NativeRegistry) {
    registry.register_space("Int", NativeSpaceMatcher::I64);
    for &(space, atom) in &[
        ("Add", "add"),
        ("Sub", "sub"),
        ("Mul", "mul"),
        ("Div", "div"),
        ("Compare", "compare"),
        ("EqualString", "equal-string"),
        ("EqualTo", "equal-to"),
        ("Duplicate", "duplicate"),
        ("For", "for"),
        ("Filter", "filter"),
        ("Odd", "odd"),
        ("Fold", "fold"),
        ("Reduce", "reduce"),
        ("Keep", "keep"),
        ("Drop", "drop"),
    ] {
        registry.register_space(space, NativeSpaceMatcher::Exact(atom));
    }
    for space in [
        "Plus",
        "Minus",
        "Times",
        "Quotient",
        "CompareInt",
        "CompareString",
        "EqualStringExpected",
        "Ordering",
    ] {
        registry.register_space(space, NativeSpaceMatcher::GeneratedOnly);
    }
    debug_assert!(
        NATIVE_PRIMITIVE_SPACES
            .iter()
            .all(|name| registry.spaces.iter().any(|space| space.name == *name))
    );
}

fn register_integer_primitives(registry: &mut NativeRegistry) {
    let primitives: &[(&str, &str, NativeHandler, NativeHandler)] = &[
        ("Add", "Plus", native_add_int, native_plus_int),
        ("Sub", "Minus", native_sub_int, native_minus_int),
        ("Mul", "Times", native_mul_int, native_times_int),
        ("Div", "Quotient", native_div_int, native_quotient_int),
    ];
    for &(left, intermediate, capture, apply) in primitives {
        registry
            .register(left, "Int", intermediate, capture)
            .expect("bootstrap Native Relation must be unique");
        registry
            .register(intermediate, "Int", "Int", apply)
            .expect("bootstrap Native Relation must be unique");
    }
    registry
        .register("Compare", "Int", "CompareInt", native_compare_capture_int)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register("CompareInt", "Int", "Ordering", native_compare_apply_int)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register(
            "Compare",
            "String",
            "CompareString",
            native_compare_capture_string,
        )
        .expect("bootstrap Native Relation must be unique");
    registry
        .register(
            "CompareString",
            "String",
            "Ordering",
            native_compare_apply_string,
        )
        .expect("bootstrap Native Relation must be unique");
    registry
        .register(
            "EqualString",
            "String",
            "EqualStringExpected",
            native_equal_string_capture,
        )
        .expect("bootstrap Native Relation must be unique");
    registry
        .register(
            "EqualStringExpected",
            "String",
            ".any",
            native_equal_string_apply,
        )
        .expect("bootstrap Native Relation must be unique");
}

/// A deliberately small reference primitive for the state-transition
/// contract. `Duplicate Int : Duplicate` first creates an internal emitter;
/// the emitter then writes two values to its left and leaves `Duplicate` as
/// the continuation anchor.
fn register_transition_primitives(registry: &mut NativeRegistry) {
    registry
        .register("Duplicate", "Int", "Duplicate", native_duplicate_int)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register("For", "*", "Each", native_for_transform)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register("Each", "Int", "Each", native_each_int)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register("Filter", "*", "Sieve", native_filter_transform)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register("Sieve", "*", "Sieve", native_sieve_input)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register("Odd", "Int", ".any", native_odd_int)
        .expect("bootstrap Native Relation must be unique");
    registry
        .register("Fold", "*", "FoldInit", native_fold_transform)
        .expect("native");
    registry
        .register("FoldInit", "*", "FoldAccum", native_fold_input)
        .expect("native");
    registry
        .register("FoldAccum", "*", "FoldAccum", native_fold_input)
        .expect("native");
    registry
        .register("Reduce", "*", "ReduceInit", native_reduce_transform)
        .expect("native");
    registry
        .register("ReduceInit", "*", "ReduceAccum", native_reduce_input)
        .expect("native");
    registry
        .register("ReduceAccum", "*", "ReduceAccum", native_reduce_input)
        .expect("native");
}

type NativeHandler = fn(&mut NativeContext<'_>, TermId) -> Result<NativeOutcome>;

#[allow(dead_code)]
enum NativeOutcome {
    Success(TermId),
    NotApplicable,
    Trap(Error),
}

#[derive(Clone)]
enum NativeTermState {
    Arithmetic {
        captured: TermId,
        operation: ArithmeticOperation,
    },
    Comparison {
        captured: TermId,
    },
    StringComparison {
        captured: TermId,
    },
    StringEquality {
        expected: TermId,
    },
    Ordering,
    EmitTwice {
        captured: TermId,
    },
    EmitOnce {
        captured: TermId,
    },
    Each {
        transform: TermId,
    },
    EachApply {
        transform: TermId,
        input: TermId,
    },
    EachEmit {
        transform: TermId,
        value: TermId,
    },
    Sieve {
        predicate: TermId,
    },
    SieveApply {
        predicate: TermId,
        input: TermId,
    },
    SieveKeep {
        predicate: TermId,
        input: TermId,
    },
    SieveDrop {
        predicate: TermId,
    },
    FilterChoice(FilterChoice),
    FoldInit {
        transform: TermId,
    },
    FoldAccum {
        transform: TermId,
        accumulator: TermId,
    },
    FoldApply {
        transform: TermId,
        accumulator: TermId,
        input: TermId,
    },
    FoldNext {
        transform: TermId,
        accumulator: TermId,
    },
    FoldClose {
        accumulator: TermId,
    },
    ReduceInit {
        transform: TermId,
    },
    ReduceAccum {
        transform: TermId,
        accumulator: TermId,
    },
    ReduceApply {
        transform: TermId,
        accumulator: TermId,
        input: TermId,
    },
    ReduceNext {
        transform: TermId,
        accumulator: TermId,
    },
    ReduceClose {
        accumulator: TermId,
    },
    /// Generic payload-backed Template state.  Template-specific transitions
    /// are added later; the bootstrap runtime can already materialize and
    /// retain the immutable payload.
    Template {
        template: String,
        payload: Vec<TermId>,
        schema: bool,
        space_checker: bool,
    },
    #[cfg(test)]
    SelfLoop,
}

#[derive(Clone, Copy)]
enum ArithmeticOperation {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy)]
enum FilterChoice {
    Keep,
    Drop,
}

#[derive(Clone, Copy)]
enum OrderingKind {
    Less,
    Equal,
    Greater,
}

struct NativeContext<'a> {
    runtime: &'a Runtime,
    depth: usize,
    native_term_id: Option<NativeTermId>,
    space_checker: bool,
}

enum RelationOutcome {
    Success(RelationResult),
    NotApplicable,
    Trap(Error),
}

enum SpaceCheckOutcome {
    Match,
    Miss,
}

enum RelationResult {
    Term(TermId),
    Terms(Vec<Term>),
}

/// The scheduler-level result of trying to reduce two adjacent Terms.
/// The Runtime owns the leftmost scheduling rule.
enum ReducerOutcome {
    Success {
        terms: Vec<Term>,
        relation: TraceRelation,
    },
    NoStep(String),
    Trap(Error),
}

/// The common contract for a Runtime-owned state transition.
///
/// State implementations return a replacement sequence; only the reducer
/// splices that sequence into the current normal-form candidate.
enum NativeStateTransition {
    Replace(Vec<Term>),
    NoStep,
    Trap(Error),
}

struct ReductionCell {
    expression: Option<Expression>,
    term: Option<Term>,
    trace_cell: Option<TraceCellId>,
}

struct SayoPairCall {
    left: Term,
    right: Term,
    relation: Relation,
    implementation: VirtualPath,
    context: RelationContext,
}

enum PreparedPair {
    Ready(ReducerOutcome),
    Sayo(Box<SayoPairCall>),
}

struct PendingPairApplication {
    cursor: usize,
    left_cell: Option<TraceCellId>,
    right_cell: Option<TraceCellId>,
    call: SayoPairCall,
}

#[derive(Clone, Copy)]
enum ControlTransitionKind {
    Tick,
    End,
}

impl ControlTransitionKind {
    fn trace_name(self) -> &'static str {
        match self {
            Self::Tick => "tick",
            Self::End => "end",
        }
    }
}

struct SayoControlCall {
    implementation: VirtualPath,
    base: SourcePath,
    context: RelationContext,
    payload: Vec<TermId>,
    used: HashSet<usize>,
    result_template: String,
    zero_state_transition: bool,
    slots: Vec<Expression>,
    depth: usize,
}

enum PreparedControl {
    Ready(NativeStateTransition),
    Sayo(Box<SayoControlCall>),
}

struct PendingControlApplication {
    cursor: usize,
    removed_cell: Option<TraceCellId>,
    kind: ControlTransitionKind,
    call: SayoControlCall,
    next_slot: usize,
    next_payload: Vec<Term>,
    finish_group_end: bool,
    trace_control: Option<TraceControlId>,
}

enum PendingApplication {
    Pair(PendingPairApplication),
    Control(PendingControlApplication),
}

struct ReductionFrame {
    cells: Vec<ReductionCell>,
    base: SourcePath,
    source_stack: Vec<SourcePath>,
    context: Option<RelationContext>,
    expression_depth: usize,
    group: Option<TraceGroupId>,
    steps: usize,
    resume_cursor: usize,
    pending: Option<PendingApplication>,
    failure: Option<Error>,
}

enum FrameAction {
    Complete(Result<NormalForm>),
    Call(PendingPairApplication),
    Control(PendingControlApplication),
}

impl ReductionCell {
    fn expression(expression: Expression, trace_cell: Option<TraceCellId>) -> Self {
        Self {
            expression: Some(expression),
            term: None,
            trace_cell,
        }
    }

    fn term(term: Term, trace_cell: Option<TraceCellId>) -> Self {
        Self {
            expression: None,
            term: Some(term),
            trace_cell,
        }
    }
}

impl Runtime {
    /// Construct a Runtime over a logically immutable World.
    ///
    /// The Runtime does not copy the World. The host must not mutate the
    /// World directory or referenced Sayo files for this Runtime's lifetime.
    /// Concurrent mutation is outside Sayo's evaluation semantics.
    pub fn new(world: impl AsRef<Path>) -> Result<Self> {
        Self::new_with_limits(world, RuntimeLimits::default())
    }

    pub fn new_with_limits(world: impl AsRef<Path>, limits: RuntimeLimits) -> Result<Self> {
        let fs_world = FsWorld::open(world)?;
        let user_snapshot = MemoryWorld::snapshot(&fs_world)?;
        Self::from_world_snapshots(user_snapshot, embedded_standard_world()?, limits)
    }

    /// Construct a Runtime from an owned browser-compatible user World.
    /// The embedded standard World remains a separate lower-priority layer.
    pub fn from_memory_world(world: MemoryWorld, limits: RuntimeLimits) -> Result<Self> {
        Self::from_world_snapshots(world, embedded_standard_world()?, limits)
    }

    fn from_world_snapshots(
        user_world: MemoryWorld,
        standard_world: MemoryWorld,
        limits: RuntimeLimits,
    ) -> Result<Self> {
        for entry in user_world.directories(&WorldPath::root())? {
            match entry.file_name() {
                Some("Trap") => {
                    return Err(Error::World(
                        "Template name `Trap` is reserved for the SayoVM semantic trap".into(),
                    ));
                }
                Some("Vanish") => {
                    return Err(Error::World(
                        "Template name `Vanish` is reserved for SayoVM state removal".into(),
                    ));
                }
                Some("Miss") => {
                    return Err(Error::World(
                        "Space name `Miss` is reserved for SayoVM Space Checker control".into(),
                    ));
                }
                _ => {}
            }
        }
        let user_handle = WorldHandle::new(user_world);
        let standard_handle = WorldHandle::new(standard_world);
        let world = user_handle.root();
        let standard_world = standard_handle.root();
        let arity_analysis =
            crate::arity::analyze_virtual(&[world.clone(), standard_world.clone()])?;
        let inactive_standard_components = arity_analysis.inactive_lower_components().clone();
        Ok(Self {
            world,
            standard_world,
            limits,
            terms: Mutex::new(HashMap::new()),
            next_term_id: AtomicU64::new(1),
            native_terms: Mutex::new(HashMap::new()),
            template_arities: arity_analysis.into_arities(),
            inactive_standard_components,
            next_native_term_id: AtomicU64::new(1),
            native_registry: NativeRegistry::bootstrap(),
        })
    }

    pub fn limits(&self) -> &RuntimeLimits {
        &self.limits
    }

    fn world_roots(&self) -> [&VirtualPath; 2] {
        [&self.world, &self.standard_world]
    }

    fn visible_component(&self, name: &str) -> Option<(usize, VirtualPath)> {
        self.world_roots()
            .into_iter()
            .enumerate()
            .filter_map(|(index, root)| root.join(name).ok().map(|root| (index, root)))
            .find(|(index, component)| {
                component.is_dir().unwrap_or(false)
                    && (*index == 0 || !self.inactive_standard_components.contains(name))
            })
    }

    fn component_roots(&self, name: &str) -> Vec<VirtualPath> {
        self.visible_component(name)
            .map(|(_, component)| vec![component])
            .unwrap_or_default()
    }

    fn component_root(&self, name: &str) -> Option<VirtualPath> {
        self.component_roots(name).into_iter().next()
    }

    pub fn evaluate(&self, source: &str) -> Result<NormalForm> {
        let base = SourcePath::host(std::env::current_dir()?);
        let mut stack = Vec::new();
        let mut recorder = None;
        let mut trace = TraceContext {
            recorder: &mut recorder,
            group: None,
        };
        self.evaluate_source(source, &base, &mut stack, None, &mut trace)
    }

    /// Render a normal form for users.  This is intentionally distinct from
    /// `Term::source`: native state has no Sayo source but still has a useful
    /// structural display form such as `Plus{5}`.
    pub fn display_normal_form(&self, normal_form: &NormalForm) -> Result<String> {
        Ok(normal_form
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>()
            .join(" "))
    }

    pub fn evaluate_file(&self, path: impl AsRef<Path>) -> Result<NormalForm> {
        let path = path.as_ref();
        let mut stack = Vec::new();
        let mut recorder = None;
        let mut trace = TraceContext {
            recorder: &mut recorder,
            group: None,
        };
        self.evaluate_file_path_with_context(path, &mut stack, None, &mut trace)
    }

    pub fn evaluate_with_trace(&self, source: &str) -> TracedEvaluation {
        let mut recorder = Some(TraceRecorder::new());
        let result = {
            let mut trace = TraceContext {
                recorder: &mut recorder,
                group: None,
            };
            match std::env::current_dir() {
                Ok(base) => {
                    let base = SourcePath::host(base);
                    let mut stack = Vec::new();
                    self.evaluate_source(source, &base, &mut stack, None, &mut trace)
                }
                Err(error) => Err(Error::Io(error)),
            }
        };
        Self::finish_traced_evaluation(recorder.take().expect("trace recorder is present"), result)
    }

    pub fn evaluate_file_with_trace(&self, path: impl AsRef<Path>) -> TracedEvaluation {
        let mut recorder = Some(TraceRecorder::new());
        let mut stack = Vec::new();
        let result = {
            let mut trace = TraceContext {
                recorder: &mut recorder,
                group: None,
            };
            self.evaluate_file_path_with_context(path.as_ref(), &mut stack, None, &mut trace)
        };
        Self::finish_traced_evaluation(recorder.take().expect("trace recorder is present"), result)
    }

    /// Evaluate a source file owned by this Runtime's immutable user World.
    pub fn evaluate_world_file(&self, path: &WorldPath) -> Result<NormalForm> {
        let path = self.world.join(path.as_str())?;
        let mut recorder = None;
        let mut trace = TraceContext {
            recorder: &mut recorder,
            group: None,
        };
        self.evaluate_virtual_file_with_context(&path, None, &mut trace)
    }

    /// Browser-oriented traced evaluation with a hard cap on retained trace
    /// events. Overflow is reported as a host bootstrap limit, not a semantic
    /// Sayo Trap.
    pub fn evaluate_world_file_with_trace(
        &self,
        path: &WorldPath,
        max_trace_events: usize,
    ) -> TracedEvaluation {
        let mut recorder = Some(TraceRecorder::with_event_limit(max_trace_events));
        let result = match self.world.join(path.as_str()) {
            Ok(path) => {
                let mut trace = TraceContext {
                    recorder: &mut recorder,
                    group: None,
                };
                self.evaluate_virtual_file_with_context(&path, None, &mut trace)
            }
            Err(error) => Err(error),
        };
        let mut recorder = recorder.take().expect("trace recorder is present");
        let result = if recorder.overflowed() {
            Err(Error::BootstrapLimit(format!(
                "trace event limit exceeded: limit={max_trace_events}"
            )))
        } else {
            result
        };
        if let Err(error) = &result
            && !recorder.has_recorded_trap()
        {
            recorder.record(TraceEvent::Trapped {
                group: None,
                category: error.category().into(),
                message: error.to_string(),
            });
        }
        TracedEvaluation {
            result,
            trace: recorder.finish(),
        }
    }

    fn evaluate_virtual_file_with_context(
        &self,
        path: &VirtualPath,
        context: Option<RelationContext>,
        trace: &mut TraceContext<'_>,
    ) -> Result<NormalForm> {
        if !path.is_file()? {
            return Err(Error::Source(format!(
                "Sayo source is not a file: {}",
                path.display()
            )));
        }
        let source = path
            .read()?
            .ok_or_else(|| Error::Source(format!("cannot read Sayo source {}", path.display())))?;
        let source_path = SourcePath::world(path.clone());
        let base = source_path
            .parent()
            .unwrap_or_else(|| SourcePath::world(self.world.clone()));
        let mut stack = vec![source_path];
        self.evaluate_source(&source, &base, &mut stack, context, trace)
    }

    fn finish_traced_evaluation(
        mut recorder: TraceRecorder,
        result: Result<NormalForm>,
    ) -> TracedEvaluation {
        if let Err(error) = &result
            && !recorder.has_recorded_trap()
        {
            recorder.record(TraceEvent::Trapped {
                group: None,
                category: error.category().into(),
                message: error.to_string(),
            });
        }
        TracedEvaluation {
            result,
            trace: recorder.finish(),
        }
    }

    fn evaluate_source(
        &self,
        source: &str,
        base: &SourcePath,
        stack: &mut [SourcePath],
        context: Option<RelationContext>,
        trace: &mut TraceContext<'_>,
    ) -> Result<NormalForm> {
        let expression = parser::parse(source)?;
        self.evaluate_application_group(&expression, base, stack, context, 0, trace)
    }

    fn evaluate_file_path_with_context(
        &self,
        path: &Path,
        stack: &mut Vec<SourcePath>,
        context: Option<RelationContext>,
        trace: &mut TraceContext<'_>,
    ) -> Result<NormalForm> {
        let requested = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let path = fs::canonicalize(&requested).map_err(|error| {
            Error::Source(format!(
                "cannot read Sayo source {}: {error}",
                requested.display()
            ))
        })?;
        if !path.is_file() {
            return Err(Error::Source(format!(
                "Sayo source is not a file: {}",
                path.display()
            )));
        }
        let path = SourcePath::host(path);
        if stack.iter().any(|item| item == &path) {
            return Err(Error::Source(format!(
                "cyclic Sayo source reference: {}",
                path.display()
            )));
        }
        let source = path.read()?;
        stack.push(path.clone());
        let base = path
            .parent()
            .unwrap_or_else(|| SourcePath::world(self.world.clone()));
        let result = self.evaluate_source(&source, &base, stack, context, trace);
        stack.pop();
        result
    }

    fn evaluate_expression(
        &self,
        expression: &Expression,
        base: &SourcePath,
        stack: &mut Vec<SourcePath>,
        context: Option<RelationContext>,
        expression_depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<NormalForm> {
        if expression_depth > self.limits.relation_depth {
            return Err(Error::BootstrapLimit(format!(
                "evaluation depth exceeded: limit={}",
                self.limits.relation_depth
            )));
        }
        if matches!(expression, Expression::Apply(_, _)) {
            return self.evaluate_application_group(
                expression,
                base,
                stack,
                context,
                expression_depth,
                trace,
            );
        }
        match expression {
            Expression::StringLiteral(value) => {
                let rendered = render_string_literal(value);
                self.store_term(Term {
                    term_id: None,
                    evaluation_depth: expression_depth,
                    source: rendered.clone(),
                    display: rendered,
                    space: "String".into(),
                    implementation: TermImplementation::None,
                })
                .map(NormalForm::one)
            }
            Expression::State { template, payload } => {
                if matches!(template.as_str(), "Trap" | "Vanish" | "Miss") {
                    return Err(Error::Implementation(format!(
                        "Template `{template}` is reserved and cannot be constructed directly"
                    )));
                }
                let schema_internal = context.is_some_and(|context| context.schema)
                    && matches!(template.as_str(), "Pass" | "CheckSpace");
                if !schema_internal && self.component_root(template).is_none() {
                    return Err(Error::Membership(format!(
                        "state constructor refers to unknown Template `{template}`"
                    )));
                }
                let payload = self.evaluate_payload(
                    payload,
                    base,
                    stack,
                    context,
                    expression_depth + 1,
                    trace,
                )?;
                self.register_template_state(template, payload, expression_depth, context, trace)
                    .map(NormalForm::one)
            }
            Expression::PayloadRef(index) => {
                let context = context.ok_or_else(|| {
                    Error::Implementation(
                        "payload references are only valid in Template relations".into(),
                    )
                })?;
                let native_id = context.state_native_id.ok_or_else(|| {
                    Error::Implementation("payload reference has no active Template state".into())
                })?;
                let term_id = self.template_payload_term(native_id, *index)?;
                self.term_for_id(term_id).map(NormalForm::one)
            }
            Expression::Atom(source) => {
                if source.ends_with(".sayo") {
                    if context.is_some_and(|context| context.schema || context.space_checker) {
                        return Err(Error::Implementation(
                            "restricted evaluation cannot access external Sayo files".into(),
                        ));
                    }
                    let path = base.join(Path::new(source))?.canonicalize()?;
                    if !path.is_file()? {
                        return Err(Error::Source(format!(
                            "Sayo source is not a file: {}",
                            path.display()
                        )));
                    }
                    if stack.iter().any(|item| item == &path) {
                        return Err(Error::Source(format!(
                            "cyclic Sayo source reference: {}",
                            path.display()
                        )));
                    }
                    let source = path.read()?;
                    stack.push(path.clone());
                    let next_base = path.parent().unwrap_or_else(|| base.clone());
                    let result = self.evaluate_source(&source, &next_base, stack, context, trace);
                    stack.pop();
                    return result;
                }
                if source == "Trap" {
                    return Err(Error::Implementation("semantic Trap".into()));
                }
                if source == "Miss" {
                    if !context.is_some_and(|context| context.space_checker) {
                        return Err(Error::Implementation(
                            "Miss is only valid as a Space Checker outcome".into(),
                        ));
                    }
                    return self
                        .store_term(Term {
                            term_id: None,
                            evaluation_depth: expression_depth,
                            source: "Miss".into(),
                            display: "Miss".into(),
                            space: "__Miss".into(),
                            implementation: TermImplementation::None,
                        })
                        .map(NormalForm::one);
                }
                // Inside a Template schema, a known Space name is a literal
                // reference rather than a value to classify through its
                // checker. This makes `CheckSpace{%0, Int}` express the
                // intended predicate without requiring a separate syntax.
                if context.is_some_and(|context| context.schema)
                    && (self.component_roots(source).iter().any(|root| {
                        root.join("space.sayo")
                            .and_then(|path| path.is_file())
                            .unwrap_or(false)
                    }) || self.native_registry.has_space(source))
                {
                    return self
                        .store_term(Term {
                            term_id: None,
                            evaluation_depth: expression_depth,
                            source: source.clone(),
                            display: source.clone(),
                            space: source.clone(),
                            implementation: TermImplementation::None,
                        })
                        .map(NormalForm::one);
                }
                let mut spaces = self.spaces_for(
                    source,
                    stack,
                    trace,
                    context.is_some_and(|context| context.space_checker),
                )?;
                if spaces.is_empty() {
                    spaces.extend(self.native_registry.spaces_for(source).map(str::to_owned));
                }
                match spaces.as_slice() {
                    [] => Err(Error::Membership(format!(
                        "term {source:?} does not belong to any Space"
                    ))),
                    [space] => self
                        .store_term(Term::atom(source.clone(), space.clone()))
                        .map(NormalForm::one),
                    _ => Err(Error::Membership(format!(
                        "term {source:?} belongs to multiple Spaces: {}",
                        spaces.join(", ")
                    ))),
                }
            }
            Expression::Group(inner) => {
                let normal = self.evaluate_expression(
                    inner,
                    base,
                    stack,
                    context,
                    expression_depth + 1,
                    trace,
                )?;
                self.finish_group_end(normal, expression_depth, trace)
            }
            Expression::Apply(_, _) => unreachable!("application groups are handled above"),
        }
    }

    fn finish_group_end(
        &self,
        normal: NormalForm,
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<NormalForm> {
        let mut terms = normal.into_terms();
        let Some(last) = terms.last().cloned() else {
            return Ok(NormalForm::new(terms));
        };
        match self.transition_native_end(&last, depth, trace) {
            NativeStateTransition::Replace(replacement) => {
                terms.pop();
                terms.extend(replacement);
            }
            NativeStateTransition::NoStep => {}
            NativeStateTransition::Trap(error) => return Err(error),
        }
        Ok(NormalForm::new(terms))
    }

    fn evaluate_payload(
        &self,
        payload: &[Expression],
        base: &SourcePath,
        stack: &mut Vec<SourcePath>,
        context: Option<RelationContext>,
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<Vec<Term>> {
        let mut terms = Vec::new();
        for expression in payload {
            let result =
                self.evaluate_expression(expression, base, stack, context, depth, trace)?;
            let mut result = result.into_terms();
            if result.len() != 1 {
                return Err(Error::Implementation(
                    "each Template payload expression must produce exactly one Term".into(),
                ));
            }
            terms.push(result.pop().expect("payload length checked"));
        }
        Ok(terms)
    }

    fn evaluate_application_group(
        &self,
        expression: &Expression,
        base: &SourcePath,
        stack: &mut [SourcePath],
        context: Option<RelationContext>,
        expression_depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<NormalForm> {
        let expressions = application_terms(expression);
        let (group, trace_cells) = match trace.recorder.as_mut() {
            Some(recorder) => {
                let sources = expressions.iter().map(Expression::render_source).collect();
                let (group, cells) = recorder.begin_group(trace.group, sources);
                (Some(group), cells.into_iter().map(Some).collect())
            }
            None => (None, vec![None; expressions.len()]),
        };
        let mut cells = expressions
            .into_iter()
            .zip(trace_cells)
            .map(|(expression, trace_cell)| ReductionCell::expression(expression, trace_cell))
            .collect::<Vec<_>>();
        let mut group_trace = TraceContext {
            recorder: &mut *trace.recorder,
            group,
        };
        self.reduce_application_group(
            &mut cells,
            base,
            stack,
            context,
            expression_depth,
            &mut group_trace,
        )
    }

    fn reduce_application_group(
        &self,
        cells: &mut Vec<ReductionCell>,
        base: &SourcePath,
        stack: &mut [SourcePath],
        context: Option<RelationContext>,
        expression_depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<NormalForm> {
        let root = ReductionFrame {
            cells: std::mem::take(cells),
            base: base.clone(),
            source_stack: stack.to_owned(),
            context,
            expression_depth,
            group: trace.group,
            steps: 0,
            resume_cursor: 0,
            pending: None,
            failure: None,
        };
        self.run_reduction_frames(root, trace.recorder)
    }

    fn run_reduction_frames(
        &self,
        root: ReductionFrame,
        recorder: &mut Option<TraceRecorder>,
    ) -> Result<NormalForm> {
        let mut frames = vec![root];
        loop {
            let action = self.drive_reduction_frame(
                frames
                    .last_mut()
                    .expect("reduction frame stack is non-empty"),
                recorder,
            );
            match action {
                FrameAction::Call(pending) => {
                    let implementation = pending.call.implementation.clone();
                    let context = pending.call.context;
                    frames.last_mut().expect("caller frame is present").pending =
                        Some(PendingApplication::Pair(pending));
                    match self.sayo_reduction_frame(&implementation, context, recorder) {
                        Ok(frame) => frames.push(frame),
                        Err(error) => {
                            let caller = frames.last_mut().expect("caller frame is present");
                            let PendingApplication::Pair(pending) =
                                caller.pending.take().expect("pending Sayo call")
                            else {
                                unreachable!("Sayo relation has a pair continuation")
                            };
                            let output = Self::sayo_relation_outcome(
                                &pending.call.implementation,
                                Err(error),
                            );
                            let outcome = self.finish_pair(
                                pending.call.left.clone(),
                                pending.call.right.clone(),
                                pending.call.relation.clone(),
                                output,
                            );
                            self.resume_pair(caller, pending, outcome, recorder);
                        }
                    }
                }
                FrameAction::Complete(result) => {
                    frames.pop().expect("completed frame is present");
                    let Some(caller_index) = frames.len().checked_sub(1) else {
                        return result;
                    };
                    let pending = frames[caller_index]
                        .pending
                        .take()
                        .expect("completed evaluation has a caller");
                    match pending {
                        PendingApplication::Pair(pending) => {
                            let output =
                                Self::sayo_relation_outcome(&pending.call.implementation, result);
                            let outcome = self.finish_pair(
                                pending.call.left.clone(),
                                pending.call.right.clone(),
                                pending.call.relation.clone(),
                                output,
                            );
                            self.resume_pair(&mut frames[caller_index], pending, outcome, recorder);
                        }
                        PendingApplication::Control(pending) => {
                            self.resume_control(
                                &mut frames,
                                caller_index,
                                pending,
                                result,
                                recorder,
                            )?;
                        }
                    }
                }
                FrameAction::Control(pending) => {
                    frames
                        .last_mut()
                        .expect("control caller frame is present")
                        .pending = Some(PendingApplication::Control(pending));
                    self.start_control_slot(&mut frames, recorder)?;
                }
            }
        }
    }

    fn sayo_reduction_frame(
        &self,
        implementation: &VirtualPath,
        context: RelationContext,
        recorder: &mut Option<TraceRecorder>,
    ) -> Result<ReductionFrame> {
        let source = implementation.read()?.ok_or_else(|| {
            Error::Source(format!(
                "cannot read Sayo source {}",
                implementation.display()
            ))
        })?;
        let expression = parser::parse(&source)?;
        let expressions = application_terms(&expression);
        let (group, trace_cells) = match recorder.as_mut() {
            Some(recorder) => {
                let sources = expressions.iter().map(Expression::render_source).collect();
                let (group, cells) = recorder.begin_group(context.trace_parent_group, sources);
                (Some(group), cells.into_iter().map(Some).collect())
            }
            None => (None, vec![None; expressions.len()]),
        };
        let cells = expressions
            .into_iter()
            .zip(trace_cells)
            .map(|(expression, trace_cell)| ReductionCell::expression(expression, trace_cell))
            .collect();
        Ok(ReductionFrame {
            cells,
            base: SourcePath::world(
                implementation
                    .parent()
                    .unwrap_or_else(|| self.world.clone()),
            ),
            source_stack: vec![SourcePath::world(implementation.clone())],
            context: Some(context),
            expression_depth: context.depth,
            group,
            steps: 0,
            resume_cursor: 0,
            pending: None,
            failure: None,
        })
    }

    fn sayo_relation_outcome(
        implementation: &VirtualPath,
        result: Result<NormalForm>,
    ) -> Result<RelationOutcome> {
        Ok(match result {
            Ok(normal_form) => {
                RelationOutcome::Success(RelationResult::Terms(normal_form.into_terms()))
            }
            Err(Error::BootstrapLimit(message)) => {
                RelationOutcome::Trap(Error::BootstrapLimit(message))
            }
            Err(error) => RelationOutcome::Trap(Error::Implementation(format!(
                "Sayo implementation {} failed: {error}",
                implementation.display()
            ))),
        })
    }

    fn resume_pair(
        &self,
        frame: &mut ReductionFrame,
        pending: PendingPairApplication,
        outcome: ReducerOutcome,
        recorder: &mut Option<TraceRecorder>,
    ) {
        let mut trace = TraceContext {
            recorder,
            group: frame.group,
        };
        match outcome {
            ReducerOutcome::Success { terms, relation } => {
                frame.steps += 1;
                if frame.steps > self.limits.reduction_steps {
                    frame.failure = Some(Error::BootstrapLimit(format!(
                        "reduction step limit exceeded: limit={}",
                        self.limits.reduction_steps
                    )));
                    return;
                }
                let inserted = terms
                    .into_iter()
                    .map(|term| {
                        let trace_cell =
                            trace.recorder.as_mut().map(|recorder| recorder.new_cell());
                        ReductionCell::term(term, trace_cell)
                    })
                    .collect::<Vec<_>>();
                if let (Some(group), Some(removed_left), Some(removed_right)) =
                    (frame.group, pending.left_cell, pending.right_cell)
                {
                    let inserted_trace = inserted
                        .iter()
                        .filter_map(|cell| {
                            cell.trace_cell.map(|cell_id| TraceInsertion {
                                cell: cell_id,
                                term: self.trace_term(cell.term.as_ref().expect("term cell")),
                            })
                        })
                        .collect();
                    Self::record_trace_event(
                        &mut trace,
                        TraceEvent::Rewritten {
                            group,
                            removed_left,
                            removed_right,
                            inserted: inserted_trace,
                            relation,
                        },
                    );
                }
                frame
                    .cells
                    .splice(pending.cursor..pending.cursor + 2, inserted);
                frame.resume_cursor = 0;
            }
            ReducerOutcome::NoStep(reason) => {
                if let (Some(group), Some(left_cell), Some(right_cell)) =
                    (frame.group, pending.left_cell, pending.right_cell)
                {
                    Self::record_trace_event(
                        &mut trace,
                        TraceEvent::NotApplicable {
                            group,
                            left_cell,
                            right_cell,
                            reason,
                        },
                    );
                }
                frame.resume_cursor = pending.cursor + 1;
            }
            ReducerOutcome::Trap(error) => frame.failure = Some(error),
        }
    }

    fn start_control_slot(
        &self,
        frames: &mut Vec<ReductionFrame>,
        recorder: &mut Option<TraceRecorder>,
    ) -> Result<()> {
        let caller_index = frames.len().checked_sub(1).expect("control caller frame");
        let (expression, context, base, implementation, finish_group_end, control) = {
            let PendingApplication::Control(pending) = frames[caller_index]
                .pending
                .as_mut()
                .expect("pending control call")
            else {
                unreachable!("control slot has a control continuation")
            };
            if pending.trace_control.is_none()
                && let Some(recorder) = recorder.as_mut()
            {
                pending.trace_control = Some(recorder.new_control());
            }
            let Some(slot) = pending.call.slots.get(pending.next_slot).cloned() else {
                return self.complete_control(frames, caller_index, recorder);
            };
            let (expression, finish_group_end) = match slot {
                Expression::Group(inner) => (*inner, true),
                expression => (expression, false),
            };
            pending.finish_group_end = finish_group_end;
            let control = TraceControlGroup {
                control_id: pending.trace_control.unwrap_or(0),
                anchor_cell: pending.removed_cell,
                transition: pending.kind.trace_name().into(),
                result_template: pending.call.result_template.clone(),
                implementation_source: pending
                    .call
                    .slots
                    .iter()
                    .map(Expression::render_source)
                    .collect::<Vec<_>>()
                    .join(" "),
                slots: pending
                    .call
                    .slots
                    .iter()
                    .map(Expression::render_source)
                    .collect(),
                slot: pending.next_slot,
                slot_count: pending.call.slots.len(),
            };
            (
                expression,
                pending.call.context,
                pending.call.base.clone(),
                pending.call.implementation.clone(),
                finish_group_end,
                control,
            )
        };
        let expressions = application_terms(&expression);
        let (group, trace_cells) = match recorder.as_mut() {
            Some(recorder) => {
                let sources = expressions.iter().map(Expression::render_source).collect();
                let (group, cells) =
                    recorder.begin_control_group(context.trace_parent_group, sources, control);
                (Some(group), cells.into_iter().map(Some).collect())
            }
            None => (None, vec![None; expressions.len()]),
        };
        let cells = expressions
            .into_iter()
            .zip(trace_cells)
            .map(|(expression, trace_cell)| ReductionCell::expression(expression, trace_cell))
            .collect();
        let _ = finish_group_end;
        frames.push(ReductionFrame {
            cells,
            base,
            source_stack: vec![SourcePath::world(implementation)],
            context: Some(context),
            expression_depth: context.depth,
            group,
            steps: 0,
            resume_cursor: 0,
            pending: None,
            failure: None,
        });
        Ok(())
    }

    // Resuming the final slot may push the next explicit evaluation frame.
    #[allow(clippy::ptr_arg)]
    fn resume_control(
        &self,
        frames: &mut Vec<ReductionFrame>,
        caller_index: usize,
        mut pending: PendingControlApplication,
        mut result: Result<NormalForm>,
        recorder: &mut Option<TraceRecorder>,
    ) -> Result<()> {
        if pending.finish_group_end
            && let Ok(normal_form) = result
        {
            let mut trace = TraceContext {
                recorder,
                group: frames[caller_index].group,
            };
            result = self.finish_group_end(normal_form, pending.call.depth, &mut trace);
        }
        match result {
            Ok(normal_form) => {
                let count = normal_form.terms().len();
                match normal_form.single() {
                    Ok(term) => pending.next_payload.push(term),
                    Err(_) => {
                        frames[caller_index].failure = Some(Error::Implementation(format!(
                            "Template control payload slot must produce exactly one Term, produced {count}: {}",
                            pending.call.implementation.display()
                        )));
                        return Ok(());
                    }
                }
            }
            Err(error) => {
                frames[caller_index].failure = Some(error);
                return Ok(());
            }
        }
        pending.next_slot += 1;
        frames[caller_index].pending = Some(PendingApplication::Control(pending));
        self.start_control_slot(frames, recorder)
    }

    fn complete_control(
        &self,
        frames: &mut [ReductionFrame],
        caller_index: usize,
        recorder: &mut Option<TraceRecorder>,
    ) -> Result<()> {
        let PendingApplication::Control(mut pending) = frames[caller_index]
            .pending
            .take()
            .expect("completed control continuation")
        else {
            unreachable!("completed control has a control continuation")
        };
        let mut trace = TraceContext {
            recorder,
            group: frames[caller_index].group,
        };
        let transition = self.finish_template_control(
            &pending.call.payload,
            std::mem::take(&mut pending.call.used),
            &pending.call.result_template,
            pending.call.zero_state_transition,
            std::mem::take(&mut pending.next_payload),
            pending.call.depth,
            pending.call.context,
            &mut trace,
        );
        self.resume_control_transition(&mut frames[caller_index], pending, transition, recorder);
        Ok(())
    }

    fn resume_control_transition(
        &self,
        frame: &mut ReductionFrame,
        pending: PendingControlApplication,
        transition: NativeStateTransition,
        recorder: &mut Option<TraceRecorder>,
    ) {
        match transition {
            NativeStateTransition::Replace(terms) => {
                frame.steps += 1;
                if frame.steps > self.limits.reduction_steps {
                    frame.failure = Some(Error::BootstrapLimit(format!(
                        "reduction step limit exceeded: limit={}",
                        self.limits.reduction_steps
                    )));
                    return;
                }
                let mut trace = TraceContext {
                    recorder,
                    group: frame.group,
                };
                let inserted = terms
                    .into_iter()
                    .map(|term| {
                        let trace_cell =
                            trace.recorder.as_mut().map(|recorder| recorder.new_cell());
                        ReductionCell::term(term, trace_cell)
                    })
                    .collect::<Vec<_>>();
                if let (Some(group), Some(removed)) = (frame.group, pending.removed_cell) {
                    let traced = inserted
                        .iter()
                        .filter_map(|cell| {
                            cell.trace_cell.map(|cell_id| TraceInsertion {
                                cell: cell_id,
                                term: self.trace_term(cell.term.as_ref().expect("term cell")),
                            })
                        })
                        .collect();
                    let event = match pending.kind {
                        ControlTransitionKind::Tick => TraceEvent::StateTransition {
                            group,
                            removed,
                            inserted: traced,
                            control_id: pending.trace_control,
                        },
                        ControlTransitionKind::End => TraceEvent::EndTransition {
                            group,
                            removed,
                            inserted: traced,
                            control_id: pending.trace_control,
                        },
                    };
                    Self::record_trace_event(&mut trace, event);
                }
                frame
                    .cells
                    .splice(pending.cursor..=pending.cursor, inserted);
                frame.resume_cursor = 0;
            }
            NativeStateTransition::NoStep => {
                frame.resume_cursor = match pending.kind {
                    ControlTransitionKind::Tick => pending.cursor,
                    ControlTransitionKind::End => pending.cursor + 1,
                };
            }
            NativeStateTransition::Trap(error) => frame.failure = Some(error),
        }
    }

    fn drive_reduction_frame(
        &self,
        frame: &mut ReductionFrame,
        recorder: &mut Option<TraceRecorder>,
    ) -> FrameAction {
        let group = frame.group;
        let result = (|| -> Result<FrameAction> {
            if let Some(error) = frame.failure.take() {
                return Err(error);
            }
            let cells = &mut frame.cells;
            let base = &frame.base;
            let stack = &mut frame.source_stack;
            let context = frame.context;
            let expression_depth = frame.expression_depth;
            let steps = &mut frame.steps;
            let mut trace = TraceContext { recorder, group };
            debug_assert!(!cells.is_empty());
            loop {
                if cells.is_empty() {
                    return Ok(FrameAction::Complete(Ok(NormalForm::new(Vec::new()))));
                }
                let mut cursor = std::mem::take(&mut frame.resume_cursor);
                let mut restarted = false;
                while cursor < cells.len() {
                    if self.materialize_cell(
                        cells,
                        cursor,
                        base,
                        stack,
                        context,
                        expression_depth,
                        &mut trace,
                    )? {
                        restarted = true;
                        break;
                    }
                    let current = cells[cursor]
                        .term
                        .as_ref()
                        .expect("materialized cell")
                        .clone();
                    let transition = match self.prepare_term_control(
                        &current,
                        ".tick",
                        current.evaluation_depth,
                        group,
                    ) {
                        Some(PreparedControl::Sayo(call)) => {
                            return Ok(FrameAction::Control(PendingControlApplication {
                                cursor,
                                removed_cell: cells[cursor].trace_cell,
                                kind: ControlTransitionKind::Tick,
                                call: *call,
                                next_slot: 0,
                                next_payload: Vec::new(),
                                finish_group_end: false,
                                trace_control: None,
                            }));
                        }
                        Some(PreparedControl::Ready(NativeStateTransition::NoStep)) => self
                            .transition_native_state(
                                &current,
                                current.evaluation_depth,
                                &mut trace,
                            ),
                        None => match self.transition_native_tick(
                            &current,
                            current.evaluation_depth,
                            &mut trace,
                        ) {
                            NativeStateTransition::NoStep => self.transition_native_state(
                                &current,
                                current.evaluation_depth,
                                &mut trace,
                            ),
                            transition => transition,
                        },
                        Some(PreparedControl::Ready(transition)) => transition,
                    };
                    match transition {
                        NativeStateTransition::Replace(terms) => {
                            *steps += 1;
                            if *steps > self.limits.reduction_steps {
                                return Err(Error::BootstrapLimit(format!(
                                    "reduction step limit exceeded: limit={}",
                                    self.limits.reduction_steps
                                )));
                            }
                            let inserted = terms
                                .into_iter()
                                .map(|term| {
                                    let trace_cell =
                                        trace.recorder.as_mut().map(|recorder| recorder.new_cell());
                                    ReductionCell::term(term, trace_cell)
                                })
                                .collect::<Vec<_>>();
                            if let (Some(group), Some(removed)) = (group, cells[cursor].trace_cell)
                            {
                                let inserted_trace = inserted
                                    .iter()
                                    .filter_map(|cell| {
                                        cell.trace_cell.map(|cell_id| TraceInsertion {
                                            cell: cell_id,
                                            term: self
                                                .trace_term(cell.term.as_ref().expect("term cell")),
                                        })
                                    })
                                    .collect();
                                Self::record_trace_event(
                                    &mut trace,
                                    TraceEvent::StateTransition {
                                        group,
                                        removed,
                                        inserted: inserted_trace,
                                        control_id: None,
                                    },
                                );
                            }
                            cells.splice(cursor..=cursor, inserted);
                            restarted = true;
                            break;
                        }
                        NativeStateTransition::NoStep => {}
                        NativeStateTransition::Trap(error) => return Err(error),
                    }
                    if cursor + 1 == cells.len() {
                        cursor += 1;
                        continue;
                    }
                    if self.materialize_cell(
                        cells,
                        cursor + 1,
                        base,
                        stack,
                        context,
                        expression_depth,
                        &mut trace,
                    )? {
                        restarted = true;
                        break;
                    }
                    let left_cell = cells[cursor].trace_cell;
                    let right_cell = cells[cursor + 1].trace_cell;
                    let left = cells[cursor]
                        .term
                        .as_ref()
                        .expect("materialized left cell")
                        .clone();
                    let right = cells[cursor + 1]
                        .term
                        .as_ref()
                        .expect("materialized right cell")
                        .clone();
                    if let (Some(group), Some(left_cell), Some(right_cell)) =
                        (group, left_cell, right_cell)
                    {
                        Self::record_trace_event(
                            &mut trace,
                            TraceEvent::Attempted {
                                group,
                                left_cell,
                                right_cell,
                                left: self.trace_term(&left),
                                right: self.trace_term(&right),
                            },
                        );
                    }
                    match self.prepare_pair(left, right, context, &mut trace) {
                        PreparedPair::Sayo(call) => {
                            return Ok(FrameAction::Call(PendingPairApplication {
                                cursor,
                                left_cell,
                                right_cell,
                                call: *call,
                            }));
                        }
                        PreparedPair::Ready(outcome) => match outcome {
                            ReducerOutcome::Success { terms, relation } => {
                                *steps += 1;
                                if *steps > self.limits.reduction_steps {
                                    return Err(Error::BootstrapLimit(format!(
                                        "reduction step limit exceeded: limit={}",
                                        self.limits.reduction_steps
                                    )));
                                }
                                let inserted = terms
                                    .into_iter()
                                    .map(|term| {
                                        let trace_cell = trace
                                            .recorder
                                            .as_mut()
                                            .map(|recorder| recorder.new_cell());
                                        ReductionCell::term(term, trace_cell)
                                    })
                                    .collect::<Vec<_>>();
                                if let (Some(group), Some(removed_left), Some(removed_right)) =
                                    (group, left_cell, right_cell)
                                {
                                    let inserted_trace = inserted
                                        .iter()
                                        .filter_map(|cell| {
                                            cell.trace_cell.map(|cell_id| TraceInsertion {
                                                cell: cell_id,
                                                term: self.trace_term(
                                                    cell.term.as_ref().expect("term cell"),
                                                ),
                                            })
                                        })
                                        .collect();
                                    Self::record_trace_event(
                                        &mut trace,
                                        TraceEvent::Rewritten {
                                            group,
                                            removed_left,
                                            removed_right,
                                            inserted: inserted_trace,
                                            relation,
                                        },
                                    );
                                }
                                cells.splice(cursor..cursor + 2, inserted);
                                restarted = true;
                                break;
                            }
                            ReducerOutcome::NoStep(reason) => {
                                if let (Some(group), Some(left_cell), Some(right_cell)) =
                                    (group, left_cell, right_cell)
                                {
                                    Self::record_trace_event(
                                        &mut trace,
                                        TraceEvent::NotApplicable {
                                            group,
                                            left_cell,
                                            right_cell,
                                            reason,
                                        },
                                    );
                                }
                                cursor += 1;
                            }
                            ReducerOutcome::Trap(error) => return Err(error),
                        },
                    }
                }
                if restarted {
                    continue;
                }
                let last_index = cells.len().saturating_sub(1);
                let last = cells[last_index]
                    .term
                    .as_ref()
                    .expect("normal-form candidate is materialized")
                    .clone();
                let end_transition =
                    match self.prepare_term_control(&last, ".end", last.evaluation_depth, group) {
                        Some(PreparedControl::Sayo(call)) => {
                            return Ok(FrameAction::Control(PendingControlApplication {
                                cursor: last_index,
                                removed_cell: cells[last_index].trace_cell,
                                kind: ControlTransitionKind::End,
                                call: *call,
                                next_slot: 0,
                                next_payload: Vec::new(),
                                finish_group_end: false,
                                trace_control: None,
                            }));
                        }
                        Some(PreparedControl::Ready(transition)) => transition,
                        None => {
                            self.transition_native_end(&last, last.evaluation_depth, &mut trace)
                        }
                    };
                match end_transition {
                    NativeStateTransition::Replace(terms) => {
                        *steps += 1;
                        if *steps > self.limits.reduction_steps {
                            return Err(Error::BootstrapLimit(format!(
                                "reduction step limit exceeded: limit={}",
                                self.limits.reduction_steps
                            )));
                        }
                        let removed = cells[last_index].trace_cell;
                        let replacement = terms
                            .into_iter()
                            .map(|term| {
                                let trace_cell =
                                    trace.recorder.as_mut().map(|recorder| recorder.new_cell());
                                ReductionCell::term(term, trace_cell)
                            })
                            .collect::<Vec<_>>();
                        if let (Some(group), Some(removed)) = (group, removed) {
                            let inserted = replacement
                                .iter()
                                .filter_map(|cell| {
                                    cell.trace_cell.map(|cell_id| TraceInsertion {
                                        cell: cell_id,
                                        term: self
                                            .trace_term(cell.term.as_ref().expect("term cell")),
                                    })
                                })
                                .collect();
                            Self::record_trace_event(
                                &mut trace,
                                TraceEvent::EndTransition {
                                    group,
                                    removed,
                                    inserted,
                                    control_id: None,
                                },
                            );
                        }
                        cells.splice(last_index..=last_index, replacement);
                        continue;
                    }
                    NativeStateTransition::NoStep => {}
                    NativeStateTransition::Trap(error) => return Err(error),
                }
                let terms = cells
                    .iter()
                    .map(|cell| {
                        cell.term
                            .clone()
                            .expect("all normal-form cells are materialized")
                    })
                    .collect::<Vec<_>>();
                if let Some(group) = group {
                    for cell in cells.iter() {
                        if let Some(cell_id) = cell.trace_cell {
                            Self::record_trace_event(
                                &mut trace,
                                TraceEvent::Yielded {
                                    group,
                                    cell: cell_id,
                                    term: self
                                        .trace_term(cell.term.as_ref().expect("materialized cell")),
                                },
                            );
                        }
                    }
                }
                return Ok(FrameAction::Complete(Ok(NormalForm::new(terms))));
            }
        })();
        match result {
            Ok(action) => action,
            Err(error) => {
                let mut trace = TraceContext { recorder, group };
                Self::record_trace_trap(&mut trace, group, &error);
                FrameAction::Complete(Err(error))
            }
        }
    }

    /// Returns true if a Group expanded into the surrounding sequence.  The
    /// caller must then restart its leftmost scan from the beginning.
    #[allow(clippy::too_many_arguments)]
    fn materialize_cell(
        &self,
        cells: &mut Vec<ReductionCell>,
        index: usize,
        base: &SourcePath,
        stack: &mut Vec<SourcePath>,
        context: Option<RelationContext>,
        expression_depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<bool> {
        if cells[index].term.is_none() {
            let expression = cells[index].expression.take().ok_or_else(|| {
                Error::Implementation("reducer cell has neither an expression nor a Term".into())
            })?;
            let was_group = matches!(expression, Expression::Group(_));
            let normal_form = self.evaluate_expression(
                &expression,
                base,
                stack,
                context,
                expression_depth + 1,
                trace,
            )?;
            let terms = normal_form.into_terms();
            if terms.is_empty() {
                return Err(Error::Implementation(
                    "a Group produced an empty normal form".into(),
                ));
            }
            if terms.len() == 1 {
                let term = terms.into_iter().next().expect("one term");
                if let (Some(group), Some(cell_id)) = (trace.group, cells[index].trace_cell) {
                    Self::record_trace_event(
                        trace,
                        TraceEvent::Materialized {
                            group,
                            cell: cell_id,
                            term: self.trace_term(&term),
                        },
                    );
                }
                cells[index].term = Some(term);
                return Ok(was_group);
            }
            let replacement = terms
                .into_iter()
                .map(|term| {
                    let trace_cell = trace.recorder.as_mut().map(|recorder| recorder.new_cell());
                    ReductionCell::term(term, trace_cell)
                })
                .collect::<Vec<_>>();
            cells.splice(index..=index, replacement);
            return Ok(true);
        }
        Ok(false)
    }

    fn record_trace_event(trace: &mut TraceContext<'_>, event: TraceEvent) {
        if let Some(recorder) = trace.recorder.as_mut() {
            recorder.record(event);
        }
    }

    fn record_trace_trap(trace: &mut TraceContext<'_>, group: Option<TraceGroupId>, error: &Error) {
        Self::record_trace_event(
            trace,
            TraceEvent::Trapped {
                group,
                category: error.category().into(),
                message: error.to_string(),
            },
        );
    }

    fn spaces_for(
        &self,
        source: &str,
        stack: &mut Vec<SourcePath>,
        trace: &mut TraceContext<'_>,
        inside_space_checker: bool,
    ) -> Result<Vec<String>> {
        let mut matches = Vec::new();
        let mut seen = HashSet::new();
        for root in self.world_roots() {
            for entry in root.directories()? {
                let name = entry.file_name().unwrap_or("");
                if name.starts_with('.') || !seen.insert(name.to_owned()) {
                    continue;
                }
                if entry.join("main.native")?.is_file()? {
                    return Err(Error::World(
                        "main.native declarations are not allowed; Native Relations are provided by SayoVM"
                            .into(),
                    ));
                }
                let sayo_checker = entry.join("space.sayo")?;
                if sayo_checker.is_file()? {
                    // A checker is a restricted classifier program, not a
                    // candidate for classification by other user checkers.
                    // Its atoms resolve only through the trusted Native and
                    // embedded-standard layer.
                    if inside_space_checker {
                        continue;
                    }
                    let checker_source = SourcePath::world(sayo_checker.clone());
                    if stack.iter().any(|path| path == &checker_source) {
                        continue;
                    }
                    match self.run_sayo_space_checker(&sayo_checker, source, stack, trace)? {
                        SpaceCheckOutcome::Match => matches.push(name.to_owned()),
                        SpaceCheckOutcome::Miss => {}
                    }
                    continue;
                }
            }
        }
        Ok(matches)
    }

    fn run_sayo_space_checker(
        &self,
        checker: &VirtualPath,
        candidate_source: &str,
        stack: &mut Vec<SourcePath>,
        trace: &mut TraceContext<'_>,
    ) -> Result<SpaceCheckOutcome> {
        let rendered = render_string_literal(candidate_source);
        let candidate = self.store_term(Term {
            term_id: None,
            evaluation_depth: 0,
            source: rendered.clone(),
            display: rendered,
            space: "String".into(),
            implementation: TermImplementation::None,
        })?;
        let candidate_id = candidate
            .term_id
            .ok_or_else(|| Error::Implementation("Space candidate has no TermId".into()))?;
        let state_native_id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(
                state_native_id,
                NativeTermState::Template {
                    template: "__SpaceChecker".into(),
                    payload: vec![candidate_id],
                    schema: false,
                    space_checker: true,
                },
            );
        let context = RelationContext {
            input_term_id: candidate_id,
            depth: 0,
            trace_parent_group: trace.group,
            state_native_id: Some(state_native_id),
            schema: false,
            space_checker: true,
        };
        let checker_path = SourcePath::world(checker.clone());
        stack.push(checker_path.clone());
        let checker_source = checker_path.read();
        let checker_base = checker_path
            .parent()
            .unwrap_or_else(|| SourcePath::world(self.world.clone()));
        let normal_form = checker_source
            .and_then(|source| {
                self.evaluate_source(&source, &checker_base, stack, Some(context), trace)
            })
            .map_err(|error| {
                Error::Implementation(format!(
                    "Sayo Space Checker {} failed: {error}",
                    checker.display()
                ))
            });
        stack.pop();
        let normal_form = normal_form?;
        let terms = normal_form.terms();
        let [term] = terms else {
            return Err(Error::Implementation(format!(
                "Sayo Space Checker {} must produce exactly one Term, produced {}",
                checker.display(),
                terms.len()
            )));
        };
        if term.space == "__Miss" {
            Ok(SpaceCheckOutcome::Miss)
        } else {
            Ok(SpaceCheckOutcome::Match)
        }
    }

    /// Reduce Runtime-owned state before attempting an ordinary binary
    /// Relation. This is the second priority after Group expansion.
    fn transition_native_state(
        &self,
        term: &Term,
        evaluation_depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> NativeStateTransition {
        let TermImplementation::Native(native_id) = term.implementation else {
            return NativeStateTransition::NoStep;
        };
        let state = match self.native_terms.lock() {
            Ok(states) => states.get(&native_id).cloned(),
            Err(_) => {
                return NativeStateTransition::Trap(Error::Implementation(
                    "native term store lock poisoned".into(),
                ));
            }
        };
        let Some(state) = state else {
            return NativeStateTransition::Trap(Error::Implementation(format!(
                "unknown Native Term: {native_id}"
            )));
        };
        match state {
            NativeTermState::Arithmetic { .. }
            | NativeTermState::Comparison { .. }
            | NativeTermState::StringComparison { .. }
            | NativeTermState::StringEquality { .. }
            | NativeTermState::Ordering => NativeStateTransition::NoStep,
            NativeTermState::Template {
                template,
                payload,
                schema,
                ..
            } => {
                if !schema && template == "Vanish" {
                    return NativeStateTransition::Replace(Vec::new());
                }
                if schema && template == "Pass" {
                    return NativeStateTransition::Replace(Vec::new());
                }
                if schema && template == "CheckSpace" {
                    if payload.len() != 2 {
                        return NativeStateTransition::Trap(Error::Implementation(
                            "CheckSpace requires a term and a Space".into(),
                        ));
                    }
                    let value = match self.term_for_id(payload[0]) {
                        Ok(term) => term,
                        Err(error) => return NativeStateTransition::Trap(error),
                    };
                    let expected = match self.term_for_id(payload[1]) {
                        Ok(term) => term,
                        Err(error) => return NativeStateTransition::Trap(error),
                    };
                    if value.space != expected.source && value.space != expected.display {
                        return NativeStateTransition::Trap(Error::Implementation(format!(
                            "CheckSpace failed: {} is not in Space {}",
                            value.display, expected.source
                        )));
                    }
                    match self.register_template_state(
                        "Pass",
                        Vec::new(),
                        evaluation_depth,
                        None,
                        trace,
                    ) {
                        Ok(pass) => return NativeStateTransition::Replace(vec![pass]),
                        Err(error) => return NativeStateTransition::Trap(error),
                    }
                }
                let _payload_len = payload.len();
                NativeStateTransition::NoStep
            }
            NativeTermState::EmitTwice { captured } => match (|| {
                let value = self.term_for_id(captured)?;
                let next = self.register_native_emitter(captured, false, evaluation_depth)?;
                Ok(vec![value, next])
            })() {
                Ok(terms) => NativeStateTransition::Replace(terms),
                Err(error) => NativeStateTransition::Trap(error),
            },
            NativeTermState::EmitOnce { captured } => match (|| {
                let value = self.term_for_id(captured)?;
                let continuation =
                    self.store_term(Term::atom("duplicate".into(), "Duplicate".into()))?;
                Ok(vec![value, continuation])
            })() {
                Ok(terms) => NativeStateTransition::Replace(terms),
                Err(error) => NativeStateTransition::Trap(error),
            },
            NativeTermState::Each { .. } => NativeStateTransition::NoStep,
            NativeTermState::EachApply { transform, input } => {
                match (|| {
                    let value =
                        self.apply_transform_once(transform, input, term.evaluation_depth, trace)?;
                    Ok(vec![self.register_native_each_emit(
                        transform,
                        value.term_id.ok_or_else(|| {
                            Error::Implementation("Each value has no TermId".into())
                        })?,
                        term.evaluation_depth,
                    )?])
                })() {
                    Ok(terms) => NativeStateTransition::Replace(terms),
                    Err(error) => NativeStateTransition::Trap(error),
                }
            }
            NativeTermState::EachEmit { transform, value } => match (|| {
                Ok(vec![
                    self.term_for_id(value)?,
                    self.register_native_each(transform, term.evaluation_depth)?,
                ])
            })() {
                Ok(terms) => NativeStateTransition::Replace(terms),
                Err(error) => NativeStateTransition::Trap(error),
            },
            NativeTermState::Sieve { .. } | NativeTermState::FilterChoice(_) => {
                NativeStateTransition::NoStep
            }
            NativeTermState::SieveApply { predicate, input } => match (|| {
                let decision =
                    self.apply_transform_once(predicate, input, term.evaluation_depth, trace)?;
                let kind = self.filter_choice(&decision)?;
                Ok(vec![self.register_native_sieve_resolution(
                    predicate,
                    input,
                    kind,
                    term.evaluation_depth,
                )?])
            })() {
                Ok(terms) => NativeStateTransition::Replace(terms),
                Err(error) => NativeStateTransition::Trap(error),
            },
            NativeTermState::SieveKeep { predicate, input } => match (|| {
                Ok(vec![
                    self.term_for_id(input)?,
                    self.register_native_sieve(predicate, term.evaluation_depth)?,
                ])
            })() {
                Ok(terms) => NativeStateTransition::Replace(terms),
                Err(error) => NativeStateTransition::Trap(error),
            },
            NativeTermState::SieveDrop { predicate } => {
                match self.register_native_sieve(predicate, term.evaluation_depth) {
                    Ok(term) => NativeStateTransition::Replace(vec![term]),
                    Err(error) => NativeStateTransition::Trap(error),
                }
            }
            NativeTermState::FoldInit { .. } | NativeTermState::FoldAccum { .. } => {
                NativeStateTransition::NoStep
            }
            NativeTermState::FoldApply {
                transform,
                accumulator,
                input,
            } => {
                match (|| {
                    let partial = self.apply_transform_once(
                        transform,
                        accumulator,
                        term.evaluation_depth,
                        trace,
                    )?;
                    let value = self.apply_transform_once(
                        partial.term_id.ok_or_else(|| {
                            Error::Implementation("Fold partial has no id".into())
                        })?,
                        input,
                        term.evaluation_depth,
                        trace,
                    )?;
                    Ok(vec![
                        self.register_native_fold_next(
                            transform,
                            value.term_id.ok_or_else(|| {
                                Error::Implementation("Fold value has no id".into())
                            })?,
                            term.evaluation_depth,
                        )?,
                    ])
                })() {
                    Ok(terms) => NativeStateTransition::Replace(terms),
                    Err(error) => NativeStateTransition::Trap(error),
                }
            }
            NativeTermState::FoldNext {
                transform,
                accumulator,
            } => {
                match self.register_native_fold_accum(transform, accumulator, term.evaluation_depth)
                {
                    Ok(term) => NativeStateTransition::Replace(vec![term]),
                    Err(error) => NativeStateTransition::Trap(error),
                }
            }
            NativeTermState::FoldClose { accumulator } => match self.term_for_id(accumulator) {
                Ok(term) => NativeStateTransition::Replace(vec![term]),
                Err(error) => NativeStateTransition::Trap(error),
            },
            NativeTermState::ReduceInit { .. } | NativeTermState::ReduceAccum { .. } => {
                NativeStateTransition::NoStep
            }
            NativeTermState::ReduceApply {
                transform,
                accumulator,
                input,
            } => {
                match (|| {
                    let partial = self.apply_transform_once(
                        transform,
                        accumulator,
                        term.evaluation_depth,
                        trace,
                    )?;
                    let value = self.apply_transform_once(
                        partial.term_id.ok_or_else(|| {
                            Error::Implementation("Reduce partial has no id".into())
                        })?,
                        input,
                        term.evaluation_depth,
                        trace,
                    )?;
                    Ok(vec![self.register_native_reduce_next(
                        transform,
                        value.term_id.ok_or_else(|| {
                            Error::Implementation("Reduce value has no id".into())
                        })?,
                        term.evaluation_depth,
                    )?])
                })() {
                    Ok(terms) => NativeStateTransition::Replace(terms),
                    Err(error) => NativeStateTransition::Trap(error),
                }
            }
            NativeTermState::ReduceNext {
                transform,
                accumulator,
            } => match self.register_native_reduce_accum(
                transform,
                accumulator,
                term.evaluation_depth,
            ) {
                Ok(term) => NativeStateTransition::Replace(vec![term]),
                Err(error) => NativeStateTransition::Trap(error),
            },
            NativeTermState::ReduceClose { accumulator } => match self.term_for_id(accumulator) {
                Ok(term) => NativeStateTransition::Replace(vec![term]),
                Err(error) => NativeStateTransition::Trap(error),
            },
            #[cfg(test)]
            NativeTermState::SelfLoop => NativeStateTransition::Replace(vec![term.clone()]),
        }
    }

    fn prepare_term_control(
        &self,
        term: &Term,
        slot: &str,
        depth: usize,
        trace_group: Option<TraceGroupId>,
    ) -> Option<PreparedControl> {
        let TermImplementation::Native(native_id) = term.implementation else {
            return None;
        };
        let state = self
            .native_terms
            .lock()
            .ok()
            .and_then(|states| states.get(&native_id).cloned());
        let Some(NativeTermState::Template {
            template,
            payload,
            space_checker,
            ..
        }) = state
        else {
            return None;
        };
        Some(self.prepare_template_control(
            native_id,
            &template,
            &payload,
            space_checker,
            slot,
            depth,
            trace_group,
        ))
    }

    /// `End` is never materialized as a Term. It is offered only to the final
    /// Runtime state of a Group after ordinary reduction has reached a normal
    /// form candidate.
    fn transition_native_end(
        &self,
        term: &Term,
        evaluation_depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> NativeStateTransition {
        let TermImplementation::Native(id) = term.implementation else {
            return NativeStateTransition::NoStep;
        };
        let state = self
            .native_terms
            .lock()
            .ok()
            .and_then(|states| states.get(&id).cloned());
        match state {
            Some(NativeTermState::FoldAccum { accumulator, .. }) => {
                match self.register_native_fold_close(accumulator, evaluation_depth) {
                    Ok(term) => NativeStateTransition::Replace(vec![term]),
                    Err(error) => NativeStateTransition::Trap(error),
                }
            }
            Some(NativeTermState::ReduceInit { .. }) => NativeStateTransition::Trap(
                Error::Implementation("Reduce requires at least one input Term".into()),
            ),
            Some(NativeTermState::ReduceAccum { accumulator, .. }) => {
                match self.register_native_reduce_close(accumulator, evaluation_depth) {
                    Ok(term) => NativeStateTransition::Replace(vec![term]),
                    Err(error) => NativeStateTransition::Trap(error),
                }
            }
            Some(NativeTermState::Template {
                template,
                payload,
                space_checker,
                ..
            }) => self.run_template_end(
                id,
                &template,
                &payload,
                space_checker,
                evaluation_depth,
                trace,
            ),
            _ => NativeStateTransition::NoStep,
        }
    }

    fn transition_native_tick(
        &self,
        term: &Term,
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> NativeStateTransition {
        let TermImplementation::Native(native_id) = term.implementation else {
            return NativeStateTransition::NoStep;
        };
        let state = self
            .native_terms
            .lock()
            .ok()
            .and_then(|states| states.get(&native_id).cloned());
        let Some(NativeTermState::Template {
            template,
            payload,
            space_checker,
            ..
        }) = state
        else {
            return NativeStateTransition::NoStep;
        };
        self.run_template_control(
            native_id,
            &template,
            &payload,
            space_checker,
            ".tick",
            depth,
            trace,
        )
    }

    fn run_template_end(
        &self,
        native_id: NativeTermId,
        template: &str,
        payload: &[TermId],
        space_checker: bool,
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> NativeStateTransition {
        self.run_template_control(
            native_id,
            template,
            payload,
            space_checker,
            ".end",
            depth,
            trace,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_template_control(
        &self,
        native_id: NativeTermId,
        template: &str,
        payload: &[TermId],
        space_checker: bool,
        slot: &str,
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> NativeStateTransition {
        let mut selected = None;
        for component in self.component_roots(template) {
            let root = match component.join(slot) {
                Ok(root) => root,
                Err(error) => return NativeStateTransition::Trap(error),
            };
            if !matches!(root.is_dir(), Ok(true)) {
                continue;
            }
            let results = match root.directories().map(|results| {
                results
                    .into_iter()
                    .filter(|result| {
                        result
                            .file_name()
                            .is_some_and(|name| name == ".any" || !name.starts_with('.'))
                    })
                    .collect::<Vec<_>>()
            }) {
                Ok(results) => results,
                Err(error) => return NativeStateTransition::Trap(error),
            };
            match results.as_slice() {
                [] => continue,
                [result] => {
                    selected = Some(result.clone());
                    break;
                }
                _ => {
                    return NativeStateTransition::Trap(Error::Relation(format!(
                        "ambiguous Template control Relation for `{template}` {slot}: {}",
                        results
                            .iter()
                            .filter_map(VirtualPath::file_name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }
        let Some(result) = selected else {
            return NativeStateTransition::NoStep;
        };
        let result_template = result.file_name().unwrap_or("").to_owned();
        let implementation = match result.join("main.sayo") {
            Ok(implementation) => implementation,
            Err(error) => return NativeStateTransition::Trap(error),
        };
        let has_implementation = match implementation.is_file() {
            Ok(value) => value,
            Err(error) => return NativeStateTransition::Trap(error),
        };
        let zero_state_transition = !has_implementation && result_template != "Vanish";
        if zero_state_transition && matches!(result_template.as_str(), "Trap" | ".any") {
            return NativeStateTransition::Trap(Error::Implementation(format!(
                "Template `{template}` {slot} with Result `{result_template}` requires main.sayo"
            )));
        }
        let context = RelationContext {
            input_term_id: payload.first().copied().unwrap_or(0),
            depth,
            trace_parent_group: trace.group,
            state_native_id: Some(native_id),
            schema: false,
            space_checker,
        };
        let transition = (|| {
            let (next_payload, used) = if has_implementation {
                let used = self.payload_references(&implementation)?;
                let terms = self.run_sayo_control_payload(&implementation, context, trace)?;
                (terms, used)
            } else {
                (Vec::new(), HashSet::new())
            };

            let mut replacement = payload
                .iter()
                .enumerate()
                .filter(|(index, _)| !used.contains(index))
                .map(|(_, id)| self.term_for_id(*id))
                .collect::<Result<Vec<_>>>()?;

            if zero_state_transition || self.is_capture_backed_template(&result_template) {
                let next = self.register_template_state(
                    &result_template,
                    next_payload,
                    depth,
                    Some(context),
                    trace,
                )?;
                replacement.push(next);
            } else {
                let Some(anchor) = next_payload.last() else {
                    return Err(Error::Implementation(format!(
                        "Template `{template}` {slot} produced no `{result_template}` anchor"
                    )));
                };
                if result_template != ".any" && anchor.space != result_template {
                    return Err(Error::Implementation(format!(
                        "Template `{template}` {slot} expected `{result_template}` anchor, got `{}`",
                        anchor.space
                    )));
                }
                replacement.extend(next_payload);
            }
            Ok(Some(replacement))
        })();
        match transition {
            Ok(Some(terms)) => NativeStateTransition::Replace(terms),
            Ok(None) => NativeStateTransition::NoStep,
            Err(error) => NativeStateTransition::Trap(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_template_control(
        &self,
        native_id: NativeTermId,
        template: &str,
        payload: &[TermId],
        space_checker: bool,
        slot: &str,
        depth: usize,
        trace_group: Option<TraceGroupId>,
    ) -> PreparedControl {
        let mut selected = None;
        for component in self.component_roots(template) {
            let root = match component.join(slot) {
                Ok(root) => root,
                Err(error) => {
                    return PreparedControl::Ready(NativeStateTransition::Trap(error));
                }
            };
            if !matches!(root.is_dir(), Ok(true)) {
                continue;
            }
            let results = match root.directories().map(|results| {
                results
                    .into_iter()
                    .filter(|result| {
                        result
                            .file_name()
                            .is_some_and(|name| name == ".any" || !name.starts_with('.'))
                    })
                    .collect::<Vec<_>>()
            }) {
                Ok(results) => results,
                Err(error) => {
                    return PreparedControl::Ready(NativeStateTransition::Trap(error));
                }
            };
            match results.as_slice() {
                [] => continue,
                [result] => {
                    selected = Some(result.clone());
                    break;
                }
                _ => {
                    return PreparedControl::Ready(NativeStateTransition::Trap(Error::Relation(
                        format!(
                            "ambiguous Template control Relation for `{template}` {slot}: {}",
                            results
                                .iter()
                                .filter_map(VirtualPath::file_name)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )));
                }
            }
        }
        let Some(result) = selected else {
            return PreparedControl::Ready(NativeStateTransition::NoStep);
        };
        let result_template = result.file_name().unwrap_or("").to_owned();
        let implementation = match result.join("main.sayo") {
            Ok(implementation) => implementation,
            Err(error) => {
                return PreparedControl::Ready(NativeStateTransition::Trap(error));
            }
        };
        let has_implementation = match implementation.is_file() {
            Ok(value) => value,
            Err(error) => {
                return PreparedControl::Ready(NativeStateTransition::Trap(error));
            }
        };
        let zero_state_transition = !has_implementation && result_template != "Vanish";
        if zero_state_transition && matches!(result_template.as_str(), "Trap" | ".any") {
            return PreparedControl::Ready(NativeStateTransition::Trap(Error::Implementation(
                format!(
                    "Template `{template}` {slot} with Result `{result_template}` requires main.sayo"
                ),
            )));
        }
        let context = RelationContext {
            input_term_id: payload.first().copied().unwrap_or(0),
            depth,
            trace_parent_group: trace_group,
            state_native_id: Some(native_id),
            schema: false,
            space_checker,
        };
        if !has_implementation {
            let mut recorder = None;
            let mut trace = TraceContext {
                recorder: &mut recorder,
                group: trace_group,
            };
            return PreparedControl::Ready(self.finish_template_control(
                payload,
                HashSet::new(),
                &result_template,
                zero_state_transition,
                Vec::new(),
                depth,
                context,
                &mut trace,
            ));
        }
        let source = match implementation.read() {
            Ok(Some(source)) => source,
            Ok(None) => unreachable!("implementation presence checked"),
            Err(error) => {
                return PreparedControl::Ready(NativeStateTransition::Trap(error));
            }
        };
        let expression = match parser::parse(&source) {
            Ok(expression) => expression,
            Err(error) => {
                return PreparedControl::Ready(NativeStateTransition::Trap(error));
            }
        };
        let used = match self.payload_references(&implementation) {
            Ok(used) => used,
            Err(error) => {
                return PreparedControl::Ready(NativeStateTransition::Trap(error));
            }
        };
        PreparedControl::Sayo(Box::new(SayoControlCall {
            base: SourcePath::world(
                implementation
                    .parent()
                    .unwrap_or_else(|| self.world.clone()),
            ),
            implementation,
            context,
            payload: payload.to_vec(),
            used,
            result_template,
            zero_state_transition,
            slots: application_terms(&expression),
            depth,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_template_control(
        &self,
        payload: &[TermId],
        used: HashSet<usize>,
        result_template: &str,
        zero_state_transition: bool,
        next_payload: Vec<Term>,
        depth: usize,
        context: RelationContext,
        trace: &mut TraceContext<'_>,
    ) -> NativeStateTransition {
        let result = (|| {
            let mut replacement = payload
                .iter()
                .enumerate()
                .filter(|(index, _)| !used.contains(index))
                .map(|(_, id)| self.term_for_id(*id))
                .collect::<Result<Vec<_>>>()?;
            if zero_state_transition || self.is_capture_backed_template(result_template) {
                replacement.push(self.register_template_state(
                    result_template,
                    next_payload,
                    depth,
                    Some(context),
                    trace,
                )?);
            } else {
                let Some(anchor) = next_payload.last() else {
                    return Err(Error::Implementation(format!(
                        "Template control produced no `{result_template}` anchor"
                    )));
                };
                if result_template != ".any" && anchor.space != result_template {
                    return Err(Error::Implementation(format!(
                        "Template control expected `{result_template}` anchor, got `{}`",
                        anchor.space
                    )));
                }
                replacement.extend(next_payload);
            }
            Ok(replacement)
        })();
        match result {
            Ok(terms) => NativeStateTransition::Replace(terms),
            Err(error) => NativeStateTransition::Trap(error),
        }
    }

    fn is_capture_backed_template(&self, template: &str) -> bool {
        if template == "Vanish" {
            return true;
        }
        if is_native_primitive_space(template) {
            return false;
        }
        self.component_root(template).is_some_and(|root| {
            let file = |name| {
                root.join(name)
                    .and_then(|path| path.is_file())
                    .unwrap_or(false)
            };
            file("template.sayo") || (!file("main.sayo") && !file("space.sayo"))
        })
    }

    fn payload_references(&self, implementation: &VirtualPath) -> Result<HashSet<usize>> {
        let mut references = HashSet::new();
        let mut visited = Vec::new();
        Self::collect_payload_references_from_file(implementation, &mut references, &mut visited)?;
        Ok(references)
    }

    fn collect_payload_references_from_file(
        path: &VirtualPath,
        references: &mut HashSet<usize>,
        visited: &mut Vec<VirtualPath>,
    ) -> Result<()> {
        if visited.iter().any(|visited| visited == path) {
            return Ok(());
        }
        visited.push(path.clone());
        let source = path
            .read()?
            .ok_or_else(|| Error::Source(format!("cannot read Sayo source {}", path.display())))?;
        let expression = parser::parse(&source)?;
        Self::collect_payload_references(&expression, references);

        let mut files = Vec::new();
        Self::collect_sayo_file_references(&expression, &mut files);
        let base = path.parent().unwrap_or_else(|| path.clone());
        for file in files {
            let referenced = base.resolve(file)?;
            Self::collect_payload_references_from_file(&referenced, references, visited)?;
        }
        Ok(())
    }

    fn collect_sayo_file_references(expression: &Expression, files: &mut Vec<PathBuf>) {
        match expression {
            Expression::Atom(source) if source.ends_with(".sayo") => {
                files.push(PathBuf::from(source));
            }
            Expression::State { payload, .. } => {
                for expression in payload {
                    Self::collect_sayo_file_references(expression, files);
                }
            }
            Expression::Apply(left, right) => {
                Self::collect_sayo_file_references(left, files);
                Self::collect_sayo_file_references(right, files);
            }
            Expression::Group(inner) => Self::collect_sayo_file_references(inner, files),
            Expression::Atom(_) | Expression::StringLiteral(_) | Expression::PayloadRef(_) => {}
        }
    }

    fn run_sayo_control_payload(
        &self,
        implementation: &VirtualPath,
        context: RelationContext,
        trace: &mut TraceContext<'_>,
    ) -> Result<Vec<Term>> {
        let source = implementation.read()?.ok_or_else(|| {
            Error::Source(format!(
                "cannot read Sayo source {}",
                implementation.display()
            ))
        })?;
        let expression = parser::parse(&source)?;
        let slots = application_terms(&expression);
        let base = SourcePath::world(
            implementation
                .parent()
                .unwrap_or_else(|| self.world.clone()),
        );
        let mut payload = Vec::with_capacity(slots.len());
        for slot in slots {
            let mut stack = vec![SourcePath::world(implementation.clone())];
            let normal_form = {
                let mut relation_trace = TraceContext {
                    recorder: &mut *trace.recorder,
                    group: context.trace_parent_group,
                };
                self.evaluate_expression(
                    &slot,
                    &base,
                    &mut stack,
                    Some(context),
                    context.depth,
                    &mut relation_trace,
                )?
            };
            let count = normal_form.terms().len();
            let term = normal_form.single().map_err(|_| {
                Error::Implementation(format!(
                    "Template control payload slot must produce exactly one Term, produced {count}: {}",
                    implementation.display()
                ))
            })?;
            payload.push(term);
        }
        Ok(payload)
    }

    fn collect_payload_references(expression: &Expression, references: &mut HashSet<usize>) {
        match expression {
            Expression::PayloadRef(index) => {
                references.insert(*index);
            }
            Expression::State { payload, .. } => {
                for expression in payload {
                    Self::collect_payload_references(expression, references);
                }
            }
            Expression::Apply(left, right) => {
                Self::collect_payload_references(left, references);
                Self::collect_payload_references(right, references);
            }
            Expression::Group(inner) => Self::collect_payload_references(inner, references),
            Expression::Atom(_) | Expression::StringLiteral(_) => {}
        }
    }

    fn register_native_emitter(
        &self,
        captured: TermId,
        twice: bool,
        evaluation_depth: usize,
    ) -> Result<Term> {
        let native_id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        let state = if twice {
            NativeTermState::EmitTwice { captured }
        } else {
            NativeTermState::EmitOnce { captured }
        };
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(native_id, state);
        let value_display = self.term_for_id(captured)?.display;
        let name = if twice { "EmitTwice" } else { "EmitOnce" };
        self.store_term(Term {
            term_id: None,
            evaluation_depth,
            source: String::new(),
            display: format!("{name}{{{value_display}}}"),
            space: "Duplicate".into(),
            implementation: TermImplementation::Native(native_id),
        })
    }

    fn register_template_state(
        &self,
        template: &str,
        payload: Vec<Term>,
        evaluation_depth: usize,
        parent_context: Option<RelationContext>,
        trace: &mut TraceContext<'_>,
    ) -> Result<Term> {
        self.validate_static_template_arity(template, payload.len())?;
        let payload_ids = payload
            .into_iter()
            .map(|term| {
                term.term_id.ok_or_else(|| {
                    Error::Implementation("template payload term has no TermId".into())
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let native_id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(
                native_id,
                NativeTermState::Template {
                    template: template.to_owned(),
                    payload: payload_ids.clone(),
                    schema: parent_context
                        .as_ref()
                        .is_some_and(|context| context.schema),
                    space_checker: parent_context
                        .as_ref()
                        .is_some_and(|context| context.space_checker),
                },
            );
        let payload_display = payload_ids
            .iter()
            .map(|id| self.term_for_id(*id).map(|term| term.display))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let term = self.store_term(Term {
            term_id: None,
            evaluation_depth,
            source: format!("{template} {{ {payload_display} }}"),
            display: format!("{template}{{{payload_display}}}"),
            space: template.to_owned(),
            implementation: TermImplementation::Native(native_id),
        })?;
        if let Some(schema) = self.find_template_schema(template) {
            self.validate_template_schema(&schema, &term, &payload_ids, evaluation_depth, trace)?;
        }
        let _ = parent_context;
        Ok(term)
    }

    fn find_template_schema(&self, template: &str) -> Option<VirtualPath> {
        self.component_roots(template)
            .into_iter()
            .filter_map(|root| root.join("template.sayo").ok())
            .find(|schema| schema.is_file().unwrap_or(false))
    }

    fn validate_template_schema(
        &self,
        schema: &VirtualPath,
        state: &Term,
        payload: &[TermId],
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<()> {
        // A schema has no external right input. Payload references are resolved
        // through `state_native_id`; the context input id is unused and may use
        // the sentinel for a zero-payload state.
        let input_term_id = payload.first().copied().unwrap_or(0);
        let native_id = match state.implementation {
            TermImplementation::Native(id) => id,
            _ => return Err(Error::Implementation("schema state is not Native".into())),
        };
        let context = RelationContext {
            input_term_id,
            depth,
            trace_parent_group: trace.group,
            state_native_id: Some(native_id),
            schema: true,
            space_checker: false,
        };
        let result = self.run_sayo_relation(schema, context, trace)?;
        match result {
            RelationOutcome::Success(_) => Ok(()),
            RelationOutcome::NotApplicable => Err(Error::Implementation(
                "Template schema returned NoStep".into(),
            )),
            RelationOutcome::Trap(error) => Err(error),
        }
    }

    fn validate_static_template_arity(&self, template: &str, payload_len: usize) -> Result<()> {
        if template == "Vanish" {
            return Ok(());
        }
        let expected = match template {
            "Pass" => 0,
            "CheckSpace" => 2,
            _ => self
                .template_arities
                .get(template)
                .copied()
                .ok_or_else(|| {
                    Error::Implementation(format!(
                        "Template `{template}` has no statically derived payload arity"
                    ))
                })?,
        };
        if expected != payload_len {
            return Err(Error::Implementation(format!(
                "Template `{template}` has static payload arity {expected}, got {payload_len}"
            )));
        }
        Ok(())
    }

    #[cfg(test)]
    fn register_native_self_loop(&self) -> Result<Term> {
        let native_id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(native_id, NativeTermState::SelfLoop);
        self.store_term(Term {
            term_id: None,
            evaluation_depth: 0,
            source: String::new(),
            display: "SelfLoop".into(),
            space: "SelfLoop".into(),
            implementation: TermImplementation::Native(native_id),
        })
    }

    fn register_native_each(&self, transform: TermId, evaluation_depth: usize) -> Result<Term> {
        self.register_native_each_state(
            NativeTermState::Each { transform },
            transform,
            None,
            evaluation_depth,
        )
    }

    fn register_native_fold(
        &self,
        state: NativeTermState,
        display: String,
        depth: usize,
    ) -> Result<Term> {
        let id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(id, state);
        self.store_term(Term {
            term_id: None,
            evaluation_depth: depth,
            source: String::new(),
            display,
            space: "FoldAccum".into(),
            implementation: TermImplementation::Native(id),
        })
    }
    fn register_native_fold_accum(
        &self,
        transform: TermId,
        accumulator: TermId,
        depth: usize,
    ) -> Result<Term> {
        self.register_native_fold(
            NativeTermState::FoldAccum {
                transform,
                accumulator,
            },
            format!(
                "FoldAccum{{{}, {}}}",
                self.term_for_id(transform)?.display,
                self.term_for_id(accumulator)?.display
            ),
            depth,
        )
    }
    fn register_native_fold_apply(
        &self,
        transform: TermId,
        accumulator: TermId,
        input: TermId,
        depth: usize,
    ) -> Result<Term> {
        self.register_native_fold(
            NativeTermState::FoldApply {
                transform,
                accumulator,
                input,
            },
            format!(
                "FoldApply{{{}, {}, {}}}",
                self.term_for_id(transform)?.display,
                self.term_for_id(accumulator)?.display,
                self.term_for_id(input)?.display
            ),
            depth,
        )
    }
    fn register_native_fold_next(
        &self,
        transform: TermId,
        accumulator: TermId,
        depth: usize,
    ) -> Result<Term> {
        self.register_native_fold(
            NativeTermState::FoldNext {
                transform,
                accumulator,
            },
            format!(
                "FoldNext{{{}, {}}}",
                self.term_for_id(transform)?.display,
                self.term_for_id(accumulator)?.display
            ),
            depth,
        )
    }
    fn register_native_fold_close(&self, accumulator: TermId, depth: usize) -> Result<Term> {
        self.register_native_fold(
            NativeTermState::FoldClose { accumulator },
            format!("FoldClose{{{}}}", self.term_for_id(accumulator)?.display),
            depth,
        )
    }

    fn register_native_reduce(
        &self,
        state: NativeTermState,
        display: String,
        depth: usize,
    ) -> Result<Term> {
        let id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(id, state);
        self.store_term(Term {
            term_id: None,
            evaluation_depth: depth,
            source: String::new(),
            display,
            space: "ReduceAccum".into(),
            implementation: TermImplementation::Native(id),
        })
    }

    fn register_native_reduce_accum(
        &self,
        transform: TermId,
        accumulator: TermId,
        depth: usize,
    ) -> Result<Term> {
        self.register_native_reduce(
            NativeTermState::ReduceAccum {
                transform,
                accumulator,
            },
            format!(
                "ReduceAccum{{{}, {}}}",
                self.term_for_id(transform)?.display,
                self.term_for_id(accumulator)?.display
            ),
            depth,
        )
    }

    fn register_native_reduce_apply(
        &self,
        transform: TermId,
        accumulator: TermId,
        input: TermId,
        depth: usize,
    ) -> Result<Term> {
        self.register_native_reduce(
            NativeTermState::ReduceApply {
                transform,
                accumulator,
                input,
            },
            format!(
                "ReduceApply{{{}, {}, {}}}",
                self.term_for_id(transform)?.display,
                self.term_for_id(accumulator)?.display,
                self.term_for_id(input)?.display
            ),
            depth,
        )
    }

    fn register_native_reduce_next(
        &self,
        transform: TermId,
        accumulator: TermId,
        depth: usize,
    ) -> Result<Term> {
        self.register_native_reduce(
            NativeTermState::ReduceNext {
                transform,
                accumulator,
            },
            format!(
                "ReduceNext{{{}, {}}}",
                self.term_for_id(transform)?.display,
                self.term_for_id(accumulator)?.display
            ),
            depth,
        )
    }

    fn register_native_reduce_close(&self, accumulator: TermId, depth: usize) -> Result<Term> {
        self.register_native_reduce(
            NativeTermState::ReduceClose { accumulator },
            format!("ReduceClose{{{}}}", self.term_for_id(accumulator)?.display),
            depth,
        )
    }

    fn register_native_each_apply(
        &self,
        transform: TermId,
        input: TermId,
        evaluation_depth: usize,
    ) -> Result<Term> {
        self.register_native_each_state(
            NativeTermState::EachApply { transform, input },
            transform,
            Some(input),
            evaluation_depth,
        )
    }

    fn register_native_each_emit(
        &self,
        transform: TermId,
        value: TermId,
        evaluation_depth: usize,
    ) -> Result<Term> {
        self.register_native_each_state(
            NativeTermState::EachEmit { transform, value },
            transform,
            Some(value),
            evaluation_depth,
        )
    }

    fn register_native_each_state(
        &self,
        state: NativeTermState,
        transform: TermId,
        argument: Option<TermId>,
        evaluation_depth: usize,
    ) -> Result<Term> {
        let native_id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(native_id, state.clone());
        let transform_display = self.term_for_id(transform)?.display;
        let display = match state {
            NativeTermState::Each { .. } => format!("Each{{{transform_display}}}"),
            NativeTermState::EachApply { .. } => format!(
                "EachApply{{{transform_display}, {}}}",
                self.term_for_id(argument.expect("EachApply has an input"))?
                    .display
            ),
            NativeTermState::EachEmit { .. } => format!(
                "EachEmit{{{transform_display}, {}}}",
                self.term_for_id(argument.expect("EachEmit has a value"))?
                    .display
            ),
            _ => unreachable!("Each state registration only accepts Each states"),
        };
        self.store_term(Term {
            term_id: None,
            evaluation_depth,
            source: String::new(),
            display,
            space: "Each".into(),
            implementation: TermImplementation::Native(native_id),
        })
    }

    /// Apply two already-materialized Terms once through the normal Relation
    /// resolver. `For` uses this instead of knowing anything about Native,
    /// Sayo implementations.
    fn apply_transform_once(
        &self,
        transform: TermId,
        input: TermId,
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<Term> {
        let transform_term = self.term_for_id(transform)?;
        let input_term = self.term_for_id(input)?;
        match self.reduce_pair(
            transform_term,
            input_term,
            Some(RelationContext {
                input_term_id: input,
                depth,
                trace_parent_group: trace.group,
                state_native_id: None,
                schema: false,
                space_checker: false,
            }),
            trace,
        ) {
            ReducerOutcome::Success { terms, .. } => {
                let mut terms = self.normalize_materialized_terms(terms, depth, trace)?;
                if terms.len() == 1 {
                    Ok(terms.pop().expect("one-Term transform result"))
                } else {
                    Err(Error::Implementation(format!(
                        "For transform must produce exactly one Term, produced {}",
                        terms.len()
                    )))
                }
            }
            ReducerOutcome::NoStep(_) => Err(Error::Implementation(
                "For transform is not applicable to this input".into(),
            )),
            ReducerOutcome::Trap(error) => Err(error),
        }
    }

    fn normalize_materialized_terms(
        &self,
        terms: Vec<Term>,
        depth: usize,
        trace: &mut TraceContext<'_>,
    ) -> Result<Vec<Term>> {
        let mut cells = terms
            .into_iter()
            .map(|term| ReductionCell::term(term, None))
            .collect::<Vec<_>>();
        if cells.is_empty() {
            return Ok(Vec::new());
        }
        let mut stack = Vec::new();
        let base = SourcePath::world(self.world.clone());
        self.reduce_application_group(&mut cells, &base, &mut stack, None, depth, trace)
            .map(NormalForm::into_terms)
    }

    fn filter_choice(&self, term: &Term) -> Result<FilterChoice> {
        if let TermImplementation::Native(id) = term.implementation
            && let Some(NativeTermState::FilterChoice(kind)) = self
                .native_terms
                .lock()
                .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
                .get(&id)
        {
            return Ok(*kind);
        }
        match term.space.as_str() {
            "Keep" => Ok(FilterChoice::Keep),
            "Drop" => Ok(FilterChoice::Drop),
            space => Err(Error::Implementation(format!(
                "Filter predicate must return Keep or Drop, got {space}"
            ))),
        }
    }

    fn register_native_sieve(&self, predicate: TermId, depth: usize) -> Result<Term> {
        self.register_native_sieve_state(
            NativeTermState::Sieve { predicate },
            predicate,
            None,
            depth,
        )
    }

    fn register_native_sieve_apply(
        &self,
        predicate: TermId,
        input: TermId,
        depth: usize,
    ) -> Result<Term> {
        self.register_native_sieve_state(
            NativeTermState::SieveApply { predicate, input },
            predicate,
            Some(input),
            depth,
        )
    }

    fn register_native_sieve_resolution(
        &self,
        predicate: TermId,
        input: TermId,
        kind: FilterChoice,
        depth: usize,
    ) -> Result<Term> {
        let state = match kind {
            FilterChoice::Keep => NativeTermState::SieveKeep { predicate, input },
            FilterChoice::Drop => NativeTermState::SieveDrop { predicate },
        };
        self.register_native_sieve_state(state, predicate, Some(input), depth)
    }

    fn register_native_sieve_state(
        &self,
        state: NativeTermState,
        predicate: TermId,
        input: Option<TermId>,
        depth: usize,
    ) -> Result<Term> {
        let id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(id, state.clone());
        let predicate = self.term_for_id(predicate)?.display;
        let display = match state {
            NativeTermState::Sieve { .. } => format!("Sieve{{{predicate}}}"),
            NativeTermState::SieveApply { .. } => format!(
                "SieveApply{{{predicate}, {}}}",
                self.term_for_id(input.expect("input"))?.display
            ),
            NativeTermState::SieveKeep { .. } => format!(
                "SieveKeep{{{predicate}, {}}}",
                self.term_for_id(input.expect("input"))?.display
            ),
            NativeTermState::SieveDrop { .. } => format!("SieveDrop{{{predicate}}}"),
            _ => unreachable!("Sieve state"),
        };
        self.store_term(Term {
            term_id: None,
            evaluation_depth: depth,
            source: String::new(),
            display,
            space: "Sieve".into(),
            implementation: TermImplementation::Native(id),
        })
    }

    fn register_native_filter_choice(&self, kind: FilterChoice, depth: usize) -> Result<Term> {
        let id = self.next_native_term_id.fetch_add(1, Ordering::Relaxed);
        self.native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(id, NativeTermState::FilterChoice(kind));
        let (source, space) = match kind {
            FilterChoice::Keep => ("keep", "Keep"),
            FilterChoice::Drop => ("drop", "Drop"),
        };
        self.store_term(Term {
            term_id: None,
            evaluation_depth: depth,
            source: source.into(),
            display: space.into(),
            space: space.into(),
            implementation: TermImplementation::Native(id),
        })
    }

    fn reduce_pair(
        &self,
        left: Term,
        right: Term,
        context: Option<RelationContext>,
        trace: &mut TraceContext<'_>,
    ) -> ReducerOutcome {
        match self.prepare_pair(left, right, context, trace) {
            PreparedPair::Ready(outcome) => outcome,
            PreparedPair::Sayo(call) => {
                let output = self.run_sayo_relation(&call.implementation, call.context, trace);
                self.finish_pair(call.left, call.right, call.relation, output)
            }
        }
    }

    fn prepare_pair(
        &self,
        left: Term,
        right: Term,
        parent_context: Option<RelationContext>,
        trace: &mut TraceContext<'_>,
    ) -> PreparedPair {
        let mut relations = match self.relations(&left.space, &right.space) {
            Ok(relations) => relations,
            Err(error) => return PreparedPair::Ready(ReducerOutcome::Trap(error)),
        };
        // A user-defined Template Relation is allowed to replace the
        // bootstrap Native bridge for that Template.  Keep Native fallback
        // only when no World implementation exists.
        if self.is_template_state(&left)
            && relations.iter().any(|relation| {
                !matches!(relation.implementation, RelationImplementation::Native(_))
            })
        {
            relations.retain(|relation| {
                !matches!(relation.implementation, RelationImplementation::Native(_))
            });
        }
        let relation = match relations.as_slice() {
            [] => {
                return PreparedPair::Ready(ReducerOutcome::NoStep(format!(
                    "no Relation for {} {}",
                    left.space, right.space
                )));
            }
            [relation] => relation.clone(),
            _ => {
                return PreparedPair::Ready(ReducerOutcome::Trap(Error::Relation(format!(
                    "ambiguous Relation for {} {}: {}",
                    left.space,
                    right.space,
                    relations
                        .iter()
                        .map(|item| {
                            format!("{} ({})", item.result_space, item.implementation_label())
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ))));
            }
        };
        let Some(input_term_id) = right.term_id else {
            return PreparedPair::Ready(ReducerOutcome::Trap(Error::Implementation(
                "right term has no TermId".into(),
            )));
        };
        let depth = parent_context
            .map(|context| context.depth.saturating_add(1))
            .unwrap_or_else(|| right.evaluation_depth.saturating_add(1));
        if depth > self.limits.relation_depth {
            return PreparedPair::Ready(ReducerOutcome::Trap(Error::BootstrapLimit(format!(
                "Relation evaluation depth exceeded: limit={}",
                self.limits.relation_depth
            ))));
        }
        let context = RelationContext {
            input_term_id,
            depth,
            trace_parent_group: trace.group,
            state_native_id: match &left.implementation {
                TermImplementation::Native(native_id) => Some(*native_id),
                _ => None,
            },
            schema: parent_context.is_some_and(|context| context.schema),
            space_checker: parent_context.is_some_and(|context| context.space_checker),
        };
        let output = match (&left.implementation, &relation.implementation) {
            (_, RelationImplementation::Capture) => {
                let mut payload = match self.template_payload_ids(&left) {
                    Ok(Some(ids)) => match ids
                        .into_iter()
                        .map(|id| self.term_for_id(id))
                        .collect::<Result<Vec<_>>>()
                    {
                        Ok(payload) => payload,
                        Err(error) => {
                            return PreparedPair::Ready(ReducerOutcome::Trap(error));
                        }
                    },
                    Ok(None) => Vec::new(),
                    Err(error) => return PreparedPair::Ready(ReducerOutcome::Trap(error)),
                };
                payload.push(right.clone());
                match self.register_template_state(
                    &relation.result_space,
                    payload,
                    depth,
                    Some(context),
                    trace,
                ) {
                    Ok(term) => Ok(RelationOutcome::Success(RelationResult::Terms(vec![term]))),
                    Err(error) => Err(error),
                }
            }
            (TermImplementation::Native(left_id), RelationImplementation::Native(_)) => {
                self.run_native_term(*left_id, input_term_id, context)
            }
            (TermImplementation::Native(_), _) => match &relation.implementation {
                RelationImplementation::Sayo(path) => {
                    return PreparedPair::Sayo(Box::new(SayoPairCall {
                        left: left.clone(),
                        right: right.clone(),
                        relation: relation.clone(),
                        implementation: path.clone(),
                        context,
                    }));
                }
                RelationImplementation::Capture => unreachable!(),
                RelationImplementation::Native(_) => unreachable!(),
            },
            (TermImplementation::Sayo(path), _) => {
                return PreparedPair::Sayo(Box::new(SayoPairCall {
                    left: left.clone(),
                    right: right.clone(),
                    relation: relation.clone(),
                    implementation: path.clone(),
                    context,
                }));
            }
            (TermImplementation::None, _) => match &relation.implementation {
                RelationImplementation::Sayo(path) => {
                    return PreparedPair::Sayo(Box::new(SayoPairCall {
                        left: left.clone(),
                        right: right.clone(),
                        relation: relation.clone(),
                        implementation: path.clone(),
                        context,
                    }));
                }
                _ => self.run_relation(&relation.implementation, context, trace),
            },
        };
        PreparedPair::Ready(self.finish_pair(left, right, relation, output))
    }

    fn finish_pair(
        &self,
        left: Term,
        right: Term,
        relation: Relation,
        output: Result<RelationOutcome>,
    ) -> ReducerOutcome {
        let output = match output {
            Ok(output) => output,
            Err(error) => return ReducerOutcome::Trap(error),
        };
        let terms = match output {
            RelationOutcome::Success(output) => match output {
                RelationResult::Term(term_id) => match self.term_for_id(term_id) {
                    Ok(term) => vec![term],
                    Err(error) => return ReducerOutcome::Trap(error),
                },
                RelationResult::Terms(terms) => terms,
            },
            RelationOutcome::NotApplicable => {
                return ReducerOutcome::NoStep(format!(
                    "Relation {} {} : {} is not applicable",
                    left.space, right.space, relation.result_space
                ));
            }
            RelationOutcome::Trap(error) => return ReducerOutcome::Trap(error),
        };
        let Some(anchor) = terms.last() else {
            return ReducerOutcome::Trap(Error::Implementation(format!(
                "Relation {} {} : {} produced an empty replacement",
                left.space, right.space, relation.result_space
            )));
        };
        if relation.result_space != ".any" && anchor.space != relation.result_space {
            return ReducerOutcome::Trap(Error::Implementation(format!(
                "Relation {} {} : {} produced anchor in Space {}",
                left.space, right.space, relation.result_space, anchor.space
            )));
        }
        ReducerOutcome::Success {
            terms,
            relation: relation.trace(&left, &right),
        }
    }

    fn relations(&self, left: &str, right: &str) -> Result<Vec<Relation>> {
        let mut relations = Vec::new();
        if let Some((root_index, component)) = self.visible_component(left) {
            let base_priority = (root_index as u8) * 4;
            let concrete = component.join(right)?;
            self.collect_world_relations(
                &concrete,
                base_priority + 1,
                base_priority + 2,
                &mut relations,
            )?;
            let fallback = component.join(".any")?;
            self.collect_world_relations(
                &fallback,
                base_priority + 3,
                base_priority + 4,
                &mut relations,
            )?;
        }
        let native_relations = self.native_relations(left, right);
        relations.extend(native_relations);
        let best_priority = relations.iter().map(|relation| relation.priority).min();
        Ok(best_priority
            .map(|priority| {
                relations
                    .into_iter()
                    .filter(|relation| relation.priority == priority)
                    .collect()
            })
            .unwrap_or_default())
    }

    fn collect_world_relations(
        &self,
        root: &VirtualPath,
        fixed_priority: u8,
        dynamic_priority: u8,
        relations: &mut Vec<Relation>,
    ) -> Result<()> {
        if !root.is_dir()? {
            return Ok(());
        }
        let results = root.directories()?;
        let has_dynamic = results
            .iter()
            .any(|result| result.file_name() == Some(".any"));
        let has_fixed = results.iter().any(|result| {
            result
                .file_name()
                .is_some_and(|name| !name.starts_with('.'))
        });
        if has_dynamic && has_fixed {
            return Err(Error::World(format!(
                "fixed and dynamic Result Relations cannot coexist: {}",
                root.display()
            )));
        }
        for result in results {
            if result.join("main.native")?.is_file()? {
                return Err(Error::World(
                    "main.native declarations are not allowed; Native Relations are provided by SayoVM"
                        .into(),
                ));
            }
            let sayo = result.join("main.sayo")?;
            let dynamic = result.file_name() == Some(".any");
            let implementation = if sayo.is_file()? {
                Some(RelationImplementation::Sayo(sayo))
            } else {
                None
            };
            let implementation =
                implementation.or_else(|| (!dynamic).then_some(RelationImplementation::Capture));
            if let Some(implementation) = implementation {
                relations.push(Relation {
                    result_space: result.file_name().unwrap().to_owned(),
                    implementation,
                    priority: if dynamic {
                        dynamic_priority
                    } else {
                        fixed_priority
                    },
                });
            }
        }
        Ok(())
    }

    fn native_relations(&self, left: &str, right: &str) -> Vec<Relation> {
        self.native_registry
            .candidates(left, right)
            .map(|relation| Relation {
                result_space: relation.result_space.to_owned(),
                implementation: RelationImplementation::Native(relation.id),
                priority: 9,
            })
            .collect()
    }

    fn run_native_term(
        &self,
        native_id: NativeTermId,
        input_term_id: TermId,
        context: RelationContext,
    ) -> Result<RelationOutcome> {
        let state = self
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .get(&native_id)
            .cloned()
            .ok_or_else(|| Error::Implementation(format!("unknown Native Term: {native_id}")))?;
        match state {
            NativeTermState::Arithmetic { operation, .. } => {
                let mut native_context = NativeContext {
                    runtime: self,
                    depth: context.depth,
                    native_term_id: Some(native_id),
                    space_checker: context.space_checker,
                };
                match native_apply_int(&mut native_context, input_term_id, operation)? {
                    NativeOutcome::Success(term_id) => {
                        Ok(RelationOutcome::Success(RelationResult::Term(term_id)))
                    }
                    NativeOutcome::NotApplicable => Ok(RelationOutcome::NotApplicable),
                    NativeOutcome::Trap(error) => Ok(RelationOutcome::Trap(error)),
                }
            }
            NativeTermState::Comparison { .. } => {
                let mut native_context = NativeContext {
                    runtime: self,
                    depth: context.depth,
                    native_term_id: Some(native_id),
                    space_checker: context.space_checker,
                };
                match native_compare_apply_int(&mut native_context, input_term_id)? {
                    NativeOutcome::Success(term_id) => {
                        Ok(RelationOutcome::Success(RelationResult::Term(term_id)))
                    }
                    NativeOutcome::NotApplicable => Ok(RelationOutcome::NotApplicable),
                    NativeOutcome::Trap(error) => Ok(RelationOutcome::Trap(error)),
                }
            }
            NativeTermState::Ordering => Ok(RelationOutcome::Trap(Error::Implementation(
                "an Ordering result has no built-in binary Relation input".into(),
            ))),
            NativeTermState::StringComparison { .. } => {
                let mut native_context = NativeContext {
                    runtime: self,
                    depth: context.depth,
                    native_term_id: Some(native_id),
                    space_checker: context.space_checker,
                };
                match native_compare_apply_string(&mut native_context, input_term_id)? {
                    NativeOutcome::Success(term_id) => {
                        Ok(RelationOutcome::Success(RelationResult::Term(term_id)))
                    }
                    NativeOutcome::NotApplicable => Ok(RelationOutcome::NotApplicable),
                    NativeOutcome::Trap(error) => Ok(RelationOutcome::Trap(error)),
                }
            }
            NativeTermState::StringEquality { .. } => {
                let mut native_context = NativeContext {
                    runtime: self,
                    depth: context.depth,
                    native_term_id: Some(native_id),
                    space_checker: context.space_checker,
                };
                match native_equal_string_apply(&mut native_context, input_term_id)? {
                    NativeOutcome::Success(term_id) => {
                        Ok(RelationOutcome::Success(RelationResult::Term(term_id)))
                    }
                    NativeOutcome::NotApplicable => Ok(RelationOutcome::NotApplicable),
                    NativeOutcome::Trap(error) => Ok(RelationOutcome::Trap(error)),
                }
            }
            NativeTermState::EmitTwice { .. } | NativeTermState::EmitOnce { .. } => {
                Ok(RelationOutcome::Trap(Error::Implementation(
                    "an internal emitter cannot receive a binary Relation input".into(),
                )))
            }
            NativeTermState::Each { transform } => {
                let term =
                    self.register_native_each_apply(transform, input_term_id, context.depth)?;
                let term_id = term.term_id.ok_or_else(|| {
                    Error::Implementation("EachApply state was not assigned a TermId".into())
                })?;
                Ok(RelationOutcome::Success(RelationResult::Term(term_id)))
            }
            NativeTermState::EachApply { .. } | NativeTermState::EachEmit { .. } => {
                Ok(RelationOutcome::Trap(Error::Implementation(
                    "an internal Each state cannot receive a binary Relation input".into(),
                )))
            }
            NativeTermState::Sieve { predicate } => {
                let term =
                    self.register_native_sieve_apply(predicate, input_term_id, context.depth)?;
                Ok(RelationOutcome::Success(RelationResult::Term(
                    term.term_id.expect("SieveApply id"),
                )))
            }
            NativeTermState::SieveApply { .. }
            | NativeTermState::SieveKeep { .. }
            | NativeTermState::SieveDrop { .. }
            | NativeTermState::FilterChoice(_) => Ok(RelationOutcome::Trap(Error::Implementation(
                "an internal Filter state cannot receive a binary Relation input".into(),
            ))),
            NativeTermState::FoldInit { transform } => {
                let term =
                    self.register_native_fold_accum(transform, input_term_id, context.depth)?;
                Ok(RelationOutcome::Success(RelationResult::Term(
                    term.term_id.expect("FoldAccum id"),
                )))
            }
            NativeTermState::FoldAccum {
                transform,
                accumulator,
            } => {
                let term = self.register_native_fold_apply(
                    transform,
                    accumulator,
                    input_term_id,
                    context.depth,
                )?;
                Ok(RelationOutcome::Success(RelationResult::Term(
                    term.term_id.expect("FoldApply id"),
                )))
            }
            NativeTermState::FoldApply { .. }
            | NativeTermState::FoldNext { .. }
            | NativeTermState::FoldClose { .. } => {
                Ok(RelationOutcome::Trap(Error::Implementation(
                    "an internal Fold state cannot receive a binary Relation input".into(),
                )))
            }
            NativeTermState::ReduceInit { transform } => {
                let term =
                    self.register_native_reduce_accum(transform, input_term_id, context.depth)?;
                Ok(RelationOutcome::Success(RelationResult::Term(
                    term.term_id.expect("ReduceAccum id"),
                )))
            }
            NativeTermState::ReduceAccum {
                transform,
                accumulator,
            } => {
                let term = self.register_native_reduce_apply(
                    transform,
                    accumulator,
                    input_term_id,
                    context.depth,
                )?;
                Ok(RelationOutcome::Success(RelationResult::Term(
                    term.term_id.expect("ReduceApply id"),
                )))
            }
            NativeTermState::ReduceApply { .. }
            | NativeTermState::ReduceNext { .. }
            | NativeTermState::ReduceClose { .. } => {
                Ok(RelationOutcome::Trap(Error::Implementation(
                    "an internal Reduce state cannot receive a binary Relation input".into(),
                )))
            }
            NativeTermState::Template {
                template, payload, ..
            } => {
                if template == "For" && payload.len() == 1 {
                    let term =
                        self.register_native_each_apply(payload[0], input_term_id, context.depth)?;
                    return Ok(RelationOutcome::Success(RelationResult::Term(
                        term.term_id.expect("EachApply id"),
                    )));
                }
                Ok(RelationOutcome::Trap(Error::Implementation(format!(
                    "Template `{template}` has no Input transition for this payload"
                ))))
            }
            #[cfg(test)]
            NativeTermState::SelfLoop => Ok(RelationOutcome::Trap(Error::Implementation(
                "a self-loop state cannot receive a binary Relation input".into(),
            ))),
        }
    }

    fn run_native_relation(
        &self,
        relation_id: NativeRelationId,
        input_term_id: TermId,
        context: RelationContext,
    ) -> Result<RelationOutcome> {
        let relation = self.native_registry.find(relation_id).ok_or_else(|| {
            Error::Implementation(format!("unknown Native Relation: {relation_id}"))
        })?;
        let mut native_context = NativeContext {
            runtime: self,
            depth: context.depth,
            native_term_id: None,
            space_checker: context.space_checker,
        };
        match (relation.handler)(&mut native_context, input_term_id)? {
            NativeOutcome::Success(term_id) => {
                Ok(RelationOutcome::Success(RelationResult::Term(term_id)))
            }
            NativeOutcome::NotApplicable => Ok(RelationOutcome::NotApplicable),
            NativeOutcome::Trap(error) => Ok(RelationOutcome::Trap(error)),
        }
    }

    fn store_term(&self, mut term: Term) -> Result<Term> {
        let id = self.next_term_id.fetch_add(1, Ordering::Relaxed);
        term.term_id = Some(id);
        self.terms
            .lock()
            .map_err(|_| Error::Implementation("term store lock poisoned".into()))?
            .insert(id, term.clone());
        Ok(term)
    }

    fn trace_term(&self, term: &Term) -> TraceTerm {
        let mut traced = TraceTerm::from(term);
        let mut ancestors = HashSet::new();
        if let Some(term_id) = term.term_id {
            ancestors.insert(term_id);
        }
        self.collect_trace_fragments(term, &mut Vec::new(), &mut ancestors, &mut traced.fragments);
        traced
    }

    fn collect_trace_fragments(
        &self,
        term: &Term,
        path: &mut Vec<usize>,
        ancestors: &mut HashSet<TermId>,
        fragments: &mut Vec<TraceFragment>,
    ) {
        let Some(payload) = self.trace_payload_ids(term) else {
            return;
        };
        for (index, origin_id) in payload.into_iter().enumerate() {
            path.push(index);
            fragments.push(TraceFragment {
                path: path.clone(),
                origin_id,
            });
            if ancestors.insert(origin_id) {
                if let Ok(payload_term) = self.term_for_id(origin_id) {
                    self.collect_trace_fragments(&payload_term, path, ancestors, fragments);
                }
                ancestors.remove(&origin_id);
            }
            path.pop();
        }
    }

    fn trace_payload_ids(&self, term: &Term) -> Option<Vec<TermId>> {
        let TermImplementation::Native(native_id) = term.implementation else {
            return None;
        };
        let state = self.native_terms.lock().ok()?.get(&native_id)?.clone();
        let payload = match state {
            NativeTermState::Arithmetic { captured, .. }
            | NativeTermState::Comparison { captured }
            | NativeTermState::StringComparison { captured }
            | NativeTermState::EmitTwice { captured }
            | NativeTermState::EmitOnce { captured } => vec![captured],
            NativeTermState::StringEquality { expected } => vec![expected],
            NativeTermState::Each { transform }
            | NativeTermState::Sieve {
                predicate: transform,
            }
            | NativeTermState::SieveDrop {
                predicate: transform,
            }
            | NativeTermState::FoldInit { transform }
            | NativeTermState::ReduceInit { transform } => vec![transform],
            NativeTermState::EachApply { transform, input }
            | NativeTermState::SieveApply {
                predicate: transform,
                input,
            }
            | NativeTermState::SieveKeep {
                predicate: transform,
                input,
            } => vec![transform, input],
            NativeTermState::EachEmit { transform, value } => vec![transform, value],
            NativeTermState::FoldAccum {
                transform,
                accumulator,
            }
            | NativeTermState::FoldNext {
                transform,
                accumulator,
            }
            | NativeTermState::ReduceAccum {
                transform,
                accumulator,
            }
            | NativeTermState::ReduceNext {
                transform,
                accumulator,
            } => vec![transform, accumulator],
            NativeTermState::FoldApply {
                transform,
                accumulator,
                input,
            }
            | NativeTermState::ReduceApply {
                transform,
                accumulator,
                input,
            } => vec![transform, accumulator, input],
            NativeTermState::FoldClose { accumulator }
            | NativeTermState::ReduceClose { accumulator } => vec![accumulator],
            NativeTermState::Template { payload, .. } => payload,
            NativeTermState::Ordering | NativeTermState::FilterChoice(_) => Vec::new(),
            #[cfg(test)]
            NativeTermState::SelfLoop => Vec::new(),
        };
        Some(payload)
    }

    fn term_for_id(&self, id: u64) -> Result<Term> {
        self.terms
            .lock()
            .map_err(|_| Error::Implementation("term store lock poisoned".into()))?
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::Implementation(format!("unknown TermId: {id}")))
    }

    fn template_payload_term(&self, native_id: NativeTermId, index: usize) -> Result<TermId> {
        let state = self
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .get(&native_id)
            .cloned()
            .ok_or_else(|| Error::Implementation(format!("unknown Template state: {native_id}")))?;
        let NativeTermState::Template { payload, .. } = state else {
            return Err(Error::Implementation(
                "payload reference requires a Template state".into(),
            ));
        };
        payload.get(index).copied().ok_or_else(|| {
            Error::Implementation(format!("payload reference out of range: %{index}"))
        })
    }

    fn template_payload_ids(&self, term: &Term) -> Result<Option<Vec<TermId>>> {
        let TermImplementation::Native(native_id) = term.implementation else {
            return Ok(None);
        };
        let state = self
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .get(&native_id)
            .cloned()
            .ok_or_else(|| Error::Implementation(format!("unknown Native Term: {native_id}")))?;
        match state {
            NativeTermState::Template { payload, .. } => Ok(Some(payload)),
            _ => Ok(None),
        }
    }

    fn is_template_state(&self, term: &Term) -> bool {
        let TermImplementation::Native(native_id) = term.implementation else {
            return false;
        };
        self.native_terms
            .lock()
            .ok()
            .and_then(|states| states.get(&native_id).cloned())
            .is_some_and(|state| matches!(state, NativeTermState::Template { .. }))
    }

    fn run_relation(
        &self,
        implementation: &RelationImplementation,
        context: RelationContext,
        trace: &mut TraceContext<'_>,
    ) -> Result<RelationOutcome> {
        match implementation {
            RelationImplementation::Capture => Err(Error::Implementation(
                "Capture Relation must be executed structurally".into(),
            )),
            RelationImplementation::Sayo(path) => self.run_sayo_relation(path, context, trace),
            RelationImplementation::Native(id) => {
                self.run_native_relation(*id, context.input_term_id, context)
            }
        }
    }

    fn run_sayo_relation(
        &self,
        implementation: &VirtualPath,
        context: RelationContext,
        trace: &mut TraceContext<'_>,
    ) -> Result<RelationOutcome> {
        let mut stack = Vec::new();
        let result = {
            let mut relation_trace = TraceContext {
                recorder: &mut *trace.recorder,
                group: context.trace_parent_group,
            };
            let path = SourcePath::world(implementation.clone());
            let source = path.read();
            match source {
                Ok(source) => {
                    stack.push(path.clone());
                    let base = path
                        .parent()
                        .unwrap_or_else(|| SourcePath::world(self.world.clone()));
                    let result = self.evaluate_source(
                        &source,
                        &base,
                        &mut stack,
                        Some(context),
                        &mut relation_trace,
                    );
                    stack.pop();
                    result
                }
                Err(error) => Err(error),
            }
        };
        Ok(match result {
            Ok(normal_form) => {
                RelationOutcome::Success(RelationResult::Terms(normal_form.into_terms()))
            }
            Err(Error::BootstrapLimit(message)) => {
                RelationOutcome::Trap(Error::BootstrapLimit(message))
            }
            Err(error) => RelationOutcome::Trap(Error::Implementation(format!(
                "Sayo implementation {} failed: {error}",
                implementation.display()
            ))),
        })
    }
}

impl<'a> NativeContext<'a> {
    fn term(&self, id: TermId) -> Result<Term> {
        self.runtime.term_for_id(id)
    }

    fn register_native_arithmetic(
        &self,
        captured: TermId,
        operation: ArithmeticOperation,
        space: &str,
        display: String,
    ) -> Result<TermId> {
        let native_id = self
            .runtime
            .next_native_term_id
            .fetch_add(1, Ordering::Relaxed);
        self.runtime
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(
                native_id,
                NativeTermState::Arithmetic {
                    captured,
                    operation,
                },
            );
        let term = self.runtime.store_term(Term {
            term_id: None,
            evaluation_depth: self.depth,
            source: String::new(),
            display,
            space: space.into(),
            implementation: TermImplementation::Native(native_id),
        })?;
        term.term_id
            .ok_or_else(|| Error::Implementation("native term was not assigned an ID".into()))
    }

    fn register_int(&self, value: i64) -> Result<TermId> {
        let term = self.runtime.store_term(Term {
            term_id: None,
            evaluation_depth: self.depth,
            source: value.to_string(),
            display: value.to_string(),
            space: "Int".into(),
            implementation: TermImplementation::None,
        })?;
        term.term_id
            .ok_or_else(|| Error::Implementation("native Int was not assigned an ID".into()))
    }

    fn register_native_comparison(&self, captured: TermId) -> Result<TermId> {
        let native_id = self
            .runtime
            .next_native_term_id
            .fetch_add(1, Ordering::Relaxed);
        self.runtime
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(native_id, NativeTermState::Comparison { captured });
        let display = format!("CompareInt{{{}}}", self.term(captured)?.display);
        let term = self.runtime.store_term(Term {
            term_id: None,
            evaluation_depth: self.depth,
            source: String::new(),
            display,
            space: "CompareInt".into(),
            implementation: TermImplementation::Native(native_id),
        })?;
        term.term_id.ok_or_else(|| {
            Error::Implementation("native comparison was not assigned a TermId".into())
        })
    }

    fn register_native_string_comparison(&self, captured: TermId) -> Result<TermId> {
        let native_id = self
            .runtime
            .next_native_term_id
            .fetch_add(1, Ordering::Relaxed);
        self.runtime
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(native_id, NativeTermState::StringComparison { captured });
        let display = format!("CompareString{{{}}}", self.term(captured)?.display);
        let term = self.runtime.store_term(Term {
            term_id: None,
            evaluation_depth: self.depth,
            source: String::new(),
            display,
            space: "CompareString".into(),
            implementation: TermImplementation::Native(native_id),
        })?;
        term.term_id.ok_or_else(|| {
            Error::Implementation("native String comparison was not assigned a TermId".into())
        })
    }

    fn register_native_string_equality(&self, expected: TermId) -> Result<TermId> {
        let native_id = self
            .runtime
            .next_native_term_id
            .fetch_add(1, Ordering::Relaxed);
        self.runtime
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(native_id, NativeTermState::StringEquality { expected });
        let display = format!("EqualStringExpected{{{}}}", self.term(expected)?.display);
        let term = self.runtime.store_term(Term {
            term_id: None,
            evaluation_depth: self.depth,
            source: String::new(),
            display,
            space: "EqualStringExpected".into(),
            implementation: TermImplementation::Native(native_id),
        })?;
        term.term_id.ok_or_else(|| {
            Error::Implementation("native String equality was not assigned a TermId".into())
        })
    }

    fn register_miss(&self) -> Result<TermId> {
        let term = self.runtime.store_term(Term {
            term_id: None,
            evaluation_depth: self.depth,
            source: "Miss".into(),
            display: "Miss".into(),
            space: "__Miss".into(),
            implementation: TermImplementation::None,
        })?;
        term.term_id
            .ok_or_else(|| Error::Implementation("Miss was not assigned a TermId".into()))
    }

    fn register_native_ordering(&self, kind: OrderingKind) -> Result<TermId> {
        let native_id = self
            .runtime
            .next_native_term_id
            .fetch_add(1, Ordering::Relaxed);
        self.runtime
            .native_terms
            .lock()
            .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
            .insert(native_id, NativeTermState::Ordering);
        let display = match kind {
            OrderingKind::Less => "Less",
            OrderingKind::Equal => "Equal",
            OrderingKind::Greater => "Greater",
        };
        let term = self.runtime.store_term(Term {
            term_id: None,
            evaluation_depth: self.depth,
            source: display.into(),
            display: display.into(),
            space: "Ordering".into(),
            implementation: TermImplementation::Native(native_id),
        })?;
        term.term_id
            .ok_or_else(|| Error::Implementation("Ordering was not assigned a TermId".into()))
    }
}

fn parse_int(term: &Term) -> Result<i64> {
    term.source.trim().parse::<i64>().map_err(|_| {
        Error::Implementation(format!(
            "Native Int operation received invalid source: {:?}",
            term.source
        ))
    })
}

fn parse_string(term: &Term) -> Result<String> {
    match parser::parse(&term.source) {
        Ok(Expression::StringLiteral(value)) => Ok(value),
        _ => Err(Error::Implementation(format!(
            "Native String operation received invalid source: {:?}",
            term.source
        ))),
    }
}

fn native_capture_int(
    context: &mut NativeContext<'_>,
    input: TermId,
    operation: ArithmeticOperation,
    space: &str,
) -> Result<NativeOutcome> {
    let term = context.term(input)?;
    parse_int(&term)?;
    let name = match operation {
        ArithmeticOperation::Add => "Plus",
        ArithmeticOperation::Sub => "Minus",
        ArithmeticOperation::Mul => "Times",
        ArithmeticOperation::Div => "Quotient",
    };
    Ok(NativeOutcome::Success(context.register_native_arithmetic(
        input,
        operation,
        space,
        format!("{name}{{{}}}", term.display),
    )?))
}

fn native_add_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_capture_int(context, input, ArithmeticOperation::Add, "Plus")
}

fn native_sub_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_capture_int(context, input, ArithmeticOperation::Sub, "Minus")
}

fn native_mul_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_capture_int(context, input, ArithmeticOperation::Mul, "Times")
}

fn native_div_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_capture_int(context, input, ArithmeticOperation::Div, "Quotient")
}

fn native_compare_capture_int(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    parse_int(&context.term(input)?)?;
    Ok(NativeOutcome::Success(
        context.register_native_comparison(input)?,
    ))
}

fn native_compare_apply_int(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    let right = parse_int(&context.term(input)?)?;
    let native_id = context
        .native_term_id
        .ok_or_else(|| Error::Implementation("comparison Native Term ID is missing".into()))?;
    let captured = context
        .runtime
        .native_terms
        .lock()
        .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
        .get(&native_id)
        .and_then(|state| match state {
            NativeTermState::Comparison { captured } => Some(*captured),
            _ => None,
        })
        .ok_or_else(|| Error::Implementation("missing captured comparison value".into()))?;
    let left = parse_int(&context.term(captured)?)?;
    let kind = match left.cmp(&right) {
        std::cmp::Ordering::Less => OrderingKind::Less,
        std::cmp::Ordering::Equal => OrderingKind::Equal,
        std::cmp::Ordering::Greater => OrderingKind::Greater,
    };
    Ok(NativeOutcome::Success(
        context.register_native_ordering(kind)?,
    ))
}

fn native_compare_capture_string(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    parse_string(&context.term(input)?)?;
    Ok(NativeOutcome::Success(
        context.register_native_string_comparison(input)?,
    ))
}

fn native_compare_apply_string(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    let right = parse_string(&context.term(input)?)?;
    let native_id = context.native_term_id.ok_or_else(|| {
        Error::Implementation("String comparison Native Term ID is missing".into())
    })?;
    let captured = context
        .runtime
        .native_terms
        .lock()
        .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
        .get(&native_id)
        .and_then(|state| match state {
            NativeTermState::StringComparison { captured } => Some(*captured),
            _ => None,
        })
        .ok_or_else(|| Error::Implementation("missing captured String comparison value".into()))?;
    let left = parse_string(&context.term(captured)?)?;
    let kind = match left.cmp(&right) {
        std::cmp::Ordering::Less => OrderingKind::Less,
        std::cmp::Ordering::Equal => OrderingKind::Equal,
        std::cmp::Ordering::Greater => OrderingKind::Greater,
    };
    Ok(NativeOutcome::Success(
        context.register_native_ordering(kind)?,
    ))
}

fn native_equal_string_capture(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    if !context.space_checker {
        return Ok(NativeOutcome::Trap(Error::Implementation(
            "equal-string is restricted to Space Checker evaluation".into(),
        )));
    }
    parse_string(&context.term(input)?)?;
    Ok(NativeOutcome::Success(
        context.register_native_string_equality(input)?,
    ))
}

fn native_equal_string_apply(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    if !context.space_checker {
        return Ok(NativeOutcome::Trap(Error::Implementation(
            "equal-string is restricted to Space Checker evaluation".into(),
        )));
    }
    let actual = parse_string(&context.term(input)?)?;
    let native_id = context
        .native_term_id
        .ok_or_else(|| Error::Implementation("String equality Native Term ID is missing".into()))?;
    let expected = context
        .runtime
        .native_terms
        .lock()
        .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
        .get(&native_id)
        .and_then(|state| match state {
            NativeTermState::StringEquality { expected } => Some(*expected),
            _ => None,
        })
        .ok_or_else(|| Error::Implementation("missing expected String value".into()))?;
    if parse_string(&context.term(expected)?)? == actual {
        Ok(NativeOutcome::Success(input))
    } else {
        Ok(NativeOutcome::Success(context.register_miss()?))
    }
}

fn native_duplicate_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    let emitter = context
        .runtime
        .register_native_emitter(input, true, context.depth)?;
    let term_id = emitter
        .term_id
        .ok_or_else(|| Error::Implementation("native emitter was not assigned an ID".into()))?;
    Ok(NativeOutcome::Success(term_id))
}

fn native_for_transform(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    let each = context.runtime.register_native_each(input, context.depth)?;
    let term_id = each
        .term_id
        .ok_or_else(|| Error::Implementation("Each state was not assigned a TermId".into()))?;
    Ok(NativeOutcome::Success(term_id))
}

fn native_each_int(_context: &mut NativeContext<'_>, _input: TermId) -> Result<NativeOutcome> {
    // A source Atom named `each` has no captured transform. Captured Each
    // states are dispatched by `run_native_term` above.
    Ok(NativeOutcome::NotApplicable)
}

fn native_filter_transform(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    let term = context
        .runtime
        .register_native_sieve(input, context.depth)?;
    Ok(NativeOutcome::Success(term.term_id.expect("Sieve id")))
}

fn native_sieve_input(_context: &mut NativeContext<'_>, _input: TermId) -> Result<NativeOutcome> {
    Ok(NativeOutcome::NotApplicable)
}

fn native_fold_transform(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    let id = context
        .runtime
        .next_native_term_id
        .fetch_add(1, Ordering::Relaxed);
    context
        .runtime
        .native_terms
        .lock()
        .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
        .insert(id, NativeTermState::FoldInit { transform: input });
    let term = context.runtime.store_term(Term {
        term_id: None,
        evaluation_depth: context.depth,
        source: String::new(),
        display: format!("FoldInit{{{}}}", context.term(input)?.display),
        space: "FoldInit".into(),
        implementation: TermImplementation::Native(id),
    })?;
    Ok(NativeOutcome::Success(term.term_id.expect("FoldInit id")))
}

fn native_fold_input(_context: &mut NativeContext<'_>, _input: TermId) -> Result<NativeOutcome> {
    Ok(NativeOutcome::NotApplicable)
}

fn native_reduce_transform(
    context: &mut NativeContext<'_>,
    input: TermId,
) -> Result<NativeOutcome> {
    let id = context
        .runtime
        .next_native_term_id
        .fetch_add(1, Ordering::Relaxed);
    context
        .runtime
        .native_terms
        .lock()
        .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?
        .insert(id, NativeTermState::ReduceInit { transform: input });
    let term = context.runtime.store_term(Term {
        term_id: None,
        evaluation_depth: context.depth,
        source: String::new(),
        display: format!("ReduceInit{{{}}}", context.term(input)?.display),
        space: "ReduceInit".into(),
        implementation: TermImplementation::Native(id),
    })?;
    Ok(NativeOutcome::Success(term.term_id.expect("ReduceInit id")))
}

fn native_reduce_input(_context: &mut NativeContext<'_>, _input: TermId) -> Result<NativeOutcome> {
    Ok(NativeOutcome::NotApplicable)
}

fn native_odd_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    let value = parse_int(&context.term(input)?)?;
    let kind = if value % 2 == 0 {
        FilterChoice::Drop
    } else {
        FilterChoice::Keep
    };
    let term = context
        .runtime
        .register_native_filter_choice(kind, context.depth)?;
    Ok(NativeOutcome::Success(
        term.term_id.expect("Filter choice id"),
    ))
}

fn native_plus_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_apply_int(context, input, ArithmeticOperation::Add)
}

fn native_minus_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_apply_int(context, input, ArithmeticOperation::Sub)
}

fn native_times_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_apply_int(context, input, ArithmeticOperation::Mul)
}

fn native_quotient_int(context: &mut NativeContext<'_>, input: TermId) -> Result<NativeOutcome> {
    native_apply_int(context, input, ArithmeticOperation::Div)
}

fn native_apply_int(
    context: &mut NativeContext<'_>,
    input: TermId,
    operation: ArithmeticOperation,
) -> Result<NativeOutcome> {
    let right = parse_int(&context.term(input)?)?;
    let native_id = context
        .native_term_id
        .ok_or_else(|| Error::Implementation("arithmetic Native Term ID is missing".into()))?;
    let native_terms = context
        .runtime
        .native_terms
        .lock()
        .map_err(|_| Error::Implementation("native term store lock poisoned".into()))?;
    let captured = native_terms.get(&native_id).and_then(|state| match state {
        NativeTermState::Arithmetic { captured, .. } => Some(*captured),
        NativeTermState::EmitTwice { .. }
        | NativeTermState::EmitOnce { .. }
        | NativeTermState::Each { .. }
        | NativeTermState::EachApply { .. }
        | NativeTermState::EachEmit { .. } => None,
        _ => None,
    });
    let captured =
        captured.ok_or_else(|| Error::Implementation("missing captured Plus value".into()))?;
    drop(native_terms);
    let left = parse_int(&context.term(captured)?)?;
    let value = match operation {
        ArithmeticOperation::Add => left.checked_add(right),
        ArithmeticOperation::Sub => left.checked_sub(right),
        ArithmeticOperation::Mul => left.checked_mul(right),
        ArithmeticOperation::Div => left.checked_div(right),
    }
    .ok_or_else(|| match operation {
        ArithmeticOperation::Add => Error::Implementation("Native Int addition overflow".into()),
        ArithmeticOperation::Sub => Error::Implementation("Native Int subtraction overflow".into()),
        ArithmeticOperation::Mul => {
            Error::Implementation("Native Int multiplication overflow".into())
        }
        ArithmeticOperation::Div if right == 0 => {
            Error::Implementation("Native Int division by zero".into())
        }
        ArithmeticOperation::Div => Error::Implementation("Native Int division overflow".into()),
    })?;
    Ok(NativeOutcome::Success(context.register_int(value)?))
}

/// Returns the top-level left-associative application chain without flattening
/// an application that appeared as an explicitly grouped right operand.
fn application_terms(expression: &Expression) -> Vec<Expression> {
    let mut right_terms = Vec::new();
    let mut current = expression;
    while let Expression::Apply(left, right) = current {
        right_terms.push((**right).clone());
        current = left.as_ref();
    }
    right_terms.reverse();
    let mut terms = Vec::with_capacity(right_terms.len() + 1);
    terms.push(current.clone());
    terms.extend(right_terms);
    terms
}

fn render_string_literal(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push('"');
    for character in value.chars() {
        match character {
            '"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            character => rendered.push(character),
        }
    }
    rendered.push('"');
    rendered
}

fn is_native_primitive_space(space: &str) -> bool {
    NATIVE_PRIMITIVE_SPACES.contains(&space)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::trace::TraceEvent;

    use super::*;

    struct TestWorld(PathBuf);

    impl TestWorld {
        fn arithmetic() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let world = std::env::temp_dir()
                .join(format!("sayo-runtime-test-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&world).unwrap();
            Self(world)
        }

        fn declare_atom(&self, space: &str, atom: &str) {
            fs::create_dir_all(self.0.join(space)).unwrap();
            fs::write(
                self.0.join(format!("{space}/space.sayo")),
                format!("equal-to \"{atom}\" %0\n"),
            )
            .unwrap();
        }

        fn declare_state(&self, template: &str, arity: usize) {
            assert!(arity > 0);
            fs::create_dir_all(self.0.join(template)).unwrap();
            let root = format!("ArityRoot{template}");
            fs::create_dir_all(self.0.join(&root)).unwrap();
            fs::write(self.0.join(format!("{root}/space.sayo")), "Miss\n").unwrap();
            let mut left = root;
            for index in 1..=arity {
                let result = if index == arity {
                    template.to_owned()
                } else {
                    format!("Arity{template}{index}")
                };
                fs::create_dir_all(self.0.join(format!("{left}/Int/{result}"))).unwrap();
                left = result;
            }
        }

        fn deferred_reduction() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let world = std::env::temp_dir().join(format!(
                "sayo-runtime-reducer-test-{}-{nonce}",
                std::process::id()
            ));
            for (space, atom) in [
                ("A", "a"),
                ("B", "b"),
                ("C", "c"),
                ("D", "d"),
                ("Result", "result"),
            ] {
                fs::create_dir_all(world.join(space)).unwrap();
                fs::write(
                    world.join(format!("{space}/space.sayo")),
                    format!("equal-to \"{atom}\" %0\n"),
                )
                .unwrap();
            }
            fs::create_dir_all(world.join("B/C/D")).unwrap();
            fs::write(world.join("B/C/D/main.sayo"), "d\n").unwrap();
            fs::create_dir_all(world.join("A/D/Result")).unwrap();
            fs::write(world.join("A/D/Result/main.sayo"), "result\n").unwrap();
            Self(world)
        }
    }

    impl Drop for TestWorld {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn runtime_with_standard(
        user: &TestWorld,
        edit_standard: impl FnOnce(&mut MemoryWorld),
    ) -> Runtime {
        let user = MemoryWorld::snapshot(&FsWorld::open(&user.0).unwrap()).unwrap();
        let mut standard = embedded_standard_world().unwrap();
        edit_standard(&mut standard);
        Runtime::from_world_snapshots(user, standard, RuntimeLimits::default()).unwrap()
    }

    fn single(normal_form: NormalForm) -> Term {
        normal_form.single().expect("expected one-Term normal form")
    }

    #[test]
    fn evaluates_native_add_and_plus_relations() {
        let world = TestWorld::arithmetic();
        let term = single(Runtime::new(&world.0).unwrap().evaluate("add 5 3").unwrap());

        assert_eq!(term.source, "8");
        assert_eq!(term.space, "Int");
        assert!(matches!(term.implementation, TermImplementation::None));
    }

    #[test]
    fn user_structural_capture_takes_precedence_over_native_fallback() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Add/Int/Plus")).unwrap();

        let term = single(Runtime::new(&world.0).unwrap().evaluate("add 5").unwrap());
        assert_eq!(term.display, "Plus{5}");
        assert!(matches!(term.implementation, TermImplementation::Native(_)));
        assert!(Runtime::new(&world.0).is_ok());
    }

    #[test]
    fn compares_native_ints_into_ordering_terms() {
        let world = TestWorld::arithmetic();
        let runtime = Runtime::new(&world.0).unwrap();

        for (source, expected) in [
            ("compare 5 9", "Less"),
            ("compare 5 5", "Equal"),
            ("compare 9 5", "Greater"),
            ("compare -9 -5", "Less"),
        ] {
            let term = single(runtime.evaluate(source).unwrap());
            assert_eq!(term.display, expected);
            assert_eq!(term.source, expected);
            assert_eq!(term.space, "Ordering");
            assert!(matches!(term.implementation, TermImplementation::Native(_)));
        }
    }

    #[test]
    fn compares_native_strings_into_ordering_terms() {
        let world = TestWorld::arithmetic();
        let runtime = Runtime::new(&world.0).unwrap();

        for (source, expected) in [
            (r#"compare "a" "b""#, "Less"),
            (r#"compare "同じ" "同じ""#, "Equal"),
            (r#"compare "z" "a""#, "Greater"),
        ] {
            let term = single(runtime.evaluate(source).unwrap());
            assert_eq!(term.display, expected);
            assert_eq!(term.space, "Ordering");
        }
    }

    #[test]
    fn compare_int_captures_its_left_operand_before_comparing() {
        let world = TestWorld::arithmetic();
        let runtime = Runtime::new(&world.0).unwrap();

        let term = single(runtime.evaluate("compare 7").unwrap());
        assert_eq!(term.display, "CompareInt{7}");
        assert_eq!(term.space, "CompareInt");
    }

    #[test]
    fn returns_a_multi_term_normal_form_when_no_transition_remains() {
        let world = TestWorld::arithmetic();
        let normal_form = Runtime::new(&world.0).unwrap().evaluate("5 3").unwrap();

        assert_eq!(normal_form.terms().len(), 2);
        assert_eq!(normal_form.terms()[0].source, "5");
        assert_eq!(normal_form.terms()[1].source, "3");
    }

    #[test]
    fn splices_a_sayo_relation_normal_form_into_its_caller() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Copy", "copy");
        fs::create_dir_all(world.0.join("Copy/Int/CopyApply")).unwrap();
        fs::create_dir_all(world.0.join("CopyApply/.tick/Copy")).unwrap();
        fs::write(
            world.0.join("CopyApply/.tick/Copy/main.sayo"),
            "%0 %0 copy\n",
        )
        .unwrap();

        let normal_form = Runtime::new(&world.0).unwrap().evaluate("copy 5").unwrap();

        assert_eq!(normal_form.terms().len(), 3);
        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.source.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "5", "copy"]
        );
        assert_eq!(normal_form.terms()[2].space, "Copy");
    }

    #[test]
    fn internal_emitter_writes_values_left_of_its_continuation() {
        let world = TestWorld::arithmetic();

        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("duplicate 5");
        let normal_form = traced.result.unwrap();
        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "5", "duplicate"]
        );
        assert_eq!(normal_form.terms()[2].space, "Duplicate");
        assert!(
            traced
                .trace
                .events()
                .iter()
                .any(|event| matches!(event, TraceEvent::StateTransition { .. }))
        );
    }

    #[test]
    fn duplicate_continuation_consumes_later_right_terms() {
        let world = TestWorld::arithmetic();

        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("duplicate 5 6");
        let normal_form = traced.result.unwrap();
        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "5", "6", "6", "duplicate"]
        );
        let trace = traced.trace.render_json();
        assert!(trace.contains("EmitTwice{5}"));
        assert!(trace.contains("EmitPair{5, 5}"));
        assert!(trace.contains("EmitOnce{5}"));
        assert!(trace.contains("\"executor\": \"capture\""));
    }

    #[test]
    fn native_primitive_atom_is_available_without_a_user_space_checker() {
        let world = TestWorld::arithmetic();

        let normal_form = Runtime::new(&world.0)
            .unwrap()
            .evaluate("duplicate 5")
            .unwrap();

        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "5", "duplicate"]
        );
        assert_eq!(normal_form.terms()[2].space, "Duplicate");
    }

    #[test]
    fn traces_end_transition_that_closes_fold() {
        let world = TestWorld::arithmetic();

        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("fold add 0 1 2 3");

        assert_eq!(traced.result.unwrap().single().unwrap().display, "6");
        assert!(traced.trace.events().iter().any(|event| {
            matches!(
                event,
                TraceEvent::EndTransition { inserted, .. }
                    if inserted
                        .iter()
                        .any(|insertion| insertion.term.source == "FoldClose{add, 6}")
            )
        }));
        assert!(
            traced
                .trace
                .render_json()
                .contains("\"event\": \"end_transition\"")
        );
    }

    #[test]
    fn standard_world_fold_returns_its_initial_accumulator_without_more_inputs() {
        let world = TestWorld::arithmetic();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("fold add 7")
                .unwrap(),
        );
        assert_eq!(term.display, "7");
    }

    #[test]
    fn standard_world_fold_traps_when_the_initial_accumulator_is_missing() {
        let world = TestWorld::arithmetic();

        let error = Runtime::new(&world.0).unwrap().evaluate("fold add");
        assert!(matches!(error, Err(Error::Implementation(message)) if message.contains("Trap")));
    }

    #[test]
    fn reduce_uses_its_first_input_as_the_accumulator() {
        let world = TestWorld::arithmetic();

        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("reduce add 1 2 3");

        assert_eq!(traced.result.unwrap().single().unwrap().display, "6");
        let sri = traced.trace.render_json();
        assert!(sri.contains("ReduceAccum{add, 1}"));
        assert!(sri.contains("\"event\": \"end_transition\""));
        assert!(sri.contains("ReduceClose{add, 6}"));
    }

    #[test]
    fn reduce_rejects_an_empty_input_sequence_at_end() {
        let world = TestWorld::arithmetic();

        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("reduce add")
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Implementation(message) if message.contains("Trap")
        ));
    }

    #[test]
    fn standard_world_reduce_returns_its_only_input() {
        let world = TestWorld::arithmetic();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("reduce add 7")
                .unwrap(),
        );
        assert_eq!(term.display, "7");
    }

    #[test]
    fn permits_a_visible_self_transition_until_the_configured_bootstrap_limit() {
        let world = TestWorld::arithmetic();
        let limits = RuntimeLimits {
            reduction_steps: 3,
            ..RuntimeLimits::default()
        };
        let runtime = Runtime::new_with_limits(&world.0, limits).unwrap();
        let self_loop = runtime.register_native_self_loop().unwrap();
        let mut cells = vec![ReductionCell::term(self_loop, None)];
        let mut stack = Vec::new();
        let mut recorder = None;
        let mut trace = TraceContext {
            recorder: &mut recorder,
            group: None,
        };
        let base = SourcePath::host(std::env::current_dir().unwrap());

        let error = runtime
            .reduce_application_group(&mut cells, &base, &mut stack, None, 0, &mut trace)
            .unwrap_err();
        assert!(matches!(
            error,
            Error::BootstrapLimit(message) if message.contains("limit=3")
        ));
    }

    #[test]
    fn zero_step_limit_accepts_an_already_normal_flat_sequence() {
        let world = TestWorld::arithmetic();
        let limits = RuntimeLimits {
            reduction_steps: 0,
            ..RuntimeLimits::default()
        };

        let normal_form = Runtime::new_with_limits(&world.0, limits)
            .unwrap()
            .evaluate("5 3")
            .unwrap();

        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "3"]
        );
    }

    #[test]
    fn relation_depth_does_not_limit_flat_term_count() {
        let world = TestWorld::arithmetic();
        let source = (0..1_024)
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        let normal_form = Runtime::new(&world.0).unwrap().evaluate(&source).unwrap();

        assert_eq!(normal_form.terms().len(), 1_024);
        assert_eq!(normal_form.terms().first().unwrap().display, "0");
        assert_eq!(normal_form.terms().last().unwrap().display, "1023");
    }

    #[test]
    fn reduction_step_limit_is_inclusive_and_traced() {
        let world = TestWorld::arithmetic();
        let one_step = Runtime::new_with_limits(
            &world.0,
            RuntimeLimits {
                reduction_steps: 1,
                ..RuntimeLimits::default()
            },
        )
        .unwrap();

        assert_eq!(
            one_step
                .evaluate("add 5")
                .unwrap()
                .single()
                .unwrap()
                .display,
            "Plus{5}"
        );
        let exceeded = one_step.evaluate_with_trace("add 5 3");
        assert!(matches!(
            exceeded.result,
            Err(Error::BootstrapLimit(message)) if message.contains("limit=1")
        ));
        assert_eq!(
            exceeded
                .trace
                .events()
                .iter()
                .filter(|event| matches!(event, TraceEvent::Trapped { .. }))
                .count(),
            1
        );

        let two_steps = Runtime::new_with_limits(
            &world.0,
            RuntimeLimits {
                reduction_steps: 2,
                ..RuntimeLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            two_steps
                .evaluate("add 5 3")
                .unwrap()
                .single()
                .unwrap()
                .display,
            "8"
        );
    }

    #[test]
    fn for_uses_an_each_state_to_emit_each_mapped_value_leftward() {
        let world = TestWorld::arithmetic();
        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("for (add 9) 1 2 3");
        let normal_form = traced.result.unwrap();
        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["10", "11", "12", "Each{Plus{9}}"]
        );
        let sri = traced.trace.render_json();
        assert!(sri.contains("EachApply{Plus{9}, 1}"));
        assert!(sri.contains("EachEmit{Plus{9}, 10}"));
        assert!(sri.contains("\"executor\": \"capture\""));
        let captured_nine = traced.trace.events().iter().find_map(|event| match event {
            TraceEvent::Attempted { right, .. } if right.source == "9" => right.term_id,
            _ => None,
        });
        assert!(captured_nine.is_some());
        assert!(traced.trace.events().iter().any(|event| {
            let inserted = match event {
                TraceEvent::Rewritten { inserted, .. }
                | TraceEvent::StateTransition { inserted, .. }
                | TraceEvent::EndTransition { inserted, .. } => inserted.as_slice(),
                _ => &[],
            };
            inserted.iter().any(|entry| {
                entry.term.fragments.iter().any(|fragment| {
                    fragment.path == [0, 0] && Some(fragment.origin_id) == captured_nine
                })
            })
        }));
    }

    #[test]
    fn for_applies_a_sayo_relation_transform() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Identity", "identity");
        fs::create_dir_all(world.0.join("Identity/Int/IdentityApply")).unwrap();
        fs::create_dir_all(world.0.join("IdentityApply/.tick/Int")).unwrap();
        fs::write(world.0.join("IdentityApply/.tick/Int/main.sayo"), "%0\n").unwrap();

        let normal_form = Runtime::new(&world.0)
            .unwrap()
            .evaluate("for identity 5 6")
            .unwrap();
        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "6", "Each{identity}"]
        );
    }

    #[test]
    fn for_rejects_a_transform_that_returns_multiple_terms() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Expand", "expand");
        fs::create_dir_all(world.0.join("Expand/Int/ExpandApply")).unwrap();
        fs::create_dir_all(world.0.join("ExpandApply/.tick/Int")).unwrap();
        fs::write(world.0.join("ExpandApply/.tick/Int/main.sayo"), "%0 %0\n").unwrap();

        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("for expand 5")
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Implementation(message) if message.contains("exactly one Term")
        ));
    }

    #[test]
    fn filter_emits_kept_values_and_drops_rejected_values() {
        let world = TestWorld::arithmetic();
        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("filter odd 1 2 3 4");
        let normal_form = traced.result.unwrap();
        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "3", "Sieve{odd}"]
        );
        let sri = traced.trace.render_json();
        assert!(sri.contains("SieveChoose{odd, 1}"));
        assert!(sri.contains("SieveEmit{odd, 1}"));
        assert!(sri.contains("SieveDiscard{odd}"));
        assert!(sri.contains("\"space\": \"Keep\""));
        assert!(sri.contains("\"space\": \"Drop\""));
    }

    #[test]
    fn filter_accepts_a_sayo_keep_space_result() {
        let world = TestWorld::arithmetic();
        world.declare_atom("AlwaysKeep", "always_keep");
        fs::create_dir_all(world.0.join("AlwaysKeep/Int/Keep")).unwrap();
        fs::write(world.0.join("AlwaysKeep/Int/Keep/main.sayo"), "keep\n").unwrap();

        let normal_form = Runtime::new(&world.0)
            .unwrap()
            .evaluate("filter always_keep 1 2")
            .unwrap();
        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "Sieve{always_keep}"]
        );
    }

    #[test]
    fn filter_rejects_a_predicate_result_outside_keep_and_drop_spaces() {
        let world = TestWorld::arithmetic();

        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("filter add 1")
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Implementation(message) if message.contains("semantic Trap")
        ));
    }

    #[test]
    fn evaluates_parenthesized_group_before_splicing_it() {
        let world = TestWorld::arithmetic();
        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("add (add 2 3) 4")
                .unwrap(),
        );
        assert_eq!(term.source, "9");
    }

    #[test]
    fn materializes_a_payload_backed_state_term() {
        let world = TestWorld::arithmetic();
        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Each { (add 9) }")
                .unwrap(),
        );
        assert_eq!(term.space, "Each");
        assert_eq!(term.display, "Each{Plus{9}}");
        assert!(matches!(term.implementation, TermImplementation::Native(_)));
    }

    #[test]
    fn captures_a_right_term_with_an_empty_relation_directory() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Seed", "seed");
        fs::create_dir_all(world.0.join("Seed/Int/Box")).unwrap();
        fs::create_dir_all(world.0.join("Box")).unwrap();

        let term = single(Runtime::new(&world.0).unwrap().evaluate("seed 7").unwrap());
        assert_eq!(term.display, "Box{7}");
    }

    #[test]
    fn template_tick_uses_its_result_as_next_payload_and_emits_unused_payload() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 2);
        fs::create_dir_all(world.0.join("Box/.tick/Next")).unwrap();
        fs::create_dir_all(world.0.join("Next")).unwrap();
        fs::write(world.0.join("Box/.tick/Next/main.sayo"), "%0\n").unwrap();

        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7, 8 }")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["8", "Next{7}"]);
    }

    #[test]
    fn empty_tick_relation_transitions_to_a_zero_payload_state() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box/.tick/Empty")).unwrap();
        fs::create_dir_all(world.0.join("Empty")).unwrap();

        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7 }")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();

        assert_eq!(terms, vec!["7", "Empty{}"]);
    }

    #[test]
    fn empty_tick_relation_can_construct_a_source_backed_zero_payload_state() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Before/.tick/After")).unwrap();
        fs::write(world.0.join("Before/space.sayo"), "Miss\n").unwrap();
        fs::create_dir_all(world.0.join("After")).unwrap();
        fs::write(world.0.join("After/space.sayo"), "Miss\n").unwrap();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Before {}")
                .unwrap(),
        );

        assert_eq!(term.display, "After{}");
    }

    #[test]
    fn tick_counts_payload_references_used_by_a_referenced_sayo_file() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box/.tick/Next")).unwrap();
        fs::create_dir_all(world.0.join("Next")).unwrap();
        fs::write(world.0.join("Box/.tick/Next/main.sayo"), "payload.sayo\n").unwrap();
        fs::write(world.0.join("Box/.tick/Next/payload.sayo"), "%0\n").unwrap();

        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7 }")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["Next{7}"]);
    }

    #[test]
    fn ambiguous_template_control_relations_trap() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box/.tick/First")).unwrap();
        fs::create_dir_all(world.0.join("Box/.tick/Second")).unwrap();
        fs::write(world.0.join("Box/.tick/First/main.sayo"), "%0\n").unwrap();
        fs::write(world.0.join("Box/.tick/Second/main.sayo"), "%0\n").unwrap();

        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7 }")
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Relation(message)
                if message.contains("ambiguous Template control Relation for `Box` .tick")
                    && message.contains("First, Second")
        ));
    }

    #[test]
    fn empty_transition_to_vanish_emits_payload_then_removes_the_state_shell() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box/.tick/Vanish")).unwrap();

        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7 }")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["7"]);
    }

    #[test]
    fn transition_can_move_payload_into_vanish_to_destroy_it() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 2);
        fs::create_dir_all(world.0.join("Box/.tick/Vanish")).unwrap();
        fs::write(world.0.join("Box/.tick/Vanish/main.sayo"), "%0\n").unwrap();

        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7, 8 }")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["8"]);
    }

    #[test]
    fn vanish_accepts_different_payload_arities_and_destroys_every_term() {
        let world = TestWorld::arithmetic();
        world.declare_state("DiscardOne", 1);
        world.declare_state("DiscardTwo", 2);
        fs::create_dir_all(world.0.join("DiscardOne/.tick/Vanish")).unwrap();
        fs::create_dir_all(world.0.join("DiscardTwo/.tick/Vanish")).unwrap();
        fs::write(world.0.join("DiscardOne/.tick/Vanish/main.sayo"), "%0\n").unwrap();
        fs::write(world.0.join("DiscardTwo/.tick/Vanish/main.sayo"), "%0 %1\n").unwrap();
        let runtime = Runtime::new(&world.0).unwrap();

        assert!(
            runtime
                .evaluate("DiscardOne { 1 }")
                .unwrap()
                .terms()
                .is_empty()
        );
        assert!(
            runtime
                .evaluate("DiscardTwo { 1, 2 }")
                .unwrap()
                .terms()
                .is_empty()
        );
    }

    #[test]
    fn users_can_construct_a_declared_standard_world_state() {
        let world = TestWorld::arithmetic();

        let normal_form = Runtime::new(&world.0)
            .unwrap()
            .evaluate("SieveApply { odd, 1 }")
            .unwrap();

        assert_eq!(
            normal_form
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "Sieve{odd}"]
        );
    }

    #[test]
    fn state_construction_rejects_unknown_and_reserved_templates() {
        let world = TestWorld::arithmetic();
        let runtime = Runtime::new(&world.0).unwrap();

        assert!(matches!(
            runtime.evaluate("UnknownState { 1 }"),
            Err(Error::Membership(message)) if message.contains("unknown Template `UnknownState`")
        ));
        assert!(matches!(
            runtime.evaluate("Vanish { 1 }"),
            Err(Error::Implementation(message)) if message.contains("reserved")
        ));
        assert!(matches!(
            runtime.evaluate("Trap { 1 }"),
            Err(Error::Implementation(message)) if message.contains("reserved")
        ));
        assert!(matches!(
            runtime.evaluate("SieveApply { odd }"),
            Err(Error::Implementation(message))
                if message.contains("Template `SieveApply` has static payload arity 2, got 1")
        ));
    }

    #[test]
    fn applies_the_standard_world_for_template() {
        let world = TestWorld::arithmetic();
        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("for (add 9) 1 2")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["10", "11", "Each{Plus{9}}"]);
    }

    #[test]
    fn rejects_payload_state_construction_for_a_zero_arity_root() {
        let world = TestWorld::arithmetic();
        let error = Runtime::new(&world.0).unwrap().evaluate("For { (add 9) }");
        assert!(matches!(
            error,
            Err(Error::Implementation(message))
                if message.contains("Template `For` has static payload arity 0, got 1")
        ));
    }

    #[test]
    fn constructs_a_zero_payload_state_for_a_zero_arity_template() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Empty")).unwrap();
        fs::write(world.0.join("Empty/space.sayo"), "Miss\n").unwrap();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Empty {}")
                .unwrap(),
        );

        assert_eq!(term.display, "Empty{}");
    }

    #[test]
    fn keeps_each_template_payload_arity_fixed_across_state_instances() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        let runtime = Runtime::new(&world.0).unwrap();

        let first = single(runtime.evaluate("Box { 7 }").unwrap());
        assert_eq!(first.display, "Box{7}");

        let error = runtime.evaluate("Box { 7, 8 }").unwrap_err();
        assert!(matches!(
            error,
            Error::Implementation(message)
                if message.contains("Template `Box` has static payload arity 1, got 2")
        ));
    }

    #[test]
    fn runs_a_restricted_sayo_template_schema() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box")).unwrap();
        fs::write(world.0.join("Box/template.sayo"), "%0\n").unwrap();
        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Box { 7 }")
                .unwrap(),
        );
        assert_eq!(term.display, "Box{7}");
    }

    #[test]
    fn runs_a_schema_for_a_zero_payload_state() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Empty")).unwrap();
        fs::write(world.0.join("Empty/space.sayo"), "Miss\n").unwrap();
        fs::write(world.0.join("Empty/template.sayo"), "Pass {}\n").unwrap();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Empty {}")
                .unwrap(),
        );

        assert_eq!(term.display, "Empty{}");
    }

    #[test]
    fn treats_space_names_as_literals_in_template_schemas() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box")).unwrap();
        fs::write(world.0.join("Box/template.sayo"), "CheckSpace{%0, Int}\n").unwrap();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Box { 7 }")
                .unwrap(),
        );
        assert_eq!(term.display, "Box{7}");
    }

    #[test]
    fn traps_when_a_schema_space_check_does_not_match() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box")).unwrap();
        fs::write(world.0.join("Box/template.sayo"), "CheckSpace{%0, Int}\n").unwrap();

        let error = Runtime::new(&world.0).unwrap().evaluate("Box { add }");
        assert!(matches!(
            error,
            Err(Error::Implementation(message))
                if message.contains("CheckSpace failed") && message.contains("Space Int")
        ));
    }

    #[test]
    fn uses_any_relation_as_a_structural_capture_fallback() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Box", "box");
        fs::create_dir_all(world.0.join("Box/.any/BoxInput")).unwrap();
        let term = single(Runtime::new(&world.0).unwrap().evaluate("box 1").unwrap());
        assert_eq!(term.display, "BoxInput{1}");
    }

    #[test]
    fn resolves_a_template_payload_reference_in_a_relation() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box")).unwrap();
        fs::create_dir_all(world.0.join("Box/Int/Int")).unwrap();
        fs::write(world.0.join("Box/Int/Int/main.sayo"), "%0\n").unwrap();
        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Box { 7 } 1")
                .unwrap(),
        );
        assert_eq!(term.display, "7");
    }

    #[test]
    fn accepts_a_dynamic_result_relation() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box")).unwrap();
        fs::create_dir_all(world.0.join("Box/Int/.any")).unwrap();
        fs::write(world.0.join("Box/Int/.any/main.sayo"), "%0\n").unwrap();
        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("Box { 7 } 1")
                .unwrap(),
        );
        assert_eq!(term.display, "7");
    }

    #[test]
    fn rejects_fixed_and_dynamic_results_for_the_same_relation_input() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box/Int/Next")).unwrap();
        fs::create_dir_all(world.0.join("Box/Int/.any")).unwrap();
        fs::write(world.0.join("Box/Int/.any/main.sayo"), "%0\n").unwrap();

        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7 } 1")
            .unwrap_err();

        assert!(matches!(
            error,
            Error::World(message)
                if message.contains("fixed and dynamic Result Relations cannot coexist")
        ));
    }

    #[test]
    fn applies_a_template_end_relation_at_group_end() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box")).unwrap();
        fs::create_dir_all(world.0.join("Box/.end/Int")).unwrap();
        fs::write(world.0.join("Box/.end/Int/main.sayo"), "%0\n").unwrap();
        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("(Box { 7 })")
                .unwrap(),
        );
        assert_eq!(term.display, "7");
    }

    #[test]
    fn empty_end_relation_transitions_to_a_zero_payload_state() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box/.end/Empty")).unwrap();
        fs::create_dir_all(world.0.join("Empty")).unwrap();

        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("(Box { 7 })")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();

        assert_eq!(terms, vec!["7", "Empty{}"]);
    }

    #[test]
    fn applies_a_template_tick_relation_before_input() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        fs::create_dir_all(world.0.join("Box")).unwrap();
        fs::create_dir_all(world.0.join("Box/.tick/Int")).unwrap();
        fs::write(world.0.join("Box/.tick/Int/main.sayo"), "%0\n").unwrap();
        let terms = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Box { 7 } 1")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["7", "1"]);
    }

    #[test]
    fn traces_native_rewrites_and_the_final_yield() {
        let world = TestWorld::arithmetic();
        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("add 5 3");

        assert_eq!(traced.result.as_ref().unwrap().terms()[0].source, "8");
        assert!(traced.trace.events().iter().any(|event| {
            matches!(
                event,
                TraceEvent::Rewritten { relation, .. }
                    if relation.executor == "native"
                        && relation.left_space == "Add"
                        && relation.right_space == "Int"
                        && relation.result_space == "Plus"
            )
        }));
        assert!(traced.trace.events().iter().any(|event| {
            matches!(
                event,
                TraceEvent::Yielded { term, .. }
                    if term.source == "8" && term.space == "Int"
            )
        }));
    }

    #[test]
    fn retries_the_left_pair_after_the_right_pair_reduces() {
        let world = TestWorld::deferred_reduction();
        let term = single(Runtime::new(&world.0).unwrap().evaluate("a b c").unwrap());

        assert_eq!(term.space, "Result");
        assert_eq!(term.source, "result");
    }

    #[test]
    fn scans_past_a_no_step_before_retrying_a_reduced_right_term() {
        let world = TestWorld::deferred_reduction();
        let traced = Runtime::new(&world.0).unwrap().evaluate_with_trace("a b c");

        assert_eq!(traced.result.as_ref().unwrap().terms()[0].space, "Result");
        assert!(
            traced
                .trace
                .events()
                .iter()
                .any(|event| matches!(event, TraceEvent::NotApplicable { .. }))
        );
        assert!(
            traced
                .trace
                .events()
                .iter()
                .any(|event| matches!(event, TraceEvent::Rewritten { .. }))
        );
    }

    #[test]
    fn evaluates_native_sub_mul_and_div_relations() {
        let world = TestWorld::arithmetic();
        let runtime = Runtime::new(&world.0).unwrap();
        for (source, expected) in [("sub 5 3", "2"), ("mul 5 3", "15"), ("div 15 3", "5")] {
            let term = single(runtime.evaluate(source).unwrap());
            assert_eq!(term.source, expected, "{source}");
            assert_eq!(term.space, "Int");
        }
    }

    #[test]
    fn native_division_rejects_zero() {
        let world = TestWorld::arithmetic();
        let error = Runtime::new(&world.0).unwrap().evaluate("div 5 0");
        assert!(
            matches!(error, Err(Error::Implementation(message)) if message.contains("division by zero"))
        );
    }

    #[test]
    fn traces_traps_without_losing_the_evaluation_error() {
        let world = TestWorld::arithmetic();
        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("div 5 0");

        assert!(matches!(
            traced.result,
            Err(Error::Implementation(message)) if message.contains("division by zero")
        ));
        assert!(traced.trace.events().iter().any(|event| {
            matches!(
                event,
                TraceEvent::Trapped { category, message, .. }
                    if category == "implementation error" && message.contains("division by zero")
            )
        }));
    }

    #[test]
    fn evaluating_the_reserved_trap_atom_stops_semantically() {
        let world = TestWorld::arithmetic();
        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("Trap")
            .unwrap_err();
        assert!(matches!(error, Error::Implementation(message) if message == "semantic Trap"));
    }

    #[test]
    fn prefers_a_concrete_world_relation_over_native_wildcard() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Add/Int/Int")).unwrap();
        fs::write(world.0.join("Add/Int/Int/main.sayo"), "77\n").unwrap();

        let normal = Runtime::new(&world.0).unwrap().evaluate("add 5").unwrap();
        assert_eq!(
            normal
                .terms()
                .iter()
                .map(|term| term.display.as_str())
                .collect::<Vec<_>>(),
            vec!["77"]
        );
    }

    #[test]
    fn rejects_a_world_that_defines_the_reserved_trap_template() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let world = std::env::temp_dir().join(format!(
            "sayo-runtime-test-reserved-trap-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(world.join("Int")).unwrap();
        fs::create_dir_all(world.join("Trap")).unwrap();

        let error = match Runtime::new(&world) {
            Ok(_) => panic!("reserved Trap world was accepted"),
            Err(error) => error,
        };
        let _ = fs::remove_dir_all(&world);
        assert!(
            matches!(error, Error::World(message) if message.contains("Trap") && message.contains("reserved"))
        );
    }

    #[test]
    fn rejects_duplicate_native_registration() {
        let mut registry = NativeRegistry::bootstrap();
        let error = registry
            .register("Add", "Int", "Plus", native_add_int)
            .unwrap_err();
        assert!(
            matches!(error, Error::Implementation(message) if message.contains("duplicate Native Relation"))
        );
    }

    #[test]
    fn native_add_supports_negative_ints() {
        let world = TestWorld::arithmetic();
        let term = Runtime::new(&world.0)
            .unwrap()
            .evaluate("add -5 3")
            .unwrap();
        let term = single(term);

        assert_eq!(term.source, "-2");
        assert_eq!(term.space, "Int");
    }

    #[test]
    fn native_add_normalizes_leading_zeroes() {
        let world = TestWorld::arithmetic();
        let term = Runtime::new(&world.0)
            .unwrap()
            .evaluate("add 0005 3")
            .unwrap();
        let term = single(term);

        assert_eq!(term.source, "8");
        assert_eq!(term.space, "Int");
    }

    #[test]
    fn native_add_rejects_i64_overflow() {
        let world = TestWorld::arithmetic();
        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("add 9223372036854775807 1")
            .unwrap_err();

        assert!(matches!(error, Error::Implementation(message) if message.contains("overflow")));
    }

    #[test]
    fn evaluates_sayo_file_references_relative_to_the_source_file() {
        let world = TestWorld::arithmetic();
        fs::write(world.0.join("five.sayo"), "5\n").unwrap();
        fs::write(world.0.join("add-five.sayo"), "add five.sayo 3\n").unwrap();

        let term = Runtime::new(&world.0)
            .unwrap()
            .evaluate_file(world.0.join("add-five.sayo"))
            .unwrap();
        let term = single(term);
        assert_eq!(term.source, "8");
        assert_eq!(term.space, "Int");
    }

    #[test]
    fn runtime_uses_the_world_snapshot_taken_at_construction() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Const", "const");
        fs::create_dir_all(world.0.join("Const/Int/Int")).unwrap();
        fs::write(world.0.join("Const/Int/Int/main.sayo"), "5\n").unwrap();
        let runtime = Runtime::new(&world.0).unwrap();

        fs::write(world.0.join("Const/Int/Int/main.sayo"), "99\n").unwrap();

        let term = single(runtime.evaluate("const 1").unwrap());
        assert_eq!(term.display, "5");
    }

    #[test]
    fn evaluates_a_constant_sayo_relation() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Const", "const");
        fs::create_dir_all(world.0.join("Const/Int/Int")).unwrap();
        fs::write(world.0.join("Const/Int/Int/main.sayo"), "5\n").unwrap();

        let term = single(Runtime::new(&world.0).unwrap().evaluate("const 3").unwrap());
        assert_eq!(term.source.trim(), "5");
        assert_eq!(term.space, "Int");
        assert!(matches!(term.implementation, TermImplementation::None));
    }

    #[test]
    fn sayo_relation_can_construct_a_zero_payload_state() {
        let world = TestWorld::arithmetic();
        world.declare_atom("MakeEmpty", "make_empty");
        fs::create_dir_all(world.0.join("MakeEmpty/Int/Empty")).unwrap();
        fs::write(world.0.join("MakeEmpty/Int/Empty/main.sayo"), "Empty {}\n").unwrap();
        fs::create_dir_all(world.0.join("Empty")).unwrap();
        fs::write(world.0.join("Empty/space.sayo"), "Miss\n").unwrap();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate("make_empty 3")
                .unwrap(),
        );

        assert_eq!(term.display, "Empty{}");
        assert_eq!(term.space, "Empty");
        assert!(matches!(term.implementation, TermImplementation::Native(_)));
    }

    #[test]
    fn resolves_spaces_and_relations_from_the_memory_standard_world() {
        let world = TestWorld::arithmetic();
        let runtime = runtime_with_standard(&world, |standard| {
            standard
                .insert_file(
                    "StandardValue/space.sayo",
                    "equal-to \"standard-value\" %0\n",
                )
                .unwrap();
            standard
                .insert_file("StandardValue/Int/Int/main.sayo", "42\n")
                .unwrap();
        });

        let term = single(runtime.evaluate("standard-value 1").unwrap());
        assert_eq!(term.display, "42");
        assert_eq!(term.space, "Int");
        assert!(!world.0.join("StandardValue").exists());
    }

    #[test]
    fn user_world_relations_override_standard_world_relations() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Add/Int/Int")).unwrap();
        fs::write(world.0.join("Add/Int/Int/main.sayo"), "77\n").unwrap();
        let runtime = runtime_with_standard(&world, |standard| {
            standard
                .insert_file("Add/Int/Int/main.sayo", "99\n")
                .unwrap();
        });

        let term = single(runtime.evaluate("add 5").unwrap());
        assert_eq!(term.display, "77");
    }

    #[test]
    fn user_component_without_a_relation_hides_the_standard_component_relation() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Shadowed", "shadowed");
        let runtime = runtime_with_standard(&world, |standard| {
            standard
                .insert_file("Shadowed/Int/Int/main.sayo", "99\n")
                .unwrap();
        });

        let terms = runtime
            .evaluate("shadowed 1")
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["shadowed", "1"]);
    }

    #[test]
    fn user_component_hides_standard_schema_and_control_relations() {
        let world = TestWorld::arithmetic();
        world.declare_state("Box", 1);
        let runtime = runtime_with_standard(&world, |standard| {
            standard.insert_file("Box/template.sayo", "Trap\n").unwrap();
            standard
                .insert_file("Box/.tick/Next/main.sayo", "%0\n")
                .unwrap();
            standard.create_dir("Next").unwrap();
        });

        let term = single(runtime.evaluate("Box { 7 }").unwrap());
        assert_eq!(term.display, "Box{7}");
    }

    #[test]
    fn shadowed_standard_entry_uses_native_fallback_without_rejoining_internal_templates() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("For")).unwrap();
        fs::write(world.0.join("For/space.sayo"), "equal-to \"for\" %0\n").unwrap();

        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("for (add 9) 1 2");
        let terms = traced
            .result
            .unwrap()
            .terms()
            .iter()
            .map(|term| term.display.clone())
            .collect::<Vec<_>>();
        assert_eq!(terms, vec!["10", "11", "Each{Plus{9}}"]);
        assert!(traced.trace.events().iter().any(|event| matches!(event,
            TraceEvent::Rewritten { relation, .. }
                if relation.executor == "native" && relation.left_space == "For"
        )));
        assert!(!traced.trace.events().iter().any(|event| matches!(event,
            TraceEvent::Rewritten { relation, .. }
                if relation.executor == "capture" && matches!(relation.left_space.as_str(), "For" | "Each")
        )));
    }

    #[test]
    fn native_state_machine_optimizations_match_standard_sayo_observations() {
        let sayo_world = TestWorld::arithmetic();
        let native_world = TestWorld::arithmetic();
        for entry in ["Duplicate", "For", "Filter", "Fold", "Reduce"] {
            fs::create_dir_all(native_world.0.join(entry)).unwrap();
        }
        let sayo_runtime = Runtime::new(&sayo_world.0).unwrap();
        let native_runtime = Runtime::new(&native_world.0).unwrap();

        for source in [
            "duplicate 5 6",
            "for (add 9) 1 2 3",
            "filter odd 1 2 3 4",
            "fold add 0 1 2 3",
            "reduce add 1 2 3",
        ] {
            let observe = |runtime: &Runtime| {
                runtime
                    .evaluate(source)
                    .unwrap()
                    .terms()
                    .iter()
                    .map(|term| (term.display.clone(), term.space.clone()))
                    .collect::<Vec<_>>()
            };

            assert_eq!(observe(&sayo_runtime), observe(&native_runtime), "{source}");
        }
    }

    #[test]
    fn standard_world_relations_override_native_fallbacks() {
        let world = TestWorld::arithmetic();
        let runtime = runtime_with_standard(&world, |standard| {
            standard
                .insert_file("Add/Int/Int/main.sayo", "99\n")
                .unwrap();
        });

        let term = single(runtime.evaluate("add 5").unwrap());
        assert_eq!(term.display, "99");
    }

    #[test]
    fn evaluates_input_as_an_ordinary_atom() {
        let world = TestWorld::arithmetic();
        world.declare_atom("input", "input");

        let term = single(Runtime::new(&world.0).unwrap().evaluate("input").unwrap());
        assert_eq!(term.source, "input");
        assert_eq!(term.space, "input");
    }

    #[test]
    fn evaluates_a_string_literal_as_an_immutable_string_term() {
        let world = TestWorld::arithmetic();
        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate(r#""Sayo 日本語 \"quoted\" \\ \n""#)
                .unwrap(),
        );

        assert_eq!(term.space, "String");
        assert_eq!(term.source, r#""Sayo 日本語 \"quoted\" \\ \n""#);
        assert_eq!(term.display, term.source);
        assert!(matches!(term.implementation, TermImplementation::None));
    }

    #[test]
    fn sayo_space_checker_matches_and_misses_with_equal_to() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Hello")).unwrap();
        fs::write(world.0.join("Hello/space.sayo"), "equal-to \"hello\" %0\n").unwrap();
        let runtime = Runtime::new(&world.0).unwrap();

        let hello = single(runtime.evaluate("hello").unwrap());
        assert_eq!(hello.space, "Hello");
        let add = single(runtime.evaluate("add").unwrap());
        assert_eq!(add.space, "Add");
    }

    #[test]
    fn miss_is_checker_only_and_falls_through_to_native_classification() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Never")).unwrap();
        fs::write(world.0.join("Never/space.sayo"), "Miss\n").unwrap();
        let runtime = Runtime::new(&world.0).unwrap();

        assert_eq!(single(runtime.evaluate("add").unwrap()).space, "Add");
        let error = runtime.evaluate("Miss").unwrap_err();
        assert!(matches!(error, Error::Implementation(message) if message.contains("only valid")));
    }

    #[test]
    fn checker_trap_and_invalid_arity_are_implementation_errors() {
        for (source, expected) in [("Trap\n", "semantic Trap"), ("1 2\n", "exactly one Term")] {
            let world = TestWorld::arithmetic();
            fs::create_dir_all(world.0.join("Broken")).unwrap();
            fs::write(world.0.join("Broken/space.sayo"), source).unwrap();
            let error = Runtime::new(&world.0).unwrap().evaluate("add").unwrap_err();
            assert!(
                matches!(error, Error::Implementation(message) if message.contains("Sayo Space Checker") && message.contains(expected))
            );
        }
    }

    #[test]
    fn checker_cannot_use_user_world_atoms() {
        let world = TestWorld::arithmetic();
        fs::create_dir_all(world.0.join("Restricted")).unwrap();
        fs::write(world.0.join("Restricted/space.sayo"), "echo\n").unwrap();

        let error = Runtime::new(&world.0).unwrap().evaluate("add").unwrap_err();
        assert!(
            matches!(error, Error::Implementation(message) if message.contains("Sayo Space Checker") && message.contains("does not belong"))
        );
    }

    #[test]
    fn structural_relation_can_capture_a_string_term() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Hold", "hold");
        fs::create_dir_all(world.0.join("Hold/String/Holding")).unwrap();
        fs::create_dir_all(world.0.join("Holding")).unwrap();

        let term = single(
            Runtime::new(&world.0)
                .unwrap()
                .evaluate(r#"hold "two words""#)
                .unwrap(),
        );
        assert_eq!(term.display, r#"Holding{"two words"}"#);
    }

    #[test]
    fn traces_a_sayo_relation_as_a_child_group() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Increment", "increment");
        fs::create_dir_all(world.0.join("Increment/Int/IncrementApply")).unwrap();
        fs::create_dir_all(world.0.join("IncrementApply/.tick/Int")).unwrap();
        fs::write(
            world.0.join("IncrementApply/.tick/Int/main.sayo"),
            "(add %0 1)\n",
        )
        .unwrap();

        let traced = Runtime::new(&world.0)
            .unwrap()
            .evaluate_with_trace("increment 5");

        assert_eq!(traced.result.as_ref().unwrap().terms()[0].source, "6");
        let groups = traced
            .trace
            .events()
            .iter()
            .filter_map(|event| match event {
                TraceEvent::GroupStarted { group, parent, .. } => Some((*group, *parent)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(groups.len() >= 2);
        assert_eq!(groups[0].1, None);
        assert!(groups.iter().skip(1).any(|(_, parent)| parent.is_some()));
    }

    #[test]
    fn reports_membership_when_input_has_no_space() {
        let world = TestWorld::arithmetic();
        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate("input")
            .unwrap_err();
        assert!(matches!(error, Error::Membership(message) if message.contains("input")));
    }

    #[test]
    fn stops_when_relation_depth_exceeds_the_limit() {
        let world = TestWorld::arithmetic();
        world.declare_atom("Loop", "loop");
        fs::create_dir_all(world.0.join("Loop/Int/LoopApply")).unwrap();
        fs::create_dir_all(world.0.join("LoopApply/.tick/Loop")).unwrap();
        fs::write(
            world.0.join("LoopApply/.tick/Loop/main.sayo"),
            "(loop %0)\n",
        )
        .unwrap();

        let source = std::iter::once("loop")
            .chain(std::iter::repeat_n(
                "1",
                RuntimeLimits::default().relation_depth + 1,
            ))
            .collect::<Vec<_>>()
            .join(" ");
        let error = Runtime::new(&world.0)
            .unwrap()
            .evaluate(&source)
            .unwrap_err();
        assert!(
            matches!(error, Error::BootstrapLimit(message) if message.contains("depth exceeded"))
        );
    }
}
