//! Verifies that SayoVM does not depend on executables discovered through PATH.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_without_external_executables(arguments: &[&str]) -> Output {
    let world = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/native_arithmetic/world");
    let mut command = Command::new(env!("CARGO_BIN_EXE_sayo"));
    command.env("PATH", "").arg("--world").arg(world);
    command.args(arguments).output().unwrap()
}

fn assert_evaluation(arguments: &[&str], expected: &str) {
    let output = run_without_external_executables(arguments);
    assert!(
        output.status.success(),
        "sayo failed without external executables: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[test]
fn standard_language_features_run_without_external_executables() {
    for (arguments, expected) in [
        (vec!["add", "5", "3"], "8\n"),
        (vec!["sub", "5", "3"], "2\n"),
        (vec!["mul", "5", "3"], "15\n"),
        (vec!["div", "6", "3"], "2\n"),
        (vec!["compare", "5", "9"], "Less\n"),
        (vec!["duplicate", "5"], "5 5 duplicate\n"),
        (
            vec!["for", "(add 9)", "1", "2", "3"],
            "10 11 12 Each{Plus{9}}\n",
        ),
        (
            vec!["filter", "odd", "1", "2", "3", "4"],
            "1 3 Sieve{odd}\n",
        ),
        (vec!["fold", "add", "0", "1", "2", "3"], "6\n"),
        (vec!["reduce", "add", "1", "2", "3"], "6\n"),
    ] {
        assert_evaluation(&arguments, expected);
    }
}

#[test]
fn arithmetic_world_validation_needs_no_external_executable() {
    let world = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/native_arithmetic/world");
    let output = Command::new(env!("CARGO_BIN_EXE_sayo"))
        .env("PATH", "")
        .args(["check", "--world"])
        .arg(world)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "sayo check failed without external executables: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("0 Sayo implementation(s)"));
}

#[test]
fn sayo_space_checker_needs_no_external_executable() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let world = std::env::temp_dir().join(format!(
        "sayo-no-external-executor-checker-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(world.join("Hello")).unwrap();
    fs::write(world.join("Hello/space.sayo"), "equal-to \"hello\" %0\n").unwrap();

    let check = Command::new(env!("CARGO_BIN_EXE_sayo"))
        .env("PATH", "")
        .args(["check", "--world"])
        .arg(&world)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "checker World validation failed without external executables: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    let evaluate = Command::new(env!("CARGO_BIN_EXE_sayo"))
        .env("PATH", "")
        .arg("--world")
        .arg(&world)
        .arg("--show-space")
        .arg("hello")
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&world);

    assert!(
        evaluate.status.success(),
        "checker evaluation failed without external executables: {}",
        String::from_utf8_lossy(&evaluate.stderr)
    );
    assert_eq!(String::from_utf8(evaluate.stdout).unwrap(), "hello\n");
    assert_eq!(
        String::from_utf8(evaluate.stderr).unwrap(),
        "space: Hello\n"
    );
}
