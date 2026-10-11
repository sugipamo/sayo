//! Opt-in native Host executable. The ordinary `sayo` CLI stays pure.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sayo::host::{EffectHost, HostLimits, effect_world};
use sayo::{Error, Result, Runtime, RuntimeLimits};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    #[serde(default)]
    limits: HostLimits,
    #[serde(default)]
    blobs: BTreeMap<String, String>,
    #[serde(default)]
    blob_files: BTreeMap<String, PathBuf>,
    #[serde(default)]
    services: BTreeMap<String, ServiceConfiguration>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum ServiceConfiguration {
    ReadFile {
        path: PathBuf,
    },
    WriteFile {
        path: PathBuf,
    },
    Http {
        url: String,
        #[serde(default = "get_method")]
        method: String,
        #[serde(default)]
        headers: BTreeMap<String, HeaderConfiguration>,
        #[serde(default)]
        response_metadata: bool,
    },
    Clock,
    Random,
    Notify,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum HeaderConfiguration {
    Literal(String),
    Environment {
        env: String,
        #[serde(default)]
        prefix: String,
    },
}

fn get_method() -> String {
    "GET".into()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sayo-host: {}: {error}", error.category());
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut config = None;
    let mut world = None;
    let mut export = None;
    let mut program = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "Usage: sayo-host --config HOST.json [--world PATH] [--export-blobs PATH] FILE.sayo"
                );
                println!(
                    "Without --world, use the built-in user World for Request, Service, Data, Succeeded, Failed, and HTTP metadata."
                );
                return Ok(());
            }
            "--config" => config = Some(value(&mut args, "--config")?),
            "--world" => world = Some(value(&mut args, "--world")?),
            "--export-blobs" => export = Some(value(&mut args, "--export-blobs")?),
            _ if argument.starts_with('-') => {
                return Err(Error::Usage("unknown Host option".into()));
            }
            _ if program.is_none() => program = Some(PathBuf::from(argument)),
            _ => return Err(Error::Usage("only one program file is accepted".into())),
        }
    }
    let config = config.ok_or_else(|| Error::Usage("--config is required".into()))?;
    let program = program.ok_or_else(|| Error::Usage("program file is required".into()))?;
    let mut bytes = Vec::new();
    std::fs::File::open(&config)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err(Error::Usage("Host configuration exceeds size limit".into()));
    }
    let configuration: Configuration = serde_json::from_slice(&bytes)
        .map_err(|e| Error::Usage(format!("invalid Host configuration: {e}")))?;
    let base = config.parent().unwrap_or_else(|| Path::new("."));
    let blob_limit = configuration.limits.max_blob_bytes;
    let mut host = EffectHost::new(configuration.limits)?;
    for (id, value) in configuration.blobs {
        host.insert_blob(id, value.into_bytes())?;
    }
    for (id, path) in configuration.blob_files {
        let mut bytes = Vec::new();
        std::fs::File::open(base.join(path))?
            .take((blob_limit as u64).saturating_add(1))
            .read_to_end(&mut bytes)?;
        host.insert_blob(id, bytes)?;
    }
    for (id, service) in configuration.services {
        match service {
            ServiceConfiguration::ReadFile { path } => {
                host.register_file_read(&id, base.join(path))?
            }
            ServiceConfiguration::WriteFile { path } => {
                host.register_file_write(&id, base.join(path))?
            }
            ServiceConfiguration::Http {
                method,
                url,
                headers,
                response_metadata,
            } => {
                let mut resolved = BTreeMap::new();
                for (name, value) in headers {
                    let value = match value {
                        HeaderConfiguration::Literal(value) => value,
                        HeaderConfiguration::Environment { env, prefix } => {
                            let value = std::env::var(&env).map_err(|_| {
                                Error::Usage("HTTP header environment variable is missing".into())
                            })?;
                            format!("{prefix}{value}")
                        }
                    };
                    resolved.insert(name, value);
                }
                if response_metadata {
                    host.register_http_response_with_headers(&id, &method, &url, resolved)?;
                } else {
                    host.register_http_with_headers(&id, &method, &url, resolved)?;
                }
            }
            ServiceConfiguration::Clock => host.register_clock(&id)?,
            ServiceConfiguration::Random => host.register_random(&id)?,
            ServiceConfiguration::Notify => host.register_notification(&id, std::io::stderr())?,
        }
    }
    let runtime = match world {
        Some(path) => Runtime::new(path)?,
        None => Runtime::from_memory_world(effect_world()?, RuntimeLimits::default())?,
    };
    let pending = runtime.evaluate_file(program)?;
    let result = host.run(&runtime, pending)?;
    println!("{}", runtime.display_normal_form(&result.normal_form)?);
    eprintln!(
        "host: {} observation(s), {} failed",
        result.observations.len(),
        result
            .observations
            .iter()
            .filter(|observation| !observation.succeeded)
            .count()
    );
    if let Some(path) = export {
        std::fs::create_dir_all(&path)?;
        let mut exported = BTreeMap::new();
        for observation in &result.observations {
            if exported.insert(observation.output.id(), ()).is_none() {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path.join(format!("{}.blob", observation.output.id())))?;
                std::io::Write::write_all(
                    &mut file,
                    host.blob(&observation.output).expect("observation blob"),
                )?;
            }
        }
    }
    Ok(())
}

fn value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| Error::Usage(format!("{option} needs a path")))
}
