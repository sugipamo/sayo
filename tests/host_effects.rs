#![cfg(all(feature = "host-effects", not(target_arch = "wasm32")))]

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sayo::host::{EffectHost, HostLimits, effect_world};
use sayo::{ExternalKind, ExternalReference, NormalForm, Runtime, RuntimeLimits};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sayo-host-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn runtime() -> Runtime {
    Runtime::from_memory_world(effect_world().unwrap(), RuntimeLimits::default()).unwrap()
}
fn host() -> EffectHost {
    EffectHost::new(HostLimits::default()).unwrap()
}
fn request(runtime: &Runtime, handle: &str, blob: &str) -> sayo::model::Term {
    runtime
        .evaluate(&format!(
            "Request{{Service<handle:{handle}>, Data<blob:{blob}>}}"
        ))
        .unwrap()
        .single()
        .unwrap()
}
fn failure(host: &EffectHost, observation: &sayo::host::EffectObservation) -> serde_json::Value {
    assert!(!observation.succeeded);
    assert!(
        observation.response.terms()[0]
            .display
            .starts_with("Failed{")
    );
    serde_json::from_slice(host.blob(&observation.output).unwrap()).unwrap()
}

#[test]
fn real_files_are_binary_and_writes_are_cached_without_overwriting() {
    let directory = Directory::new();
    let input = directory.0.join("input");
    let output = directory.0.join("output");
    let bytes = b"hello\0\xff\n";
    fs::write(&input, bytes).unwrap();
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("empty", vec![]).unwrap();
    host.register_file_read("read", input).unwrap();
    host.register_file_write("write", &output).unwrap();
    assert!(
        !output.exists(),
        "registering a writer must not create the file"
    );
    let read = host
        .execute(&runtime, &request(&runtime, "read", "empty"))
        .unwrap();
    assert!(read.succeeded);
    assert_eq!(host.blob(&read.output).unwrap(), bytes);
    let write_request = request(&runtime, "write", read.output.id());
    let written = host.execute(&runtime, &write_request).unwrap();
    assert!(written.succeeded);
    assert_eq!(fs::read(&output).unwrap(), bytes);
    let again = host.execute(&runtime, &write_request).unwrap();
    assert_eq!(again.output, written.output);
    assert!(again.succeeded);
    let second_occurrence = host
        .execute(&runtime, &request(&runtime, "write", "empty"))
        .unwrap();
    assert_eq!(failure(&host, &second_occurrence)["code"], "io");
    assert_eq!(fs::read(&output).unwrap(), bytes);
    assert_eq!(host.blob(&read.output).unwrap(), bytes);
}

#[test]
fn clock_and_os_random_are_fixed_observations_on_replay() {
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("empty", vec![]).unwrap();
    host.insert_blob("size", br#"{"bytes":32}"#.to_vec())
        .unwrap();
    host.register_clock("clock").unwrap();
    host.register_random("random").unwrap();
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let clock = host
        .execute(&runtime, &request(&runtime, "clock", "empty"))
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(host.blob(&clock.output).unwrap()).unwrap();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let millis = value["unix_ms"].as_u64().unwrap() as u128;
    assert!((before..=after).contains(&millis));
    let random_request = request(&runtime, "random", "size");
    let random = host.execute(&runtime, &random_request).unwrap();
    assert!(random.succeeded);
    assert_eq!(host.blob(&random.output).unwrap().len(), 32);
    let replay = runtime
        .evaluate_terms(random.response.clone().into_terms())
        .unwrap();
    assert_eq!(
        runtime.display_normal_form(&replay).unwrap(),
        runtime.display_normal_form(&random.response).unwrap()
    );
    let again = host.execute(&runtime, &random_request).unwrap();
    assert_eq!(again.output, random.output);
    assert_eq!(again.response.terms(), random.response.terms());
}

#[test]
fn handles_and_blobs_are_explicit_and_immutable() {
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("input", b"message".to_vec()).unwrap();
    assert!(host.insert_blob("input", b"replacement".to_vec()).is_err());
    let input = ExternalReference::new(ExternalKind::Blob, "input").unwrap();
    assert_eq!(host.blob(&input).unwrap(), b"message");
    let unknown = host
        .execute(&runtime, &request(&runtime, "missing", "input"))
        .unwrap();
    assert_eq!(failure(&host, &unknown)["code"], "unknown_handle");
    host.register_clock("clock").unwrap();
    assert!(host.register_random("clock").is_err());
    let missing = host
        .execute(&runtime, &request(&runtime, "clock", "missing"))
        .unwrap();
    assert_eq!(failure(&host, &missing)["code"], "missing_blob");
    let input_handle = ExternalReference::new(ExternalKind::Handle, "input").unwrap();
    assert!(host.blob(&input_handle).is_none());
}

#[test]
fn sequential_occurrences_are_not_deduplicated_by_blob_or_handle() {
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("input", b"hello".to_vec()).unwrap();
    let sink = Sink::default();
    host.register_notification("notice", sink.clone()).unwrap();
    let pending = runtime.evaluate("Request{Service<handle:notice>, Data<blob:input>} Request{Service<handle:notice>, Data<blob:input>}").unwrap();
    assert!(
        sink.0.lock().unwrap().is_empty(),
        "pure evaluation must not dispatch"
    );
    let completed = host.run(&runtime, pending).unwrap();
    assert_eq!(completed.observations.len(), 2);
    assert_ne!(
        completed.observations[0].request.term_id,
        completed.observations[1].request.term_id
    );
    assert_eq!(*sink.0.lock().unwrap(), b"hellohello");
    assert_eq!(completed.normal_form.terms().len(), 2);
}

#[test]
fn duplicate_identity_is_cached_but_different_runtimes_do_not_collide() {
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("input", b"hello".to_vec()).unwrap();
    let sink = Sink::default();
    host.register_notification("notice", sink.clone()).unwrap();
    let request = request(&runtime, "notice", "input");
    let result = host
        .run(
            &runtime,
            NormalForm::new(vec![request.clone(), request.clone()]),
        )
        .unwrap();
    assert_eq!(*sink.0.lock().unwrap(), b"hello");
    assert_eq!(result.observations[0].output, result.observations[1].output);
    let other = self::runtime();
    let foreign = self::request(&other, "notice", "input");
    assert!(host.execute(&runtime, &foreign).is_err());
    let second = host.execute(&other, &foreign).unwrap();
    assert!(second.succeeded);
    assert_eq!(*sink.0.lock().unwrap(), b"hellohello");
}

#[test]
fn response_rules_can_generate_the_next_request_in_a_pure_interval() {
    let mut world = effect_world().unwrap();
    world
        .insert_file("Succeeded/.tick/.any/main.sayo", "%0")
        .unwrap();
    world
        .insert_file("Next/space.sayo", "equal-to \"next\" %0")
        .unwrap();
    world.create_dir("Next/Data/Forward").unwrap();
    world
        .insert_file(
            "Forward/.tick/.any/main.sayo",
            "Request{Service<handle:notice>, %0}",
        )
        .unwrap();
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    let mut host = host();
    let sink = Sink::default();
    host.register_clock("clock").unwrap();
    host.register_notification("notice", sink.clone()).unwrap();
    host.insert_blob("empty", vec![]).unwrap();
    let pending = runtime
        .evaluate("next Request{Service<handle:clock>, Data<blob:empty>}")
        .unwrap();
    let result = host.run(&runtime, pending).unwrap();
    assert_eq!(result.observations.len(), 2);
    assert_eq!(
        *sink.0.lock().unwrap(),
        host.blob(&result.observations[0].output).unwrap()
    );
    assert_eq!(result.normal_form.terms()[0].space, "Data");
}

#[test]
fn nested_requests_are_not_dispatched() {
    let mut world = effect_world().unwrap();
    world
        .insert_file("Envelope/template.sayo", "Pass{}")
        .unwrap();
    world
        .insert_file("Wrap/space.sayo", "equal-to \"wrap\" %0")
        .unwrap();
    world.create_dir("Wrap/Request/Envelope").unwrap();
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    let mut host = host();
    host.insert_blob("input", b"hello".to_vec()).unwrap();
    let sink = Sink::default();
    host.register_notification("notice", sink.clone()).unwrap();
    let pending = runtime
        .evaluate("Envelope{Request{Service<handle:notice>, Data<blob:input>}}")
        .unwrap();
    let result = host.run(&runtime, pending).unwrap();
    assert!(result.observations.is_empty());
    assert!(sink.0.lock().unwrap().is_empty());
}

#[test]
fn invalid_response_world_and_forged_terms_cannot_trigger_effects() {
    let mut world = effect_world().unwrap();
    world
        .insert_file("Failed/template.sayo", "CheckSpace{%0, Service}")
        .unwrap();
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    let mut host = host();
    host.insert_blob("input", b"hello".to_vec()).unwrap();
    let sink = Sink::default();
    host.register_notification("notice", sink.clone()).unwrap();
    let mut request = request(&runtime, "notice", "input");
    assert!(host.execute(&runtime, &request).is_err());
    assert!(host.execute(&runtime, &request).is_err());
    request.display.push_str("changed");
    assert!(host.run(&runtime, NormalForm::one(request)).is_err());
    assert!(sink.0.lock().unwrap().is_empty());
}

#[test]
fn partial_notification_failure_is_not_retried() {
    struct BrokenSink(Sink);
    impl Write for BrokenSink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.0.lock().unwrap().is_empty() {
                self.0.write(&bytes[..1])
            } else {
                Err(std::io::Error::other("sink failed"))
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("input", b"hello".to_vec()).unwrap();
    let sink = Sink::default();
    host.register_notification("notice", BrokenSink(sink.clone()))
        .unwrap();
    let request = request(&runtime, "notice", "input");
    let observation = host.execute(&runtime, &request).unwrap();
    assert_eq!(
        failure(&host, &observation)["effects_may_have_occurred"],
        true
    );
    let replay = host.execute(&runtime, &request).unwrap();
    assert_eq!(observation.output, replay.output);
    assert_eq!(*sink.0.lock().unwrap(), b"h");
}

#[test]
fn limits_and_invalid_parameters_prevent_unbounded_effects() {
    let directory = Directory::new();
    let input = directory.0.join("large");
    fs::write(&input, vec![0; 1025]).unwrap();
    let runtime = runtime();
    let mut host = EffectHost::new(HostLimits {
        max_blob_bytes: 1024,
        ..HostLimits::default()
    })
    .unwrap();
    assert!(host.insert_blob("large", vec![0; 1025]).is_err());
    host.insert_blob("empty", vec![]).unwrap();
    host.insert_blob("large-size", br#"{"bytes":1025}"#.to_vec())
        .unwrap();
    host.insert_blob("invalid-size", br#"{"bytes":3,"unknown":1}"#.to_vec())
        .unwrap();
    host.register_file_read("read", input).unwrap();
    host.register_random("random").unwrap();
    for (handle, blob, code) in [
        ("read", "empty", "size_limit"),
        ("random", "large-size", "size_limit"),
        ("random", "invalid-size", "invalid_input"),
    ] {
        let result = host
            .execute(&runtime, &request(&runtime, handle, blob))
            .unwrap();
        assert_eq!(failure(&host, &result)["code"], code);
    }
    let mut tiny = EffectHost::new(HostLimits {
        max_blob_bytes: 1024,
        max_storage_bytes: 1024,
        ..HostLimits::default()
    })
    .unwrap();
    tiny.insert_blob("input", vec![0]).unwrap();
    let sink = Sink::default();
    tiny.register_notification("notice", sink.clone()).unwrap();
    assert!(
        tiny.execute(&runtime, &request(&runtime, "notice", "input"))
            .is_err()
    );
    assert!(sink.0.lock().unwrap().is_empty());
}

#[test]
fn request_and_interval_limits_stop_before_the_next_side_effect() {
    let runtime = runtime();
    let sink = Sink::default();
    let mut limited = EffectHost::new(HostLimits {
        max_requests: 1,
        ..HostLimits::default()
    })
    .unwrap();
    limited.insert_blob("input", b"hello".to_vec()).unwrap();
    limited
        .register_notification("notice", sink.clone())
        .unwrap();
    let pending = runtime.evaluate("Request{Service<handle:notice>, Data<blob:input>} Request{Service<handle:notice>, Data<blob:input>}").unwrap();
    assert!(limited.run(&runtime, pending).is_err());
    assert_eq!(*sink.0.lock().unwrap(), b"hello");
    assert_eq!(limited.observations().count(), 1);

    let mut world = effect_world().unwrap();
    world
        .insert_file(
            "Succeeded/.tick/.any/main.sayo",
            "Request{Service<handle:notice>, %0}",
        )
        .unwrap();
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    let sink = Sink::default();
    let mut limited = EffectHost::new(HostLimits {
        max_intervals: 1,
        ..HostLimits::default()
    })
    .unwrap();
    limited.insert_blob("input", b"hello".to_vec()).unwrap();
    limited
        .register_notification("notice", sink.clone())
        .unwrap();
    let pending = NormalForm::one(request(&runtime, "notice", "input"));
    assert!(limited.run(&runtime, pending).is_err());
    assert_eq!(*sink.0.lock().unwrap(), b"hello");
}

#[test]
fn reference_kind_validation_cannot_grant_an_operation() {
    let runtime = runtime();
    let mut host = host();
    let sink = Sink::default();
    host.insert_blob("input", b"hello".to_vec()).unwrap();
    host.register_notification("notice", sink.clone()).unwrap();
    for source in [
        "Request{Service<blob:input>, Data<blob:input>}",
        "Request{Service<handle:notice>, Data<handle:notice>}",
    ] {
        let request = runtime.evaluate(source).unwrap().single().unwrap();
        let observation = host.execute(&runtime, &request).unwrap();
        assert_eq!(failure(&host, &observation)["code"], "invalid_request");
    }
    assert!(sink.0.lock().unwrap().is_empty());
}

fn server(response: Vec<u8>, delay: Duration) -> (String, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/resource", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(e) => panic!("test server could not accept: {e}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
            assert!(bytes.len() < 8192);
            if bytes.ends_with(b"\r\n\r\n") {
                break bytes.len();
            }
        };
        let content_length = String::from_utf8_lossy(&bytes)
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        assert!(content_length < 8192);
        bytes.resize(header_end + content_length, 0);
        socket.read_exact(&mut bytes[header_end..]).unwrap();
        std::thread::sleep(delay);
        let _ = socket.write_all(&response);
        bytes
    });
    (url, thread)
}

#[test]
fn http_posts_real_bytes_and_records_a_fixed_body() {
    let (url, server) = server(
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nhi\0!".to_vec(),
        Duration::ZERO,
    );
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("body", b"request\0bytes".to_vec())
        .unwrap();
    host.register_http_with_headers(
        "http",
        "POST",
        &url,
        std::collections::BTreeMap::from([("X-Sayo-Test".into(), "configured".into())]),
    )
    .unwrap();
    let request = request(&runtime, "http", "body");
    let observation = host.execute(&runtime, &request).unwrap();
    assert!(observation.succeeded);
    assert_eq!(host.blob(&observation.output).unwrap(), b"hi\0!");
    let received = server.join().unwrap();
    assert!(received.starts_with(b"POST /resource HTTP/1.1\r\n"));
    assert!(received.ends_with(b"request\0bytes"));
    assert!(
        String::from_utf8_lossy(&received)
            .to_ascii_lowercase()
            .contains("x-sayo-test: configured\r\n")
    );
    assert_eq!(
        host.execute(&runtime, &request).unwrap().output,
        observation.output
    );
}

#[test]
fn http_redirects_and_large_responses_are_failures() {
    for (response, code) in [
        (b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(), "http_status"),
        ([b"HTTP/1.1 200 OK\r\nContent-Length: 1025\r\nConnection: close\r\n\r\n".as_slice(), &vec![0; 1025]].concat(), "size_limit"),
    ] {
        let (url, server) = server(response, Duration::ZERO);
        let runtime = runtime();
        let mut host = EffectHost::new(HostLimits { max_blob_bytes: 1024, ..HostLimits::default() }).unwrap();
        host.insert_blob("empty", vec![]).unwrap();
        host.register_http("http", "GET", &url).unwrap();
        let observation = host.execute(&runtime, &request(&runtime, "http", "empty")).unwrap();
        let failure = failure(&host, &observation);
        assert_eq!(failure["code"], code);
        assert_eq!(failure["effects_may_have_occurred"], true);
        server.join().unwrap();
    }
}

#[test]
fn http_timeout_is_bounded_and_cached() {
    let (url, server) = server(
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
        Duration::from_millis(200),
    );
    let runtime = runtime();
    let mut host = EffectHost::new(HostLimits {
        http_timeout_ms: 50,
        ..HostLimits::default()
    })
    .unwrap();
    host.insert_blob("empty", vec![]).unwrap();
    host.register_http("http", "GET", &url).unwrap();
    let request = request(&runtime, "http", "empty");
    let result = host.execute(&runtime, &request).unwrap();
    assert_eq!(failure(&host, &result)["code"], "http_transport");
    assert_eq!(
        host.execute(&runtime, &request).unwrap().output,
        result.output
    );
    server.join().unwrap();
}

#[test]
fn native_cli_runs_the_checked_in_example_and_exports_binary_observations() {
    let directory = Directory::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/host_effects");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sayo-host"))
        .arg("--config")
        .arg(root.join("host.json"))
        .arg("--export-blobs")
        .arg(directory.0.join("blobs"))
        .arg(root.join("main.sayo"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Hello from the Sayo Host!"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("3 observation(s), 0 failed"));
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .matches("Succeeded{")
            .count(),
        3
    );
    let files = fs::read_dir(directory.0.join("blobs"))
        .unwrap()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        files
            .iter()
            .any(|file| fs::read(file.path()).unwrap().len() == 16)
    );
}

#[test]
fn structured_http_preserves_binary_failure_body_and_replays_without_io() {
    let body = b"Failed{not Sayo}\0\xff";
    let response = [
        format!(
            "HTTP/1.1 503 Unavailable\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
        body,
    ]
    .concat();
    let (url, server) = server(response, Duration::ZERO);
    let runtime = runtime();
    let mut host = host();
    host.insert_blob("input", b"sample".to_vec()).unwrap();
    host.register_http_response_with_headers("web", "POST", &url, Default::default())
        .unwrap();
    let request = request(&runtime, "web", "input");
    let result = host.execute(&runtime, &request).unwrap();
    assert!(!result.succeeded);
    assert_eq!(result.http_status, Some(503));
    assert_eq!(host.blob(&result.output).unwrap(), body);
    let expected = format!(
        "HttpResponse{{HttpStatus{{503}}, Failed{{Data<{}>}}}}",
        result.output.render_source()
    );
    assert_eq!(
        runtime.display_normal_form(&result.response).unwrap(),
        expected
    );
    assert!(server.join().unwrap().ends_with(b"sample"));
    assert_eq!(
        host.execute(&runtime, &request).unwrap().output,
        result.output
    );
    let replay = runtime
        .evaluate_terms(result.response.into_terms())
        .unwrap();
    assert_eq!(runtime.display_normal_form(&replay).unwrap(), expected);
    assert_eq!(host.observations().count(), 1);
}

#[test]
fn structured_http_reports_status_availability_for_body_and_transport_failures() {
    for (response, delay, status, code) in [
        (
            [
                b"HTTP/1.1 200 OK\r\nContent-Length: 1025\r\nConnection: close\r\n\r\n".as_slice(),
                &vec![0; 1025],
            ]
            .concat(),
            Duration::ZERO,
            Some(200),
            "size_limit",
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            Duration::from_millis(200),
            None,
            "http_transport",
        ),
    ] {
        let (url, server) = server(response, delay);
        let runtime = runtime();
        let mut host = EffectHost::new(HostLimits {
            max_blob_bytes: 1024,
            http_timeout_ms: if status.is_some() { 5000 } else { 50 },
            ..HostLimits::default()
        })
        .unwrap();
        host.insert_blob("empty", vec![]).unwrap();
        host.register_http_response_with_headers("web", "GET", &url, Default::default())
            .unwrap();
        let request = request(&runtime, "web", "empty");
        let result = host.execute(&runtime, &request).unwrap();
        assert!(!result.succeeded);
        assert_eq!(result.http_status, status);
        let diagnostic: serde_json::Value =
            serde_json::from_slice(host.blob(&result.output).unwrap()).unwrap();
        assert_eq!(diagnostic["code"], code);
        assert_eq!(diagnostic["effects_may_have_occurred"], true);
        let displayed = runtime.display_normal_form(&result.response).unwrap();
        assert!(displayed.contains("Failed{"));
        assert!(displayed.contains(if status.is_some() {
            "HttpStatus{200}"
        } else {
            "NoHttpStatus{}"
        }));
        assert_eq!(
            host.execute(&runtime, &request).unwrap().output,
            result.output
        );
        server.join().unwrap();
    }
}

#[test]
fn structured_http_checks_metadata_world_before_sending() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut world = effect_world().unwrap();
    world
        .insert_file("HttpStatus/template.sayo", "CheckSpace{%0, String}")
        .unwrap();
    let runtime = Runtime::from_memory_world(world, RuntimeLimits::default()).unwrap();
    let mut host = host();
    host.insert_blob("empty", vec![]).unwrap();
    host.register_http_response_with_headers(
        "web",
        "GET",
        &format!("http://{}/sample", listener.local_addr().unwrap()),
        Default::default(),
    )
    .unwrap();
    let request = request(&runtime, "web", "empty");
    assert!(host.execute(&runtime, &request).is_err());
    assert!(host.execute(&runtime, &request).is_err());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn pipeline(directory: &Directory, url: &str) -> std::process::Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/file_http_pipeline");
    let config = serde_json::json!({
        "blobs": {"empty": ""},
        "services": {
            "reader": {"kind": "read-file", "path": "input"},
            "web": {"kind": "http", "method": "POST", "url": url, "response_metadata": true},
            "writer": {"kind": "write-file", "path": "output"}
        }
    });
    fs::write(
        directory.0.join("host.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sayo-host"))
        .arg("--config")
        .arg(directory.0.join("host.json"))
        .arg("--world")
        .arg(root.join("world"))
        .arg("--export-blobs")
        .arg(directory.0.join("blobs"))
        .arg(root.join("main.sayo"))
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn pipeline_cli_saves_only_success_bodies_and_stops_on_http_failure() {
    let input = b"synthetic request\0\xff";
    let body = b"synthetic response\0\xfe";
    for status in [201, 503, 302] {
        let directory = Directory::new();
        fs::write(directory.0.join("input"), input).unwrap();
        let response = [
            format!(
                "HTTP/1.1 {status} Sample\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
            body,
        ]
        .concat();
        let (url, server) = server(response, Duration::ZERO);
        let output = pipeline(&directory, &url);
        assert!(server.join().unwrap().ends_with(input));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains(&format!("HttpStatus{{{status}}}")),
            "{stdout}"
        );
        if status == 201 {
            assert!(stdout.starts_with("Saved{Data<blob:"), "{stdout}");
            assert_eq!(fs::read(directory.0.join("output")).unwrap(), body);
            assert!(String::from_utf8_lossy(&output.stderr).contains("3 observation(s), 0 failed"));
        } else {
            assert!(stdout.starts_with("HttpFailed{Data<blob:"), "{stdout}");
            assert!(!directory.0.join("output").exists());
            assert!(String::from_utf8_lossy(&output.stderr).contains("2 observation(s), 1 failed"));
            assert!(
                fs::read_dir(directory.0.join("blobs"))
                    .unwrap()
                    .any(|entry| fs::read(entry.unwrap().path()).unwrap() == body)
            );
        }
    }
}

#[test]
fn pipeline_read_failure_makes_no_http_request_or_output_file() {
    let directory = Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let output = pipeline(
        &directory,
        &format!("http://{}/sample", listener.local_addr().unwrap()),
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("ReadFailed{Data<blob:"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("1 observation(s), 1 failed"));
    assert!(!directory.0.join("output").exists());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn pipeline_save_failure_preserves_existing_file_without_retrying_http() {
    let directory = Directory::new();
    fs::write(directory.0.join("input"), b"synthetic request").unwrap();
    fs::write(directory.0.join("output"), b"keep original").unwrap();
    let (url, server) = server(
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnew".to_vec(),
        Duration::ZERO,
    );
    let output = pipeline(&directory, &url);
    assert!(server.join().unwrap().ends_with(b"synthetic request"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("SaveFailed{Data<blob:"), "{stdout}");
    assert!(stdout.contains("HttpStatus{200}"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("3 observation(s), 1 failed"));
    assert_eq!(
        fs::read(directory.0.join("output")).unwrap(),
        b"keep original"
    );
}
