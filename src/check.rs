use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::parser::parse;
use crate::runtime::Runtime;

const BUILTIN_SPACES: &[&str] = &["Keep", "Drop"];

#[derive(Debug, Default)]
pub struct Report {
    pub spaces: usize,
    pub templates: usize,
    pub relations: usize,
    pub implementations: usize,
    pub sayo_implementations: usize,
}

pub fn world(path: impl AsRef<Path>) -> Result<Report> {
    world_with_external_spaces(path, &[])
}

/// Validate both the user World itself and the effective World assembled with
/// the embedded standard World and Native fallbacks. The returned counts are
/// intentionally limited to declarations owned by the user World.
pub fn effective_world(path: impl AsRef<Path>) -> Result<Report> {
    let path = path.as_ref();
    let report = world(path)?;
    Runtime::new(path)?;
    Ok(report)
}

fn world_with_external_spaces(path: impl AsRef<Path>, external_spaces: &[&str]) -> Result<Report> {
    let requested = path.as_ref();
    let world = fs::canonicalize(requested).map_err(|error| {
        Error::World(format!(
            "cannot open World {}: {error}",
            requested.display()
        ))
    })?;
    if !world.is_dir() {
        return Err(Error::World(format!(
            "World is not a directory: {}",
            world.display()
        )));
    }

    let world_directories = directories(&world)?;
    if world_directories
        .iter()
        .any(|path| path.join("main.native").is_file())
    {
        return Err(Error::World(
            "main.native declarations are not allowed; Native Relations are provided by SayoVM"
                .into(),
        ));
    }
    for path in &world_directories {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let message = match name {
            "Trap" => Some("Template name `Trap` is reserved for the SayoVM semantic trap"),
            "Vanish" => Some("Template name `Vanish` is reserved for SayoVM state removal"),
            "Miss" => Some("Space name `Miss` is reserved for SayoVM Space Checker control"),
            _ => None,
        };
        if let Some(message) = message {
            return Err(Error::World(message.into()));
        }
    }
    let components = world_directories
        .iter()
        .filter(|space| !is_hidden(space))
        .cloned()
        .collect::<Vec<_>>();
    for component in &components {
        validate_name(
            component
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(""),
            "Space or Template",
            component,
        )?;
    }
    let component_names = components
        .iter()
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    let mut report = Report {
        spaces: components
            .iter()
            .filter(|component| {
                implementation_path(component).is_some() || component.join("space.sayo").is_file()
            })
            .count(),
        templates: components
            .iter()
            .filter(|component| {
                implementation_path(component).is_none() && !component.join("space.sayo").is_file()
            })
            .count(),
        ..Report::default()
    };

    for component in &components {
        if let Some(implementation) = implementation_path(component) {
            check_implementation(&implementation)?;
            report.implementations += 1;
            count_kind(&implementation, &mut report);
        }
        let space_checker = component.join("space.sayo");
        if space_checker.is_file() {
            check_implementation(&space_checker)?;
            report.implementations += 1;
            count_kind(&space_checker, &mut report);
        }
        let schema = component.join("template.sayo");
        if schema.is_file() {
            check_implementation(&schema)?;
            report.implementations += 1;
            count_kind(&schema, &mut report);
        }
        for right in directories(component)? {
            let right_name = right.file_name().map(|name| name.to_string_lossy());
            let right_name_text = right_name.as_deref().unwrap_or("");
            let reserved_slot = matches!(right_name_text, ".any" | ".tick" | ".end");
            if is_hidden(&right) && !reserved_slot {
                continue;
            }
            validate_name(right_name_text, "RightSpace or control slot", &right)?;
            if !reserved_slot
                && !component_names
                    .iter()
                    .any(|name| right_name.as_deref() == Some(name.as_str()))
                && !external_spaces.contains(&right_name_text)
                && !BUILTIN_SPACES.contains(&right_name_text)
            {
                return Err(Error::World(format!(
                    "Relation RightSpace is not a Space: {}",
                    right.display()
                )));
            }
            let results = directories(&right)?;
            let has_dynamic = results
                .iter()
                .any(|result| result.file_name().and_then(|name| name.to_str()) == Some(".any"));
            let has_fixed = results.iter().any(|result| {
                result
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with('.'))
            });
            if has_dynamic && has_fixed {
                return Err(Error::World(format!(
                    "fixed and dynamic Result Relations cannot coexist: {}",
                    right.display()
                )));
            }
            for result in results {
                let result_name = result.file_name().map(|name| name.to_string_lossy());
                let dynamic_result = result_name.as_deref() == Some(".any");
                if is_hidden(&result) && !dynamic_result {
                    continue;
                }
                if result.join("main.native").is_file() {
                    return Err(Error::World(
                        "main.native declarations are not allowed; Native Relations are provided by SayoVM"
                            .into(),
                    ));
                }
                validate_name(
                    result_name.as_deref().unwrap_or(""),
                    "Relation ResultSpace",
                    &result,
                )?;
                if !matches!(result_name.as_deref(), Some("Vanish" | "Trap" | ".any"))
                    && !component_names
                        .iter()
                        .any(|name| result_name.as_deref() == Some(name.as_str()))
                    && !result_name
                        .as_deref()
                        .is_some_and(|name| BUILTIN_SPACES.contains(&name))
                {
                    return Err(Error::World(format!(
                        "Relation ResultSpace is not a Space: {}",
                        result.display()
                    )));
                }
                if matches!(right_name_text, ".tick" | ".end")
                    && matches!(result_name.as_deref(), Some("Trap" | ".any"))
                    && !result.join("main.sayo").is_file()
                {
                    return Err(Error::World(format!(
                        "Template control Relation with Result Trap or .any requires main.sayo: {}",
                        result.display()
                    )));
                }
                report.relations += 1;
                if let Some(implementation) = implementation_path(&result) {
                    check_implementation(&implementation)?;
                    report.implementations += 1;
                    count_kind(&implementation, &mut report);
                } else if fs::read_dir(&result)
                    .map(|entries| entries.flatten().all(|entry| is_hidden(&entry.path())))
                    .unwrap_or(false)
                {
                    // An empty fixed-Result directory is a structural Capture
                    // Relation. Hidden metadata does not change that meaning.
                } else {
                    return Err(Error::World(format!(
                        "Capture Relation contains unsupported entries: {}",
                        result.display()
                    )));
                }
            }
        }
    }
    crate::arity::analyze(&[world.as_path()])?;
    Ok(report)
}

fn check_implementation(path: &Path) -> Result<()> {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("main.sayo" | "template.sayo" | "space.sayo") => {
            let source = fs::read_to_string(path)?;
            parse(&source).map(|_| ()).map_err(|error| {
                Error::Implementation(format!("invalid Sayo {}: {error}", path.display()))
            })
        }
        _ => Err(Error::World(format!(
            "unsupported implementation: {}",
            path.display()
        ))),
    }
}

fn count_kind(path: &Path, report: &mut Report) {
    if let Some("main.sayo" | "space.sayo") = path.file_name().and_then(|name| name.to_str()) {
        report.sayo_implementations += 1;
    }
}

fn implementation_path(path: &Path) -> Option<PathBuf> {
    let sayo = path.join("main.sayo");
    sayo.is_file().then_some(sayo)
}

fn validate_name(name: &str, kind: &str, path: &Path) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.chars().any(char::is_whitespace) {
        return Err(Error::World(format!(
            "invalid {kind} name {name:?}: {}",
            path.display()
        )));
    }
    Ok(())
}

fn directories(path: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|entry| entry.is_dir())
        .collect::<Vec<_>>();
    entries.sort();
    Ok(entries)
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') || name.starts_with("__"))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn validates_the_example_world() {
        let world_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/native_arithmetic/world");
        let report = world(&world_path).unwrap();

        assert_eq!(report.spaces, 0);
        assert_eq!(report.templates, 0);
        assert_eq!(report.relations, 0);
        assert_eq!(report.implementations, 0);
        assert_eq!(report.sayo_implementations, 0);
    }

    #[test]
    fn accepts_an_empty_user_world_overlay() {
        let world_path = temporary_world("empty-overlay");
        fs::create_dir_all(&world_path).unwrap();

        let report = effective_world(&world_path).unwrap();
        let _ = fs::remove_dir_all(&world_path);

        assert_eq!(report.spaces, 0);
        assert_eq!(report.templates, 0);
        assert_eq!(report.relations, 0);
        assert_eq!(report.implementations, 0);
    }

    #[test]
    fn validates_the_embedded_standard_world_source_tree() {
        let world_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("standard_world");
        // The embedded standard World is linked with the user's World at
        // runtime. `Int` is therefore an explicit external dependency here,
        // not a component duplicated into the standard World.
        let report = world_with_external_spaces(&world_path, &["Int"]).unwrap();

        assert_eq!(report.spaces, 0);
        assert_eq!(report.templates, 29);
        assert_eq!(report.relations, 35);
        assert_eq!(report.sayo_implementations, 22);
    }

    #[test]
    fn effective_check_rejects_a_user_shadow_incompatible_with_the_standard_world() {
        let world_path = temporary_world("incompatible-standard-shadow");
        fs::create_dir_all(world_path.join("Each")).unwrap();
        declare_space(&world_path, "Each");

        assert!(world(&world_path).is_ok());
        let result = effective_world(&world_path);
        let _ = fs::remove_dir_all(&world_path);
        assert!(matches!(
            result,
            Err(Error::World(message)) if message.contains("payload arity")
        ));
    }

    #[test]
    fn rejects_invalid_sayo() {
        let world_path = temporary_world("invalid-sayo");
        fs::create_dir_all(world_path.join("Int")).unwrap();
        fs::write(world_path.join("Int/main.sayo"), "(5\n").unwrap();

        let result = world(&world_path);
        let _ = fs::remove_dir_all(&world_path);
        assert!(matches!(result, Err(Error::Implementation(_))));
    }

    #[test]
    fn rejects_an_invalid_template_schema() {
        let world_path = temporary_world("invalid-template-schema");
        fs::create_dir_all(world_path.join("Box")).unwrap();
        fs::write(world_path.join("Box/template.sayo"), "(%0\n").unwrap();

        let result = world(&world_path);
        let _ = fs::remove_dir_all(&world_path);
        assert!(matches!(result, Err(Error::Implementation(_))));
    }

    #[test]
    fn rejects_native_relation_declarations_in_the_world() {
        let world_path = temporary_world("native-declaration");
        fs::create_dir_all(world_path.join("Int")).unwrap();
        fs::create_dir_all(world_path.join("Add/Int/Plus")).unwrap();
        fs::create_dir_all(world_path.join("Plus")).unwrap();
        declare_space(&world_path, "Int");
        declare_space(&world_path, "Add");
        declare_space(&world_path, "Plus");
        fs::write(world_path.join("Add/Int/Plus/main.native"), "add_int\n").unwrap();

        let result = world(&world_path);
        let _ = fs::remove_dir_all(&world_path);
        assert!(
            matches!(result, Err(Error::World(message)) if message.contains("main.native") && message.contains("not allowed"))
        );
    }

    #[test]
    fn accepts_an_empty_directory_as_a_capture_relation() {
        let world_path = temporary_world("capture-relation");
        fs::create_dir_all(world_path.join("Int")).unwrap();
        fs::create_dir_all(world_path.join("Add/Int/Plus")).unwrap();
        fs::create_dir_all(world_path.join("Plus")).unwrap();
        declare_space(&world_path, "Int");
        declare_space(&world_path, "Add");

        let report = world(&world_path).unwrap();
        let _ = fs::remove_dir_all(&world_path);
        assert_eq!(report.relations, 1);
        assert_eq!(report.implementations, 2);
    }

    #[test]
    fn accepts_an_implementationless_internal_template() {
        let world_path = temporary_world("internal-template");
        fs::create_dir_all(world_path.join("Int")).unwrap();
        fs::create_dir_all(world_path.join("Identity/Int/IdentityApply")).unwrap();
        fs::create_dir_all(world_path.join("IdentityApply/.tick/Int")).unwrap();
        declare_space(&world_path, "Int");
        declare_space(&world_path, "Identity");
        fs::write(world_path.join("IdentityApply/.tick/Int/main.sayo"), "%0\n").unwrap();

        let report = world(&world_path).unwrap();
        let _ = fs::remove_dir_all(&world_path);
        assert_eq!(report.spaces, 2);
        assert_eq!(report.templates, 1);
        assert_eq!(report.relations, 2);
        assert_eq!(report.implementations, 3);
        assert_eq!(report.sayo_implementations, 3);
    }

    #[test]
    fn accepts_an_empty_control_relation_as_a_zero_payload_transition() {
        let world_path = temporary_world("zero-payload-control");
        fs::create_dir_all(world_path.join("Box/.tick/Empty")).unwrap();
        fs::create_dir_all(world_path.join("Empty")).unwrap();
        declare_space(&world_path, "Box");

        let report = world(&world_path).unwrap();
        let _ = fs::remove_dir_all(&world_path);
        assert_eq!(report.relations, 1);
        assert_eq!(report.implementations, 1);
    }

    #[test]
    fn accepts_input_as_an_ordinary_space_name() {
        let world_path = temporary_world("ordinary-input");
        fs::create_dir_all(world_path.join("input")).unwrap();
        declare_space(&world_path, "input");

        let report = world(&world_path).unwrap();
        let _ = fs::remove_dir_all(&world_path);
        assert_eq!(report.spaces, 1);
    }

    #[test]
    fn accepts_keep_and_drop_as_builtin_result_spaces() {
        let world_path = temporary_world("builtin-filter-results");
        fs::create_dir_all(world_path.join("Int")).unwrap();
        fs::create_dir_all(world_path.join("Predicate/Int/Keep")).unwrap();
        declare_space(&world_path, "Int");
        declare_space(&world_path, "Predicate");
        fs::write(world_path.join("Predicate/Int/Keep/main.sayo"), "keep\n").unwrap();

        let report = world(&world_path).unwrap();
        let _ = fs::remove_dir_all(&world_path);
        assert_eq!(report.relations, 1);
        assert_eq!(report.sayo_implementations, 3);
    }

    #[test]
    fn rejects_fixed_and_dynamic_results_for_the_same_relation_input() {
        let world_path = temporary_world("fixed-dynamic-results");
        fs::create_dir_all(world_path.join("Int")).unwrap();
        fs::create_dir_all(world_path.join("Box/Int/Int")).unwrap();
        fs::create_dir_all(world_path.join("Box/Int/.any")).unwrap();
        declare_space(&world_path, "Int");
        fs::write(world_path.join("Box/Int/.any/main.sayo"), "%0\n").unwrap();

        let error = world(&world_path).unwrap_err();
        let _ = fs::remove_dir_all(&world_path);
        assert!(matches!(
            error,
            Error::World(message)
                if message.contains("fixed and dynamic Result Relations cannot coexist")
        ));
    }

    #[test]
    fn rejects_the_reserved_trap_template() {
        let world_path = temporary_world("reserved-trap");
        fs::create_dir_all(world_path.join("Trap")).unwrap();

        let result = world(&world_path);
        let _ = fs::remove_dir_all(&world_path);
        assert!(
            matches!(result, Err(Error::World(message)) if message.contains("Trap") && message.contains("reserved"))
        );
    }

    #[test]
    fn rejects_the_reserved_miss_space() {
        let world_path = temporary_world("reserved-miss");
        fs::create_dir_all(world_path.join("Miss")).unwrap();

        let result = world(&world_path);
        let _ = fs::remove_dir_all(&world_path);
        assert!(matches!(
            result,
            Err(Error::World(message)) if message.contains("Miss") && message.contains("reserved")
        ));
    }

    fn declare_space(world: &Path, name: &str) {
        fs::create_dir_all(world.join(name)).unwrap();
        fs::write(world.join(name).join("space.sayo"), "Miss\n").unwrap();
    }

    fn temporary_world(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sayo-check-{label}-{}-{nonce}", std::process::id()))
    }
}
