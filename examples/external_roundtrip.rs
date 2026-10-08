//! A deterministic test Host; no network, real Agent, or evaluator callback.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sayo::{ExternalKind, Result, Runtime, model::Term};

fn main() -> Result<()> {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "upper:hello".into());
    let world = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/external_requests/world");
    let runtime = Runtime::new(world)?;
    let pending = runtime.evaluate(include_str!("external_requests/main.sayo"))?;
    println!("request: {}", runtime.display_normal_form(&pending)?);

    // The host chooses to execute this one top-level Request occurrence once.
    let [request, waiting] = pending.terms() else {
        panic!("expected Request and Accept")
    };
    let payload = runtime.payload_terms(request)?.expect("Request payload");
    let [agent, document] = payload.as_slice() else {
        panic!("expected Agent and Document")
    };
    let mut host = TestHost::new(input.into_bytes());
    let response_source = host.process(agent, document);

    // Host parsing of the response envelope is outside the pure evaluator.
    // Sayo sees only already-separated, fixed references in the response.
    let response = runtime.evaluate(&response_source)?;
    let mut next = vec![waiting.clone()];
    next.extend(response.into_terms());
    let complete = runtime.evaluate_terms(next)?;
    println!("result: {}", runtime.display_normal_form(&complete)?);
    println!(
        "host output: {}",
        String::from_utf8_lossy(&host.blobs["output-1"])
    );
    println!("host calls: {}", host.calls);
    Ok(())
}

struct TestHost {
    blobs: BTreeMap<String, Vec<u8>>,
    calls: usize,
}

impl TestHost {
    fn new(input: Vec<u8>) -> Self {
        Self {
            blobs: BTreeMap::from([("input".into(), input)]),
            calls: 0,
        }
    }

    fn process(&mut self, agent: &Term, document: &Term) -> String {
        let agent = agent.external.as_ref().expect("Agent reference");
        assert_eq!(agent.kind(), ExternalKind::Handle);
        assert_eq!(agent.id(), "coder");
        let document = document.external.as_ref().expect("Document reference");
        assert_eq!(document.kind(), ExternalKind::Blob);
        let input = &self.blobs[document.id()];
        // Only this host processor reads contents and branches on them.
        let output = if let Some(text) = input.strip_prefix(b"upper:") {
            text.to_ascii_uppercase()
        } else {
            input.clone()
        };
        self.calls += 1;
        let id = format!("output-{}", self.calls);
        assert!(self.blobs.insert(id.clone(), output).is_none());
        // Same connection, new Agent Term. A handle is not an Agent snapshot.
        format!("receive Agent<handle:coder> Code<blob:{id}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_contents_can_change_processing_without_changing_pure_request() {
        let world =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/external_requests/world");
        let runtime = Runtime::new(world).unwrap();
        let pending = runtime
            .evaluate(include_str!("external_requests/main.sayo"))
            .unwrap();
        let request = &pending.terms()[0];
        let payload = runtime.payload_terms(request).unwrap().unwrap();
        for (input, expected) in [
            (b"upper:hello".as_slice(), b"HELLO".as_slice()),
            (b"hello", b"hello"),
        ] {
            let mut host = TestHost::new(input.to_vec());
            assert_eq!(host.calls, 0);
            let response = runtime
                .evaluate(&host.process(&payload[0], &payload[1]))
                .unwrap();
            assert_eq!(host.calls, 1);
            assert_eq!(host.blobs["input"], input);
            assert_eq!(host.blobs["output-1"], expected);
            let mut next = vec![pending.terms()[1].clone()];
            next.extend(response.into_terms());
            let complete = runtime.evaluate_terms(next).unwrap();
            assert_eq!(
                runtime.display_normal_form(&complete).unwrap(),
                "Completed{Code<blob:output-1>} Agent<handle:coder>"
            );
        }
    }
}
