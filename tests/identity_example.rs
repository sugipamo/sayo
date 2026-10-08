use std::path::PathBuf;
use std::process::Command;

#[test]
fn checked_in_identity_example_validates_and_evaluates() {
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/identity_relation");
    let world = example.join("world");
    let check = Command::new(env!("CARGO_BIN_EXE_sayo"))
        .args(["check", "--world"])
        .arg(&world)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "identity World validation failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    let evaluate = Command::new(env!("CARGO_BIN_EXE_sayo"))
        .arg("--world")
        .arg(&world)
        .arg(example.join("main.sayo"))
        .output()
        .unwrap();
    assert!(
        evaluate.status.success(),
        "identity example failed: {}",
        String::from_utf8_lossy(&evaluate.stderr)
    );
    assert_eq!(String::from_utf8(evaluate.stdout).unwrap(), "42\n");
}
