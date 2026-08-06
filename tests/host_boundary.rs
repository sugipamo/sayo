use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn arithmetic_world() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/native_arithmetic/world")
}

fn run(world: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sayo"))
        .env("PATH", "")
        .arg("--world")
        .arg(world)
        .args(arguments)
        .output()
        .unwrap()
}

fn temporary_world() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("sayo-host-boundary-{}-{nonce}", std::process::id()))
}

#[test]
fn stdout_renders_only_the_initial_groups_residual_terms() {
    let output = run(&arithmetic_world(), &["5", "3"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "5 3\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn an_empty_normal_form_has_no_semantic_term_to_render() {
    let world = temporary_world();
    fs::create_dir_all(world.join("Before/.tick/Vanish")).unwrap();
    fs::write(world.join("Before/main.sayo"), "Before {}\n").unwrap();

    let output = run(&world, &["Before", "{}"]);
    let _ = fs::remove_dir_all(&world);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The CLI adds a line terminator as presentation framing; there is no
    // rendered Term before it.
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "\n");
}

#[test]
fn trap_is_a_failed_evaluation_not_an_output_term() {
    let output = run(&arithmetic_world(), &["Trap"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("semantic Trap"));
}

#[test]
fn json_trace_is_one_document_on_stderr() {
    let output = run(
        &arithmetic_world(),
        &["--trace-format", "json", "add", "5", "3"],
    );

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "8\n");
    let document: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(document["format"], "sayo-reduction-ir");
    assert_eq!(document["version"], 1);
    assert!(
        document["events"]
            .as_array()
            .is_some_and(|events| { events.iter().any(|event| event["event"] == "yielded") })
    );
}

#[test]
fn trapped_json_trace_remains_one_valid_document() {
    let output = run(
        &arithmetic_world(),
        &["--trace-format", "json", "div", "5", "0"],
    );

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let document: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    let events = document["events"].as_array().unwrap();
    assert!(events.iter().any(|event| {
        event["event"] == "trapped"
            && event["category"] == "implementation error"
            && event["message"]
                .as_str()
                .is_some_and(|message| message.contains("division by zero"))
    }));
}

#[test]
fn json_trace_rejects_extra_stderr_presentations() {
    let output = run(
        &arithmetic_world(),
        &["--trace-format", "json", "--show-space", "add", "5", "3"],
    );

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("cannot be combined")
    );
}
