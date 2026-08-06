use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use sayo::{Runtime, TermImplementation, check};

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("sayo: {}: {error}", error.category());
            ExitCode::FAILURE
        }
    }
}

fn run() -> sayo::Result<ExitCode> {
    let raw_arguments = env::args().skip(1).collect::<Vec<_>>();
    if raw_arguments
        .first()
        .is_some_and(|argument| argument == "check")
    {
        return run_check(&raw_arguments[1..]).map(|()| ExitCode::SUCCESS);
    }
    let mut arguments = raw_arguments.into_iter();
    let mut world = PathBuf::from("world");
    let mut show_space = false;
    let mut show_kind = false;
    let mut trace_format = None;
    let mut expression = Vec::new();
    let mut end_options = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" if !end_options => {
                print_help();
                return Ok(ExitCode::SUCCESS);
            }
            "--world" if !end_options => {
                world = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| sayo::Error::Usage("--world needs a path".into()))?;
            }
            "--show-space" if !end_options => show_space = true,
            "--show-kind" if !end_options => show_kind = true,
            "--trace-format" if !end_options => match arguments.next().as_deref() {
                Some("json") => trace_format = Some(()),
                Some(format) => {
                    return Err(sayo::Error::Usage(format!(
                        "unknown trace format: {format}; expected json"
                    )));
                }
                None => {
                    return Err(sayo::Error::Usage("--trace-format needs json".into()));
                }
            },
            "--" if !end_options => end_options = true,
            _ if !end_options && argument.starts_with('-') && !is_negative_integer(&argument) => {
                return Err(sayo::Error::Usage(format!("unknown option: {argument}")));
            }
            _ => expression.push(argument),
        }
    }
    if expression.is_empty() {
        return Err(sayo::Error::Parse("no expression supplied".into()));
    }
    if trace_format.is_some() && (show_space || show_kind) {
        return Err(sayo::Error::Usage(
            "--trace-format json cannot be combined with --show-space or --show-kind".into(),
        ));
    }
    let runtime = Runtime::new(world)?;
    let is_file = expression.len() == 1 && PathBuf::from(&expression[0]).is_file();
    let normal_form = if trace_format.is_some() {
        let traced = if is_file {
            runtime.evaluate_file_with_trace(&expression[0])
        } else {
            runtime.evaluate_with_trace(&expression.join(" "))
        };
        eprint!("{}", traced.trace.render_json());
        match traced.result {
            Ok(normal_form) => normal_form,
            Err(error) => {
                let _ = error;
                return Ok(ExitCode::FAILURE);
            }
        }
    } else if is_file {
        runtime.evaluate_file(&expression[0])?
    } else {
        runtime.evaluate(&expression.join(" "))?
    };
    let display = runtime.display_normal_form(&normal_form)?;
    print!("{display}");
    if !display.ends_with('\n') {
        println!();
    }
    for term in normal_form.terms() {
        if show_space {
            eprintln!("space: {}", term.space);
        }
        if show_kind {
            let kind = match &term.implementation {
                TermImplementation::Sayo(_) => "sayo",
                TermImplementation::Native(_) => "native",
                TermImplementation::None => "atom",
            };
            eprintln!("implementation: {kind}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn is_negative_integer(argument: &str) -> bool {
    argument.len() > 1
        && argument[1..]
            .chars()
            .all(|character| character.is_ascii_digit())
}

fn run_check(arguments: &[String]) -> sayo::Result<()> {
    let mut world = PathBuf::from("world");
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--world" => {
                index += 1;
                world = arguments
                    .get(index)
                    .map(PathBuf::from)
                    .ok_or_else(|| sayo::Error::Usage("--world needs a path".into()))?;
            }
            "-h" | "--help" => {
                println!("Usage: sayo check [--world PATH]");
                return Ok(());
            }
            argument => {
                return Err(sayo::Error::Usage(format!(
                    "unknown check option: {argument}"
                )));
            }
        }
        index += 1;
    }
    let report = check::effective_world(world)?;
    println!(
        "World is valid: {} Space(s), {} internal Template(s), {} declared Relation(s), {} Sayo implementation(s); SayoVM Native Relations are available separately",
        report.spaces, report.templates, report.relations, report.sayo_implementations,
    );
    Ok(())
}

fn print_help() {
    println!("sayo 0.1.0");
    println!("Usage: sayo [--world PATH] [OPTIONS] EXPRESSION...");
    println!("       OPTIONS: --show-space --show-kind --trace-format json");
    println!("       sayo [--world PATH] FILE.sayo");
    println!("       sayo check [--world PATH]");
}
