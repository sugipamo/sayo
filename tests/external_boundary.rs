use std::path::PathBuf;

use sayo::{
    ExternalKind, ExternalReference, MemoryWorld, Runtime, RuntimeLimits, model::Term, parser,
};

fn test_runtime() -> Runtime {
    Runtime::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/external_requests/world"))
        .unwrap()
}

#[test]
fn external_syntax_is_typed_and_round_trips_without_evaluation() {
    for source in ["Document<blob:input.sayo>", "Agent<handle:coder-1>"] {
        let expression = parser::parse(source).unwrap();
        assert_eq!(expression.render_source(), source);
        assert_eq!(
            parser::parse(&expression.render_source()).unwrap(),
            expression
        );
    }
    assert_eq!(
        parser::parse("Document < blob:input >")
            .unwrap()
            .render_source(),
        "Document<blob:input>"
    );
    let blob = ExternalReference::new(ExternalKind::Blob, "same").unwrap();
    let handle = ExternalReference::new(ExternalKind::Handle, "same").unwrap();
    assert_ne!(blob, handle);
}

#[test]
fn external_payloads_cannot_contain_sayo_expressions_or_inline_contents() {
    for source in [
        "Document<>",
        "Document<blob:>",
        "Document<unknown:input>",
        "Document<blob:input",
        "Document<blob:input extra>",
        "Document<blob:input, handle:coder>",
        "Document<(add 1 2)>",
        "Document<\"raw contents\">",
        "Document<blob:input/other>",
        "Document<blob:input:other>",
        "%0<blob:input>",
        "Document{}<blob:input>",
        "Document<blob:input>{}",
    ] {
        assert!(parser::parse(source).is_err(), "accepted {source}");
    }
}

#[test]
fn reference_ids_are_not_source_files_or_semantic_payloads() {
    let runtime = test_runtime();
    let document = runtime
        .evaluate("Document<blob:missing.sayo>")
        .unwrap()
        .single()
        .unwrap();
    assert_eq!(document.external.as_ref().unwrap().id(), "missing.sayo");
    assert_eq!(document.space, "Document");
    assert!(runtime.payload_terms(&document).unwrap().is_none());
    assert!(runtime.evaluate("Unknown<blob:input>").is_err());
    assert!(runtime.evaluate("Int<blob:input>").is_err());
    assert!(runtime.evaluate("String<blob:input>").is_err());
}

#[test]
fn user_declarations_cannot_turn_native_value_spaces_into_external_roles() {
    let mut world = MemoryWorld::new();
    for role in ["String", "Int", "Ordering", "Pass", "CheckSpace"] {
        world
            .insert_file(format!("{role}/space.sayo"), "equal-to \"unused\" %0")
            .unwrap();
    }
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    for role in ["String", "Int", "Ordering", "Pass", "CheckSpace"] {
        assert!(runtime.evaluate(&format!("{role}<blob:input>")).is_err());
    }
}

#[test]
fn capture_preserves_whole_references_and_outer_payload_roles() {
    let runtime = test_runtime();
    let pending = runtime
        .evaluate("ask Agent<handle:coder> Document<blob:input>")
        .unwrap();
    let request = pending.single().unwrap();
    let payload = runtime.payload_terms(&request).unwrap().unwrap();
    assert_eq!(payload.len(), 2);
    assert_eq!(payload[0].space, "Agent");
    assert_eq!(
        payload[0].external.as_ref().unwrap().kind(),
        ExternalKind::Handle
    );
    assert_eq!(payload[1].space, "Document");
    assert_eq!(
        payload[1].external.as_ref().unwrap().kind(),
        ExternalKind::Blob
    );
    assert_eq!(payload[1].external.as_ref().unwrap().id(), "input");
    let constructed = runtime
        .evaluate("Request{Agent<handle:coder>, Document<blob:input>}")
        .unwrap();
    assert_eq!(
        runtime.display_normal_form(&constructed).unwrap(),
        request.display
    );
}

#[test]
fn fixed_response_resumes_retained_state_and_preserves_term_identity() {
    let runtime = test_runtime();
    let pending = runtime
        .evaluate("ask Agent<handle:coder> Document<blob:input> accept")
        .unwrap();
    let original_agent = runtime.payload_terms(&pending.terms()[0]).unwrap().unwrap()[0].clone();
    let response = runtime
        .evaluate("receive Agent<handle:coder> Code<blob:output>")
        .unwrap();
    let code = response.terms()[0].clone();
    let agent = response.terms()[1].clone();
    assert_ne!(original_agent.term_id, agent.term_id);
    assert_eq!(original_agent.external, agent.external);
    let next = vec![pending.terms()[1].clone(), code.clone(), agent.clone()];
    let complete = runtime.evaluate_terms(next.clone()).unwrap();
    assert_eq!(
        runtime.display_normal_form(&complete).unwrap(),
        "Completed{Code<blob:output>} Agent<handle:coder>"
    );
    assert_eq!(
        runtime
            .payload_terms(&complete.terms()[0])
            .unwrap()
            .unwrap(),
        vec![code]
    );
    assert_eq!(complete.terms()[1], agent);
    // Replaying fixed observations is pure and does not contact the Host.
    let replay = runtime.evaluate_terms(next).unwrap();
    assert_eq!(
        runtime.display_normal_form(&complete).unwrap(),
        runtime.display_normal_form(&replay).unwrap()
    );
}

#[test]
fn materialized_inputs_include_native_states_and_empty_sequences() {
    let runtime = test_runtime();
    let mut terms = runtime.evaluate("add 5").unwrap().into_terms();
    terms.extend(runtime.evaluate("3").unwrap().into_terms());
    assert_eq!(
        runtime
            .display_normal_form(&runtime.evaluate_terms(terms).unwrap())
            .unwrap(),
        "8"
    );
    assert!(
        runtime
            .evaluate_terms(Vec::new())
            .unwrap()
            .terms()
            .is_empty()
    );
}

#[test]
fn host_inputs_cannot_forge_or_cross_runtime_state_ids() {
    let runtime = test_runtime();
    let other = test_runtime();
    let term = runtime
        .evaluate("Agent<handle:coder>")
        .unwrap()
        .single()
        .unwrap();
    let foreign = other
        .evaluate("Agent<handle:coder>")
        .unwrap()
        .single()
        .unwrap();
    assert_eq!(term.term_id, foreign.term_id);
    assert!(runtime.evaluate_terms(vec![foreign.clone()]).is_err());
    assert!(runtime.payload_terms(&foreign).is_err());
    assert!(
        runtime
            .evaluate_terms(vec![Term::atom("agent".into(), "Agent".into())])
            .is_err()
    );
    let mut modified = term;
    modified.external = Some(Box::new(
        ExternalReference::new(ExternalKind::Handle, "different").unwrap(),
    ));
    assert!(runtime.evaluate_terms(vec![modified.clone()]).is_err());
    assert!(runtime.payload_terms(&modified).is_err());
}

#[test]
fn request_occurrences_are_not_deduplicated_or_dispatched_by_the_evaluator() {
    let runtime = test_runtime();
    let pending = runtime.evaluate(
        "(ask Agent<handle:coder> Document<blob:input>) (ask Agent<handle:coder> Document<blob:input>)"
    ).unwrap();
    assert_eq!(pending.terms().len(), 2);
    assert_eq!(pending.terms()[0].display, pending.terms()[1].display);
    assert_ne!(pending.terms()[0].term_id, pending.terms()[1].term_id);
    assert!(
        runtime
            .evaluate("ask Agent<handle:coder> Document<blob:input> Trap")
            .is_err()
    );
    let consumed = runtime
        .evaluate("receive Agent<handle:coder> Code<blob:output>")
        .unwrap();
    assert!(consumed.terms().iter().all(|term| term.space != "Response"));
}

#[test]
fn restricted_space_checkers_cannot_construct_external_values() {
    let mut world = MemoryWorld::new();
    world
        .insert_file("Agent/space.sayo", "Agent<handle:coder>")
        .unwrap();
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    assert!(runtime.evaluate("candidate").is_err());
}

#[test]
fn external_roles_do_not_run_source_classifiers_or_tick_payload_rules() {
    let mut world = MemoryWorld::new();
    world.insert_file("Document/space.sayo", "div 1 0").unwrap();
    world
        .insert_file("Document/.tick/Int/main.sayo", "42")
        .unwrap();
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    let term = runtime
        .evaluate("Document<blob:input>")
        .unwrap()
        .single()
        .unwrap();
    assert_eq!(term.display, "Document<blob:input>");
    assert!(runtime.evaluate("document").is_err());
}

#[test]
fn external_references_survive_trace_and_browser_ir_as_atomic_terms() {
    let runtime = test_runtime();
    let evaluation = runtime.evaluate_with_trace("ask Agent<handle:coder> Document<blob:input>");
    let normal = evaluation.result.unwrap();
    assert_eq!(normal.terms()[0].space, "Request");
    let json: serde_json::Value = serde_json::from_str(&evaluation.trace.render_json()).unwrap();
    assert!(json.to_string().contains("Document<blob:input>"));
    // Exercise the same host-neutral API used by the Wasm Playground.
    let files = std::collections::BTreeMap::from([
        ("Agent/space.sayo", "equal-to \"agent\" %0"),
        ("program/main.sayo", "Agent<handle:coder>"),
    ]);
    let request = serde_json::json!({"version": 1, "entry": "program/main.sayo", "files": files});
    let response: serde_json::Value =
        serde_json::from_str(&sayo::evaluate_web_json(&request.to_string())).unwrap();
    assert_eq!(response["status"], "normal");
    assert_eq!(response["normal_form"], "Agent<handle:coder>");
    assert!(response["visualization"].is_object());
}
