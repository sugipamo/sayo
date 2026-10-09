//! Explicit, synchronous native Host effects. No evaluator callback is installed.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::model::Term;
use crate::{Error, ExternalKind, ExternalReference, MemoryWorld, NormalForm, Result, Runtime};

static NEXT_HOST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostLimits {
    pub max_blob_bytes: usize,
    pub max_storage_bytes: usize,
    pub max_requests: usize,
    pub max_intervals: usize,
    pub http_timeout_ms: u64,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 1_048_576,
            max_storage_bytes: 33_554_432,
            max_requests: 128,
            max_intervals: 32,
            http_timeout_ms: 5_000,
        }
    }
}

enum Service {
    ReadFile(PathBuf),
    WriteNewFile(PathBuf),
    Http {
        client: reqwest::blocking::Client,
        url: reqwest::Url,
        method: reqwest::Method,
    },
    Clock,
    Random,
    Notify(Box<dyn Write + Send>),
}

/// A fixed observation. Reusing it in Sayo performs no external operation.
#[derive(Debug, Clone)]
pub struct EffectObservation {
    pub request: Term,
    pub response: NormalForm,
    pub output: ExternalReference,
    pub succeeded: bool,
}

#[derive(Debug)]
pub struct HostRun {
    pub normal_form: NormalForm,
    pub observations: Vec<EffectObservation>,
}

struct CachedObservation {
    request: Term,
    outcome: std::result::Result<EffectObservation, String>,
}

/// Handles grant only the operation configured by the embedding Host. Input
/// blobs cannot choose file paths, URLs, methods, output sinks, or new handles.
pub struct EffectHost {
    limits: HostLimits,
    host_id: u64,
    next_output: u64,
    blobs: BTreeMap<String, Vec<u8>>,
    stored_bytes: usize,
    services: BTreeMap<String, Service>,
    observations: BTreeMap<(u64, u64), CachedObservation>,
}

impl EffectHost {
    pub fn new(limits: HostLimits) -> Result<Self> {
        if limits.max_blob_bytes < 1024
            || limits.max_storage_bytes < limits.max_blob_bytes
            || limits.max_requests == 0
            || limits.max_intervals == 0
            || limits.http_timeout_ms == 0
        {
            return Err(Error::Usage("invalid Host limits".into()));
        }
        Ok(Self {
            limits,
            host_id: NEXT_HOST_ID.fetch_add(1, Ordering::Relaxed),
            next_output: 1,
            blobs: BTreeMap::new(),
            stored_bytes: 0,
            services: BTreeMap::new(),
            observations: BTreeMap::new(),
        })
    }

    /// Insert once. Blob contents never change or get silently replaced.
    pub fn insert_blob(
        &mut self,
        id: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<ExternalReference> {
        let reference = ExternalReference::new(ExternalKind::Blob, id)?;
        if self.blobs.contains_key(reference.id()) {
            return Err(Error::Usage("blob ID is already registered".into()));
        }
        if bytes.len() > self.limits.max_blob_bytes
            || bytes.len()
                > self
                    .limits
                    .max_storage_bytes
                    .saturating_sub(self.stored_bytes)
        {
            return Err(Error::Usage("Host blob storage limit exceeded".into()));
        }
        self.stored_bytes += bytes.len();
        self.blobs.insert(reference.id().into(), bytes);
        Ok(reference)
    }

    pub fn blob(&self, reference: &ExternalReference) -> Option<&[u8]> {
        (reference.kind() == ExternalKind::Blob)
            .then(|| self.blobs.get(reference.id()).map(Vec::as_slice))
            .flatten()
    }

    /// Inspect completed observations, including earlier operations if a later
    /// request or pure interval caused `run` to return an error.
    pub fn observations(&self) -> impl Iterator<Item = &EffectObservation> {
        self.observations
            .values()
            .filter_map(|record| record.outcome.as_ref().ok())
    }

    pub fn register_file_read(&mut self, id: &str, path: impl Into<PathBuf>) -> Result<()> {
        self.register(id, Service::ReadFile(path.into()))
    }

    /// Creation happens only on execution. Existing files are never overwritten.
    pub fn register_file_write(&mut self, id: &str, path: impl Into<PathBuf>) -> Result<()> {
        self.register(id, Service::WriteNewFile(path.into()))
    }

    /// One fixed endpoint and method; redirects are not followed. Proxy and CA
    /// configuration are inherited through reqwest/native TLS.
    pub fn register_http(&mut self, id: &str, method: &str, url: &str) -> Result<()> {
        self.register_http_with_headers(id, method, url, BTreeMap::new())
    }

    /// Headers are configured by the Host, never supplied by a Sayo blob.
    pub fn register_http_with_headers(
        &mut self,
        id: &str,
        method: &str,
        url: &str,
        headers: BTreeMap<String, String>,
    ) -> Result<()> {
        let url = reqwest::Url::parse(url).map_err(|_| Error::Usage("invalid HTTP URL".into()))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Usage(
                "HTTP endpoints need http(s) and no embedded credentials".into(),
            ));
        }
        let method = match method {
            "GET" => reqwest::Method::GET,
            "HEAD" => reqwest::Method::HEAD,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "PATCH" => reqwest::Method::PATCH,
            "DELETE" => reqwest::Method::DELETE,
            _ => return Err(Error::Usage("unsupported HTTP method".into())),
        };
        let mut defaults = reqwest::header::HeaderMap::new();
        for (name, value) in headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| Error::Usage("invalid HTTP header name".into()))?;
            let mut value = reqwest::header::HeaderValue::from_str(&value)
                .map_err(|_| Error::Usage("invalid HTTP header value".into()))?;
            value.set_sensitive(true);
            defaults.insert(name, value);
        }
        let client = reqwest::blocking::Client::builder()
            .default_headers(defaults)
            .timeout(Duration::from_millis(self.limits.http_timeout_ms))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Usage("could not initialize HTTP client".into()))?;
        self.register(
            id,
            Service::Http {
                client,
                url,
                method,
            },
        )
    }

    pub fn register_clock(&mut self, id: &str) -> Result<()> {
        self.register(id, Service::Clock)
    }

    pub fn register_random(&mut self, id: &str) -> Result<()> {
        self.register(id, Service::Random)
    }

    /// Notification means bytes written to this explicit Host-owned sink.
    /// No email, messaging account, or remote recipient is inferred.
    pub fn register_notification(
        &mut self,
        id: &str,
        sink: impl Write + Send + 'static,
    ) -> Result<()> {
        self.register(id, Service::Notify(Box::new(sink)))
    }

    fn register(&mut self, id: &str, service: Service) -> Result<()> {
        ExternalReference::new(ExternalKind::Handle, id)?;
        if self.services.contains_key(id) {
            return Err(Error::Usage("service handle is already registered".into()));
        }
        self.services.insert(id.into(), service);
        Ok(())
    }

    /// Dispatch one Request occurrence explicitly. Its identity is consumed even
    /// if response construction fails after an effect. Repeats return the cached
    /// observation/error, without retrying the operation.
    pub fn execute(&mut self, runtime: &Runtime, request: &Term) -> Result<EffectObservation> {
        let payload = runtime.payload_terms(request)?;
        if request.space != "Request" {
            return Err(Error::Usage("Host expects a Request".into()));
        }
        let key = (
            request.runtime_id,
            request.term_id.expect("validated Runtime Term"),
        );
        if let Some(cached) = self.observations.get(&key) {
            if cached.request != *request {
                return Err(Error::Usage("request identity changed".into()));
            }
            return cached.outcome.clone().map_err(Error::Usage);
        }
        if self.observations.len() >= self.limits.max_requests {
            return Err(Error::Usage("Host request limit exceeded".into()));
        }
        // Reserve a whole bounded result before touching an external service.
        if self
            .limits
            .max_storage_bytes
            .saturating_sub(self.stored_bytes)
            < self.limits.max_blob_bytes
        {
            return Err(Error::Usage(
                "insufficient Host storage for an observation".into(),
            ));
        }
        let id = loop {
            let id = format!("host-{}-result-{}", self.host_id, self.next_output);
            self.next_output += 1;
            if !self.blobs.contains_key(&id) {
                break id;
            }
        };
        self.observations.insert(
            key,
            CachedObservation {
                request: request.clone(),
                outcome: Err("request was already consumed without an observation".into()),
            },
        );
        let result = self.execute_uncached(runtime, request, payload, &id);
        self.observations
            .get_mut(&key)
            .expect("recorded request")
            .outcome = result.as_ref().cloned().map_err(ToString::to_string);
        result
    }

    fn execute_uncached(
        &mut self,
        runtime: &Runtime,
        request: &Term,
        payload: Option<Vec<Term>>,
        id: &str,
    ) -> Result<EffectObservation> {
        // Validate both response paths before any effect, even if failure is not
        // expected. These placeholders are never dispatched by the evaluator.
        runtime.evaluate(&format!("Succeeded{{Data<blob:{id}>}}"))?;
        runtime.evaluate(&format!("Failed{{Data<blob:{id}>}}"))?;
        let outcome = self.dispatch(payload.as_deref());
        let succeeded = outcome.is_ok();
        let bytes = match outcome {
            Ok(bytes) => bytes,
            Err(failure) => serde_json::to_vec(&serde_json::json!({
                "code": failure.code,
                "message": failure.message.chars().take(128).collect::<String>(),
                "effects_may_have_occurred": failure.effects_may_have_occurred,
            }))
            .expect("failure JSON"),
        };
        let output = self.insert_blob(id, bytes)?;
        let role = if succeeded { "Succeeded" } else { "Failed" };
        let response = runtime.evaluate(&format!("{role}{{Data<{}>}}", output.render_source()))?;
        Ok(EffectObservation {
            request: request.clone(),
            response,
            output,
            succeeded,
        })
    }

    fn dispatch(&mut self, payload: Option<&[Term]>) -> std::result::Result<Vec<u8>, Failure> {
        let Some([service, data]) = payload else {
            return Err(Failure::new(
                "invalid_request",
                "Request needs Service and Data",
                false,
            ));
        };
        let service = external_reference(service, "Service", ExternalKind::Handle)?;
        let data = external_reference(data, "Data", ExternalKind::Blob)?;
        let input = self
            .blobs
            .get(data.id())
            .ok_or_else(|| Failure::new("missing_blob", "input blob is not registered", false))?;
        let service = self.services.get_mut(service.id()).ok_or_else(|| {
            Failure::new("unknown_handle", "service handle is not registered", false)
        })?;
        let max_bytes = self.limits.max_blob_bytes;
        match service {
            Service::ReadFile(path) => {
                empty_input(input)?;
                let metadata = std::fs::metadata(&*path).map_err(|e| Failure::io(e, true))?;
                if !metadata.is_file() {
                    return Err(Failure::new(
                        "not_a_file",
                        "only regular files can be read",
                        true,
                    ));
                }
                let file = std::fs::File::open(path).map_err(|e| Failure::io(e, true))?;
                read_bounded(file, max_bytes, true)
            }
            Service::WriteNewFile(path) => {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|e| Failure::io(e, false))?;
                file.write_all(input)
                    .and_then(|()| file.flush())
                    .map_err(|e| Failure::io(e, true))?;
                Ok(serde_json::to_vec(&serde_json::json!({"bytes_written": input.len()})).unwrap())
            }
            Service::Http {
                client,
                url,
                method,
            } => {
                if matches!(*method, reqwest::Method::GET | reqwest::Method::HEAD) {
                    empty_input(input)?;
                }
                let mut request = client.request(method.clone(), url.clone());
                if !input.is_empty() {
                    request = request.body(input.clone());
                }
                let response = request.send().map_err(|_| {
                    Failure::new("http_transport", "HTTP exchange failed or timed out", true)
                })?;
                if !response.status().is_success() {
                    return Err(Failure::new(
                        "http_status",
                        format!("HTTP status {}", response.status().as_u16()),
                        true,
                    ));
                }
                read_bounded(response, max_bytes, true)
            }
            Service::Clock => {
                empty_input(input)?;
                let elapsed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| Failure::new("clock", "system time precedes Unix epoch", false))?;
                let millis = u64::try_from(elapsed.as_millis()).map_err(|_| {
                    Failure::new("clock", "system time is outside supported range", false)
                })?;
                Ok(serde_json::to_vec(&serde_json::json!({"unix_ms": millis})).unwrap())
            }
            Service::Random => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Parameters {
                    bytes: usize,
                }
                let parameters: Parameters = serde_json::from_slice(input).map_err(|_| {
                    Failure::new(
                        "invalid_input",
                        "random input needs JSON {\"bytes\":N}",
                        false,
                    )
                })?;
                if parameters.bytes > max_bytes {
                    return Err(Failure::new(
                        "size_limit",
                        "random output exceeds blob limit",
                        false,
                    ));
                }
                let mut bytes = vec![0; parameters.bytes];
                getrandom::fill(&mut bytes)
                    .map_err(|_| Failure::new("random", "system random source failed", true))?;
                Ok(bytes)
            }
            Service::Notify(sink) => {
                sink.write_all(input)
                    .and_then(|()| sink.flush())
                    .map_err(|e| Failure::io(e, true))?;
                Ok(serde_json::to_vec(&serde_json::json!({"bytes_written": input.len()})).unwrap())
            }
        }
    }

    /// Sequential Host policy: process top-level Requests left-to-right, replace
    /// each occurrence in place, then normalize a new pure evaluation interval.
    /// Requests nested inside payloads are not automatically discovered.
    pub fn run(&mut self, runtime: &Runtime, pending: NormalForm) -> Result<HostRun> {
        let mut normal_form = runtime.evaluate_terms(pending.into_terms())?;
        let mut observations = Vec::new();
        for _ in 0..self.limits.max_intervals {
            if !normal_form
                .terms()
                .iter()
                .any(|term| term.space == "Request")
            {
                return Ok(HostRun {
                    normal_form,
                    observations,
                });
            }
            let mut next = Vec::new();
            for term in normal_form.into_terms() {
                if term.space == "Request" {
                    let observation = self.execute(runtime, &term)?;
                    next.extend(observation.response.terms().iter().cloned());
                    observations.push(observation);
                } else {
                    next.push(term);
                }
            }
            normal_form = runtime.evaluate_terms(next)?;
        }
        if !normal_form
            .terms()
            .iter()
            .any(|term| term.space == "Request")
        {
            return Ok(HostRun {
                normal_form,
                observations,
            });
        }
        Err(Error::Usage(
            "Host evaluation interval limit exceeded".into(),
        ))
    }
}

/// An ordinary user World for the Host envelope; it adds no Native semantics.
pub fn effect_world() -> Result<MemoryWorld> {
    let mut world = MemoryWorld::new();
    for (path, source) in [
        ("Service/space.sayo", "equal-to \"service\" %0"),
        ("Data/space.sayo", "equal-to \"data\" %0"),
        ("Send/space.sayo", "equal-to \"send\" %0"),
        ("Succeed/space.sayo", "equal-to \"succeed\" %0"),
        ("Fail/space.sayo", "equal-to \"fail\" %0"),
        (
            "Request/template.sayo",
            "CheckSpace{%0, Service} CheckSpace{%1, Data}",
        ),
        ("Succeeded/template.sayo", "CheckSpace{%0, Data}"),
        ("Failed/template.sayo", "CheckSpace{%0, Data}"),
    ] {
        world.insert_file(path, source)?;
    }
    for path in [
        "Send/Service/WithService",
        "WithService/Data/Request",
        "Succeed/Data/Succeeded",
        "Fail/Data/Failed",
    ] {
        world.create_dir(path)?;
    }
    Ok(world)
}

struct Failure {
    code: &'static str,
    message: String,
    effects_may_have_occurred: bool,
}

impl Failure {
    fn new(
        code: &'static str,
        message: impl Into<String>,
        effects_may_have_occurred: bool,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            effects_may_have_occurred,
        }
    }
    fn io(error: std::io::Error, effects: bool) -> Self {
        Self::new("io", error.to_string(), effects)
    }
}

fn external_reference<'a>(
    term: &'a Term,
    role: &str,
    kind: ExternalKind,
) -> std::result::Result<&'a ExternalReference, Failure> {
    term.external
        .as_deref()
        .filter(|reference| term.space == role && reference.kind() == kind)
        .ok_or_else(|| {
            Failure::new(
                "invalid_request",
                "unexpected external role or reference kind",
                false,
            )
        })
}

fn empty_input(input: &[u8]) -> std::result::Result<(), Failure> {
    if input.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(
            "invalid_input",
            "operation requires an empty input blob",
            false,
        ))
    }
}

fn read_bounded(
    reader: impl Read,
    limit: usize,
    effects: bool,
) -> std::result::Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    reader
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| Failure::io(e, effects))?;
    if bytes.len() > limit {
        return Err(Failure::new(
            "size_limit",
            "output exceeds blob limit",
            effects,
        ));
    }
    Ok(bytes)
}
