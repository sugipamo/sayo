use crate::world::VirtualPath;
pub type TermId = u64;
pub type NativeTermId = u64;
pub type StateId = u64;
pub type TemplateId = u64;

/// Runtime-owned state for a payload-backed Template term.
///
/// The payload stores Term IDs rather than source strings so that a state can
/// refer to the same immutable terms without copying them.  The state itself
/// is immutable; a transition creates a new StateId.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateInstance {
    pub state_id: StateId,
    pub template: TemplateId,
    pub payload: Vec<TermId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermImplementation {
    None,
    Sayo(VirtualPath),
    Native(NativeTermId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    Atom(String),
    /// A decoded, immutable UTF-8 String value.
    StringLiteral(String),
    /// A state term with a Template name and an ordered Sayo payload.
    State {
        template: String,
        payload: Vec<Expression>,
    },
    /// A positional reference into the current Template payload.
    PayloadRef(usize),
    Apply(Box<Expression>, Box<Expression>),
    /// An explicit evaluation frame.  Unlike ordinary application, a group is
    /// reduced to its own normal form before that normal form is spliced into
    /// the surrounding sequence.
    Group(Box<Expression>),
}

impl Expression {
    /// Render a canonical Sayo source form without evaluating the expression.
    /// SRI uses this to expose a Group's complete initial state.
    pub fn render_source(&self) -> String {
        match self {
            Self::Atom(source) => source.clone(),
            Self::StringLiteral(value) => serde_json::to_string(value)
                .expect("a Rust UTF-8 string always has a JSON representation"),
            Self::State { template, payload } => format!(
                "{template}{{{}}}",
                payload
                    .iter()
                    .map(Self::render_source)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::PayloadRef(index) => format!("%{index}"),
            Self::Apply(left, right) => {
                format!("{} {}", left.render_source(), right.render_source())
            }
            Self::Group(inner) => format!("({})", inner.render_source()),
        }
    }
}

/// The stable result of evaluation.
///
/// A Sayo program does not have to reduce to one Term.  A normal form is the
/// ordered sequence left after no adjacent Terms can make another transition.
#[derive(Debug, Clone)]
pub struct NormalForm {
    terms: Vec<Term>,
}

impl NormalForm {
    pub fn new(terms: Vec<Term>) -> Self {
        Self { terms }
    }

    pub fn one(term: Term) -> Self {
        Self { terms: vec![term] }
    }

    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    pub fn into_terms(self) -> Vec<Term> {
        self.terms
    }

    pub fn single(self) -> std::result::Result<Term, Self> {
        match self.terms.as_slice() {
            [term] => Ok(term.clone()),
            _ => Err(self),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Term {
    pub term_id: Option<TermId>,
    pub evaluation_depth: usize,
    pub source: String,
    /// User-facing rendering. Native states have no Sayo source but retain a
    /// stable structural display such as `Plus{5}`.
    pub display: String,
    pub space: String,
    pub implementation: TermImplementation,
}

impl Term {
    pub fn atom(source: String, space: String) -> Self {
        Self {
            term_id: None,
            evaluation_depth: 0,
            display: source.clone(),
            source,
            space,
            implementation: TermImplementation::None,
        }
    }
}
