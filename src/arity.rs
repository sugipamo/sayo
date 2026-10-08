use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::Expression;
use crate::parser;
use crate::runtime::NATIVE_PRIMITIVE_SPACES;
use crate::world::{FsWorld, VirtualPath, WorldHandle};

#[derive(Debug, Clone)]
pub(crate) struct Analysis {
    arities: HashMap<String, usize>,
    inactive_lower_components: HashSet<String>,
}

impl Analysis {
    pub(crate) fn into_arities(self) -> HashMap<String, usize> {
        self.arities
    }

    pub(crate) fn inactive_lower_components(&self) -> &HashSet<String> {
        &self.inactive_lower_components
    }
}

#[derive(Debug)]
struct Capture {
    left: String,
    result: String,
    path: VirtualPath,
}

#[derive(Debug)]
struct Control {
    source: String,
    result: Option<String>,
    result_arity: Option<usize>,
    path: VirtualPath,
}

#[derive(Debug)]
struct StateConstraint {
    source: String,
    target: String,
    arity: usize,
    path: VirtualPath,
}

#[derive(Debug)]
struct Reference {
    source: String,
    max_reference: usize,
    path: VirtualPath,
}

pub(crate) fn analyze(roots: &[&Path]) -> Result<Analysis> {
    let handles = roots
        .iter()
        .map(|root| FsWorld::open(root).map(WorldHandle::new))
        .collect::<Result<Vec<_>>>()?;
    let roots = handles.iter().map(WorldHandle::root).collect::<Vec<_>>();
    analyze_virtual(&roots)
}

pub(crate) fn analyze_virtual(roots: &[VirtualPath]) -> Result<Analysis> {
    let mut component_layers = HashMap::new();
    let mut roots_by_name = HashSet::new();
    let mut capture_backed = HashSet::new();
    let mut visible_names = HashSet::new();
    let mut visible_components = Vec::new();

    for (root_index, root) in roots.iter().enumerate() {
        for component in root.directories()? {
            let Some(name) = visible_name(&component).map(str::to_owned) else {
                continue;
            };
            if !visible_names.insert(name.clone()) {
                continue;
            }
            visible_components.push((root_index, component, name));
        }
    }

    for (layer, component, name) in &visible_components {
        component_layers.insert(name.clone(), *layer);
        if has_root_implementation(component)? {
            roots_by_name.insert(name.clone());
        }
        if !NATIVE_PRIMITIVE_SPACES.contains(&name.as_str())
            && (component.join("template.sayo")?.is_file()? || !has_root_implementation(component)?)
        {
            capture_backed.insert(name.clone());
        }
    }
    for name in NATIVE_PRIMITIVE_SPACES {
        roots_by_name.insert((*name).to_owned());
    }

    let mut exact = HashMap::new();
    for name in &roots_by_name {
        set_exact(&mut exact, name, 0, "materializable root Template")?;
    }
    let mut captures = Vec::new();
    let mut controls = Vec::new();
    let mut states = Vec::new();
    let mut references = Vec::new();
    let mut reach_edges: HashMap<String, Vec<String>> = HashMap::new();

    for (_, component, left) in &visible_components {
        if let Some(path) = sayo_path(component)? {
            inspect_sayo(&path, left, &mut states, &mut reach_edges, &mut references)?;
        }
        let schema = component.join("template.sayo")?;
        if schema.is_file()? {
            inspect_payload_references(&schema, left, &mut references)?;
        }
        let space_checker = component.join("space.sayo")?;
        if space_checker.is_file()? {
            inspect_space_checker(&space_checker, left, &mut states, &mut reach_edges)?;
        }
        for right in component.directories()? {
            let Some(right_name) = relation_slot_name(&right) else {
                continue;
            };
            for result in right.directories()? {
                let Some(result_name) = result_name(&result) else {
                    continue;
                };
                let implementation = sayo_path(&result)?;
                let has_implementation = implementation.is_some();
                if matches!(right_name, ".tick" | ".end") {
                    if matches!(result_name, "Trap" | ".any") && implementation.is_none() {
                        return Err(Error::World(format!(
                            "Template control Relation with Result {result_name} requires main.sayo: {}",
                            result.display()
                        )));
                    }
                    let (result_template, result_arity) = if result_name == "Vanish"
                        || result_name == "Trap"
                        || result_name == ".any"
                    {
                        (None, None)
                    } else if implementation.is_none() {
                        // An empty fixed control Result is an explicit
                        // transition to a zero-payload StateInstance.
                        (Some(result_name.to_owned()), Some(0))
                    } else if capture_backed.contains(result_name) {
                        let path = implementation.as_ref().expect("checked as present");
                        let expression = parse_file(path)?;
                        (
                            Some(result_name.to_owned()),
                            Some(application_len(&expression)),
                        )
                    } else {
                        (None, None)
                    };
                    if let Some(path) = &implementation {
                        inspect_sayo(path, left, &mut states, &mut reach_edges, &mut references)?;
                    }
                    if let Some(target) = &result_template {
                        reach_edges
                            .entry(left.clone())
                            .or_default()
                            .push(target.clone());
                    }
                    controls.push(Control {
                        source: left.clone(),
                        result: result_template,
                        result_arity,
                        path: result,
                    });
                } else if !has_implementation && result_name != ".any" {
                    if left == result_name {
                        return Err(Error::World(format!(
                            "structural Capture must transition to a different Template: {}",
                            result.display()
                        )));
                    }
                    captures.push(Capture {
                        left: left.clone(),
                        result: result_name.to_owned(),
                        path: result.clone(),
                    });
                    reach_edges
                        .entry(left.clone())
                        .or_default()
                        .push(result_name.to_owned());
                } else if let Some(path) = implementation {
                    inspect_sayo(&path, left, &mut states, &mut reach_edges, &mut references)?;
                }
            }
        }
    }

    let mut reachable = roots_by_name.clone();
    let mut queue = roots_by_name.iter().cloned().collect::<VecDeque<_>>();
    while let Some(source) = queue.pop_front() {
        if let Some(targets) = reach_edges.get(&source) {
            for target in targets {
                if reachable.insert(target.clone()) {
                    queue.push_back(target.clone());
                }
            }
        }
    }

    for state in &states {
        if reachable.contains(&state.source) {
            set_exact(&mut exact, &state.target, state.arity, state.path.display())?;
        }
    }

    for control in &controls {
        if reachable.contains(&control.source)
            && let (Some(result), Some(arity)) = (&control.result, control.result_arity)
        {
            set_exact(&mut exact, result, arity, control.path.display())?;
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for capture in &captures {
            if !reachable.contains(&capture.left) {
                continue;
            }
            match (
                exact.get(&capture.left).copied(),
                exact.get(&capture.result).copied(),
            ) {
                (Some(left), None) => {
                    set_exact(
                        &mut exact,
                        &capture.result,
                        left + 1,
                        capture.path.display(),
                    )?;
                    changed = true;
                }
                (None, Some(result)) if result > 0 => {
                    set_exact(
                        &mut exact,
                        &capture.left,
                        result - 1,
                        capture.path.display(),
                    )?;
                    changed = true;
                }
                (Some(left), Some(result)) if result != left + 1 => {
                    return Err(Error::World(format!(
                        "inconsistent payload arity in Capture {}: {} has {left}, {} has {result}",
                        capture.path.display(),
                        capture.left,
                        capture.result
                    )));
                }
                _ => {}
            }
        }
    }

    for reference in &references {
        if !reachable.contains(&reference.source) {
            continue;
        }
        let source_arity = exact.get(&reference.source).copied().ok_or_else(|| {
            Error::World(format!(
                "cannot derive payload arity for Template `{}` used by {}",
                reference.source,
                reference.path.display()
            ))
        })?;
        if reference.max_reference >= source_arity {
            return Err(Error::World(format!(
                "payload reference %{} exceeds static arity {source_arity} of `{}`: {}",
                reference.max_reference,
                reference.source,
                reference.path.display()
            )));
        }
    }

    let mut inactive_lower_components = HashSet::new();
    for (component, layer) in component_layers {
        if !capture_backed.contains(&component) {
            continue;
        }
        if !reachable.contains(&component) && !matches!(component.as_str(), "Trap" | "Vanish") {
            if layer == 0 {
                return Err(Error::World(format!(
                    "Template `{component}` is unreachable from every zero-payload root"
                )));
            }
            // Shadowing a lower-layer entry Component can make that layer's
            // private implementation states unreachable. They are not part
            // of the effective World assembled for this Runtime.
            inactive_lower_components.insert(component);
            continue;
        }
        if !exact.contains_key(&component) {
            return Err(Error::World(format!(
                "cannot derive a unique payload arity for Template `{component}`"
            )));
        }
    }

    Ok(Analysis {
        arities: exact,
        inactive_lower_components,
    })
}

fn inspect_sayo(
    path: &VirtualPath,
    source: &str,
    state_constraints: &mut Vec<StateConstraint>,
    reach_edges: &mut HashMap<String, Vec<String>>,
    references: &mut Vec<Reference>,
) -> Result<()> {
    let mut visited = Vec::new();
    inspect_sayo_recursive(
        path,
        source,
        state_constraints,
        reach_edges,
        references,
        &mut visited,
    )
}

fn inspect_space_checker(
    path: &VirtualPath,
    source: &str,
    state_constraints: &mut Vec<StateConstraint>,
    reach_edges: &mut HashMap<String, Vec<String>>,
) -> Result<()> {
    let expression = parse_file(path)?;
    if let Some(max_reference) = max_payload_reference(&expression)
        && max_reference > 0
    {
        return Err(Error::World(format!(
            "Space Checker may only reference %0, found %{max_reference}: {}",
            path.display()
        )));
    }
    let mut files = Vec::new();
    collect_sayo_files(&expression, &mut files);
    if !files.is_empty() {
        return Err(Error::World(format!(
            "Space Checker cannot access external Sayo files: {}",
            path.display()
        )));
    }
    let mut checker_states = Vec::new();
    collect_states(&expression, &mut checker_states);
    for (target, arity) in checker_states {
        state_constraints.push(StateConstraint {
            source: source.to_owned(),
            target: target.clone(),
            arity,
            path: path.clone(),
        });
        reach_edges
            .entry(source.to_owned())
            .or_default()
            .push(target);
    }
    Ok(())
}

fn inspect_sayo_recursive(
    path: &VirtualPath,
    source: &str,
    state_constraints: &mut Vec<StateConstraint>,
    reach_edges: &mut HashMap<String, Vec<String>>,
    references: &mut Vec<Reference>,
    visited: &mut Vec<VirtualPath>,
) -> Result<()> {
    if visited.iter().any(|visited| visited == path) {
        return Ok(());
    }
    visited.push(path.clone());
    let expression = parse_file(path)?;
    record_payload_references(path, source, &expression, references);
    let mut found_states = Vec::new();
    collect_states(&expression, &mut found_states);
    for (template, arity) in found_states {
        state_constraints.push(StateConstraint {
            source: source.to_owned(),
            target: template.clone(),
            arity,
            path: path.clone(),
        });
        reach_edges
            .entry(source.to_owned())
            .or_default()
            .push(template);
    }
    let mut files = Vec::new();
    collect_sayo_files(&expression, &mut files);
    let base = path.parent().unwrap_or_else(|| path.clone());
    for file in files {
        let referenced = base.resolve(file)?;
        if referenced.is_file()? {
            inspect_sayo_recursive(
                &referenced,
                source,
                state_constraints,
                reach_edges,
                references,
                visited,
            )?;
        }
    }
    Ok(())
}

fn inspect_payload_references(
    path: &VirtualPath,
    source: &str,
    references: &mut Vec<Reference>,
) -> Result<()> {
    let expression = parse_file(path)?;
    let mut files = Vec::new();
    collect_sayo_files(&expression, &mut files);
    if let Some(file) = files.first() {
        return Err(Error::World(format!(
            "Template schema cannot access external Sayo file `{}`: {}",
            file.display(),
            path.display()
        )));
    }
    record_payload_references(path, source, &expression, references);
    Ok(())
}

fn collect_sayo_files(expression: &Expression, files: &mut Vec<PathBuf>) {
    match expression {
        Expression::Atom(source) if source.ends_with(".sayo") => {
            files.push(PathBuf::from(source));
        }
        Expression::State { payload, .. } => {
            for item in payload {
                collect_sayo_files(item, files);
            }
        }
        Expression::Apply(left, right) => {
            collect_sayo_files(left, files);
            collect_sayo_files(right, files);
        }
        Expression::Group(inner) => collect_sayo_files(inner, files),
        Expression::External { .. }
        | Expression::Atom(_)
        | Expression::StringLiteral(_)
        | Expression::PayloadRef(_) => {}
    }
}

fn record_payload_references(
    path: &VirtualPath,
    source: &str,
    expression: &Expression,
    references: &mut Vec<Reference>,
) {
    if let Some(max_reference) = max_payload_reference(expression) {
        references.push(Reference {
            source: source.to_owned(),
            max_reference,
            path: path.clone(),
        });
    }
}

fn collect_states(expression: &Expression, states: &mut Vec<(String, usize)>) {
    match expression {
        Expression::State { template, payload } => {
            states.push((template.clone(), payload.len()));
            for item in payload {
                collect_states(item, states);
            }
        }
        Expression::Apply(left, right) => {
            collect_states(left, states);
            collect_states(right, states);
        }
        Expression::Group(inner) => collect_states(inner, states),
        Expression::External { .. }
        | Expression::Atom(_)
        | Expression::StringLiteral(_)
        | Expression::PayloadRef(_) => {}
    }
}

fn max_payload_reference(expression: &Expression) -> Option<usize> {
    match expression {
        Expression::PayloadRef(index) => Some(*index),
        Expression::State { payload, .. } => payload.iter().filter_map(max_payload_reference).max(),
        Expression::Apply(left, right) => {
            max_payload_reference(left).max(max_payload_reference(right))
        }
        Expression::Group(inner) => max_payload_reference(inner),
        Expression::External { .. } | Expression::Atom(_) | Expression::StringLiteral(_) => None,
    }
}

fn application_len(expression: &Expression) -> usize {
    let mut count = 1;
    let mut current = expression;
    while let Expression::Apply(left, _) = current {
        count += 1;
        current = left;
    }
    count
}

fn set_exact(
    exact: &mut HashMap<String, usize>,
    template: &str,
    arity: usize,
    source: &str,
) -> Result<()> {
    match exact.get(template) {
        Some(existing) if *existing != arity => Err(Error::World(format!(
            "Template `{template}` has conflicting static payload arities {existing} and {arity}: {source}"
        ))),
        Some(_) => Ok(()),
        None => {
            exact.insert(template.to_owned(), arity);
            Ok(())
        }
    }
}

fn parse_file(path: &VirtualPath) -> Result<Expression> {
    let source = path
        .read()?
        .ok_or_else(|| Error::World(format!("cannot analyze missing Sayo {}", path.display())))?;
    parser::parse(&source)
        .map_err(|error| Error::World(format!("cannot analyze Sayo {}: {error}", path.display())))
}

fn has_root_implementation(component: &VirtualPath) -> Result<bool> {
    Ok(component.join("main.sayo")?.is_file()? || component.join("space.sayo")?.is_file()?)
}

fn sayo_path(path: &VirtualPath) -> Result<Option<VirtualPath>> {
    let implementation = path.join("main.sayo")?;
    Ok(implementation.is_file()?.then_some(implementation))
}

fn visible_name(path: &VirtualPath) -> Option<&str> {
    path.file_name()
        .filter(|name| !name.starts_with('.') && !name.starts_with("__"))
}

fn relation_slot_name(path: &VirtualPath) -> Option<&str> {
    path.file_name()
        .filter(|name| matches!(*name, ".any" | ".tick" | ".end") || !name.starts_with('.'))
}

fn result_name(path: &VirtualPath) -> Option<&str> {
    path.file_name()
        .filter(|name| *name == ".any" || !name.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::embedded_standard_world;
    use crate::world::{MemoryWorld, WorldHandle};

    use super::*;

    struct TestWorld(PathBuf);

    impl TestWorld {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("sayo-arity-{label}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn root(&self, name: &str) {
            fs::create_dir_all(self.0.join(name)).unwrap();
            fs::write(self.0.join(format!("{name}/space.sayo")), "Miss\n").unwrap();
        }

        fn capture(&self, left: &str, result: &str) {
            fs::create_dir_all(self.0.join(format!("{left}/Value/{result}"))).unwrap();
            fs::create_dir_all(self.0.join(result)).unwrap();
        }
    }

    impl Drop for TestWorld {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn analyzes_the_embedded_standard_world_directly_from_memory() {
        let user = WorldHandle::new(MemoryWorld::new());
        let standard = WorldHandle::new(embedded_standard_world().unwrap());
        let analysis = analyze_virtual(&[user.root(), standard.root()]).unwrap();

        assert_eq!(analysis.arities.get("Each"), Some(&1));
        assert_eq!(analysis.arities.get("FoldAccum"), Some(&2));
        assert_eq!(analysis.arities.get("EmitPair"), Some(&2));
    }

    #[test]
    fn derives_capture_arities_from_a_zero_payload_root() {
        let world = TestWorld::new("derive");
        world.root("Root");
        world.capture("Root", "One");
        world.capture("One", "Two");

        let arities = analyze(&[world.0.as_path()]).unwrap().into_arities();
        assert_eq!(arities.get("Root"), Some(&0));
        assert_eq!(arities.get("One"), Some(&1));
        assert_eq!(arities.get("Two"), Some(&2));
    }

    #[test]
    fn rejects_two_paths_that_derive_different_arities() {
        let world = TestWorld::new("merge");
        world.root("A");
        world.root("B");
        world.capture("A", "X");
        world.capture("B", "Y");
        world.capture("Y", "X");

        let error = analyze(&[world.0.as_path()]).unwrap_err();
        assert!(matches!(
            error,
            Error::World(message) if message.contains("inconsistent payload arity")
                || message.contains("conflicting static payload arities")
        ));
    }

    #[test]
    fn empty_control_relation_requires_a_zero_payload_result() {
        let world = TestWorld::new("zero-control-conflict");
        world.root("Root");
        world.root("Before");
        world.capture("Root", "After");
        fs::create_dir_all(world.0.join("Before/.tick/After")).unwrap();

        let error = analyze(&[world.0.as_path()]).unwrap_err();

        assert!(matches!(
            error,
            Error::World(message)
                if message.contains("inconsistent payload arity")
                    && message.contains("After has 0")
        ));
    }

    #[test]
    fn empty_dynamic_and_trap_controls_are_invalid() {
        for result in [".any", "Trap"] {
            let world = TestWorld::new(&format!("empty-{result}"));
            world.root("Before");
            fs::create_dir_all(world.0.join(format!("Before/.tick/{result}"))).unwrap();

            let error = analyze(&[world.0.as_path()]).unwrap_err();

            assert!(matches!(
                error,
                Error::World(message)
                    if message.contains("requires main.sayo") && message.contains(result)
            ));
        }
    }

    #[test]
    fn rejects_self_capture_and_unreachable_cycles() {
        let self_world = TestWorld::new("self");
        self_world.root("Box");
        self_world.capture("Box", "Box");
        assert!(matches!(
            analyze(&[self_world.0.as_path()]),
            Err(Error::World(message)) if message.contains("different Template")
        ));

        let cycle = TestWorld::new("cycle");
        cycle.capture("A", "B");
        cycle.capture("B", "A");
        assert!(matches!(
            analyze(&[cycle.0.as_path()]),
            Err(Error::World(message)) if message.contains("unreachable")
                || message.contains("cannot derive")
        ));
    }

    #[test]
    fn tick_can_increase_payload_arity_by_self_transformation() {
        let world = TestWorld::new("tick-grow");
        world.root("Root");
        world.capture("Root", "One");
        fs::create_dir_all(world.0.join("One/.tick/Two")).unwrap();
        fs::create_dir_all(world.0.join("Two")).unwrap();
        fs::write(world.0.join("One/.tick/Two/main.sayo"), "%0 %0\n").unwrap();

        let arities = analyze(&[world.0.as_path()]).unwrap().into_arities();
        assert_eq!(arities.get("One"), Some(&1));
        assert_eq!(arities.get("Two"), Some(&2));
    }

    #[test]
    fn rejects_out_of_range_payload_reference_in_a_normal_relation() {
        let world = TestWorld::new("normal-reference");
        world.root("Root");
        world.capture("Root", "Box");
        fs::create_dir_all(world.0.join("Value")).unwrap();
        fs::create_dir_all(world.0.join("Box/Value/Value")).unwrap();
        fs::write(world.0.join("Box/Value/Value/main.sayo"), "%1\n").unwrap();

        assert!(matches!(
            analyze(&[world.0.as_path()]),
            Err(Error::World(message)) if message.contains("payload reference %1")
                && message.contains("static arity 1")
        ));
    }

    #[test]
    fn rejects_out_of_range_payload_reference_in_a_template_schema() {
        let world = TestWorld::new("schema-reference");
        world.root("Root");
        world.capture("Root", "Box");
        fs::write(world.0.join("Box/template.sayo"), "CheckSpace{%1, Int}\n").unwrap();

        assert!(matches!(
            analyze(&[world.0.as_path()]),
            Err(Error::World(message)) if message.contains("payload reference %1")
                && message.contains("static arity 1")
        ));
    }

    #[test]
    fn checks_payload_references_in_a_referenced_sayo_file() {
        let world = TestWorld::new("file-reference");
        world.root("Root");
        world.capture("Root", "Box");
        fs::create_dir_all(world.0.join("Value")).unwrap();
        fs::create_dir_all(world.0.join("Box/Value/Value")).unwrap();
        fs::write(world.0.join("Box/Value/Value/main.sayo"), "body.sayo\n").unwrap();
        fs::write(world.0.join("Box/Value/Value/body.sayo"), "%1\n").unwrap();

        assert!(matches!(
            analyze(&[world.0.as_path()]),
            Err(Error::World(message)) if message.contains("payload reference %1")
                && message.contains("body.sayo")
        ));
    }

    #[test]
    fn rejects_external_sayo_files_in_a_template_schema() {
        let world = TestWorld::new("schema-file");
        world.root("Root");
        world.capture("Root", "Box");
        fs::write(world.0.join("Box/template.sayo"), "schema-body.sayo\n").unwrap();

        assert!(matches!(
            analyze(&[world.0.as_path()]),
            Err(Error::World(message)) if message.contains("schema cannot access external Sayo")
        ));
    }

    #[test]
    fn space_checker_accepts_only_percent_zero_and_no_external_source() {
        for (source, expected) in [
            ("equal-to \"x\" %1\n", "only reference %0"),
            ("checker-body.sayo\n", "cannot access external Sayo"),
        ] {
            let world = TestWorld::new("space-checker-restriction");
            fs::create_dir_all(world.0.join("Value")).unwrap();
            fs::write(world.0.join("Value/space.sayo"), source).unwrap();

            assert!(matches!(
                analyze(&[world.0.as_path()]),
                Err(Error::World(message)) if message.contains(expected)
            ));
        }
    }

    #[test]
    fn user_component_completely_shadows_the_same_standard_component() {
        let user = TestWorld::new("shadow-user");
        let standard = TestWorld::new("shadow-standard");
        user.root("Shadowed");
        fs::create_dir_all(standard.0.join("Shadowed")).unwrap();
        fs::write(
            standard.0.join("Shadowed/template.sayo"),
            "CheckSpace{%9, Int}\n",
        )
        .unwrap();

        let arities = analyze(&[user.0.as_path(), standard.0.as_path()])
            .unwrap()
            .into_arities();
        assert_eq!(arities.get("Shadowed"), Some(&0));
    }

    #[test]
    fn shadowed_lower_entry_deactivates_its_unreachable_internal_templates() {
        let user = TestWorld::new("inactive-user");
        let standard = TestWorld::new("inactive-standard");
        user.root("Entry");
        standard.capture("Entry", "Internal");

        let analysis = analyze(&[user.0.as_path(), standard.0.as_path()]).unwrap();
        assert!(analysis.inactive_lower_components().contains("Internal"));
    }
}
