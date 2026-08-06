use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::runtime::{Runtime, RuntimeLimits};
use crate::trace::SayoTrace;
use crate::visualization::SviDocument;
use crate::world::{MemoryWorld, WorldPath, WorldView};

pub const WEB_API_VERSION: u32 = 1;

fn default_visualization() -> bool {
    true
}

fn default_relation_depth() -> usize {
    256
}

fn default_max_steps() -> usize {
    100_000
}

fn default_max_trace_events() -> usize {
    100_000
}

/// Bootstrap resource limits supplied by the browser host. They constrain an
/// execution but do not change Sayo's language semantics.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WebEvaluationOptions {
    pub visualization: bool,
    pub relation_depth: usize,
    pub max_steps: usize,
    pub max_trace_events: usize,
}

impl Default for WebEvaluationOptions {
    fn default() -> Self {
        Self {
            visualization: default_visualization(),
            relation_depth: default_relation_depth(),
            max_steps: default_max_steps(),
            max_trace_events: default_max_trace_events(),
        }
    }
}

/// Versioned, single-document input accepted at the Wasm boundary.
///
/// `directories` is separate from `files` because an empty Relation directory
/// is semantically meaningful and cannot be reconstructed from file names.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebEvaluationRequest {
    pub version: u32,
    pub entry: String,
    #[serde(default)]
    pub directories: Vec<String>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub options: WebEvaluationOptions,
}

pub struct ValidatedWebRequest {
    pub entry: WorldPath,
    pub world: MemoryWorld,
    pub options: WebEvaluationOptions,
}

impl WebEvaluationRequest {
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json)
            .map_err(|error| Error::Usage(format!("invalid Sayo Wasm request JSON: {error}")))
    }

    pub fn validate(self) -> Result<ValidatedWebRequest> {
        if self.version != WEB_API_VERSION {
            return Err(Error::Usage(format!(
                "unsupported Sayo Wasm request version {}, expected {WEB_API_VERSION}",
                self.version
            )));
        }

        let mut world = MemoryWorld::new();
        for directory in self.directories {
            world.create_dir(directory)?;
        }
        for (path, contents) in self.files {
            world.insert_file(path, contents)?;
        }

        let entry = WorldPath::new(&self.entry)?;
        if !world.is_file(&entry)? {
            return Err(Error::Source(format!(
                "Wasm request entry is not a file: {}",
                entry.as_str()
            )));
        }

        Ok(ValidatedWebRequest {
            entry,
            world,
            options: self.options,
        })
    }
}

#[derive(Serialize)]
struct WebLimit {
    kind: &'static str,
    value: usize,
    message: String,
}

#[derive(Serialize)]
struct WebFailure {
    category: &'static str,
    message: String,
}

#[derive(Serialize)]
struct WebEvaluationResponse {
    format: &'static str,
    version: u32,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    normal_form: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trap: Option<WebFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<WebLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visualization: Option<SviDocument>,
}

/// Execute one versioned browser request and always return one JSON document.
/// This function contains no JavaScript dependency; the Wasm export is a thin
/// adapter over this host-neutral boundary.
pub fn evaluate_web_json(json: &str) -> String {
    let response =
        match WebEvaluationRequest::from_json(json).and_then(WebEvaluationRequest::validate) {
            Ok(request) => evaluate_validated_request(request),
            Err(error) => WebEvaluationResponse {
                format: "sayo-web-evaluation",
                version: WEB_API_VERSION,
                status: "trap",
                normal_form: None,
                trap: Some(WebFailure {
                    category: error.category(),
                    message: error.to_string(),
                }),
                limit: None,
                visualization: None,
            },
        };
    serde_json::to_string(&response).expect("web response contains only serializable values")
}

fn evaluate_validated_request(request: ValidatedWebRequest) -> WebEvaluationResponse {
    let options = request.options;
    let limits = RuntimeLimits {
        relation_depth: options.relation_depth,
        reduction_steps: options.max_steps,
    };
    let runtime = match Runtime::from_memory_world(request.world, limits) {
        Ok(runtime) => runtime,
        Err(error) => {
            return WebEvaluationResponse {
                format: "sayo-web-evaluation",
                version: WEB_API_VERSION,
                status: "trap",
                normal_form: None,
                trap: Some(WebFailure {
                    category: error.category(),
                    message: error.to_string(),
                }),
                limit: None,
                visualization: None,
            };
        }
    };

    if !options.visualization {
        return match runtime.evaluate_world_file(&request.entry) {
            Ok(normal_form) => WebEvaluationResponse {
                format: "sayo-web-evaluation",
                version: WEB_API_VERSION,
                status: "normal",
                normal_form: Some(
                    runtime
                        .display_normal_form(&normal_form)
                        .expect("normal form rendering is infallible"),
                ),
                trap: None,
                limit: None,
                visualization: None,
            },
            Err(error) => response_for_error(error, &options, None),
        };
    }

    let evaluated =
        runtime.evaluate_world_file_with_trace(&request.entry, options.max_trace_events);
    let trace = evaluated.trace;
    match evaluated.result {
        Ok(normal_form) => {
            let result_terms = normal_form
                .terms()
                .iter()
                .map(|term| term.display.clone())
                .collect();
            WebEvaluationResponse {
                format: "sayo-web-evaluation",
                version: WEB_API_VERSION,
                status: "normal",
                normal_form: Some(
                    runtime
                        .display_normal_form(&normal_form)
                        .expect("normal form rendering is infallible"),
                ),
                trap: None,
                limit: None,
                visualization: Some(SviDocument::build(&trace, "normal", result_terms)),
            }
        }
        Err(error) => response_for_error(error, &options, Some(trace)),
    }
}

fn response_for_error(
    error: Error,
    options: &WebEvaluationOptions,
    trace: Option<SayoTrace>,
) -> WebEvaluationResponse {
    if matches!(error, Error::BootstrapLimit(_)) {
        let message = error.to_string();
        let (kind, value) = if message.contains("trace event") {
            ("trace_events", options.max_trace_events)
        } else if message.contains("depth") {
            ("relation_depth", options.relation_depth)
        } else {
            ("steps", options.max_steps)
        };
        WebEvaluationResponse {
            format: "sayo-web-evaluation",
            version: WEB_API_VERSION,
            status: "limit",
            normal_form: None,
            trap: None,
            limit: Some(WebLimit {
                kind,
                value,
                message,
            }),
            visualization: trace
                .as_ref()
                .map(|trace| SviDocument::build(trace, "limit", Vec::new())),
        }
    } else {
        WebEvaluationResponse {
            format: "sayo-web-evaluation",
            version: WEB_API_VERSION,
            status: "trap",
            normal_form: None,
            trap: Some(WebFailure {
                category: error.category(),
                message: error.to_string(),
            }),
            limit: None,
            visualization: trace
                .as_ref()
                .map(|trace| SviDocument::build(trace, "trap", Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_files_and_semantic_empty_directories() {
        let request = WebEvaluationRequest::from_json(
            r#"{
                "version": 1,
                "entry": "main.sayo",
                "directories": ["Add/Int/Plus"],
                "files": {"main.sayo": "Add 5 3"}
            }"#,
        )
        .unwrap()
        .validate()
        .unwrap();

        assert_eq!(request.entry, WorldPath::new("main.sayo").unwrap());
        assert!(
            request
                .world
                .is_dir(&WorldPath::new("Add/Int/Plus").unwrap())
                .unwrap()
        );
        assert_eq!(request.options, WebEvaluationOptions::default());
    }

    #[test]
    fn rejects_an_unknown_protocol_version() {
        let error = WebEvaluationRequest::from_json(
            r#"{"version":0,"entry":"main.sayo","files":{"main.sayo":"5"}}"#,
        )
        .unwrap()
        .validate()
        .err()
        .expect("version must fail");
        assert!(matches!(error, Error::Usage(message) if message.contains("version 0")));
    }

    #[test]
    fn rejects_a_missing_entry_file() {
        let error =
            WebEvaluationRequest::from_json(r#"{"version":1,"entry":"main.sayo","files":{}}"#)
                .unwrap()
                .validate()
                .err()
                .expect("entry must fail");
        assert!(matches!(error, Error::Source(message) if message.contains("main.sayo")));
    }

    #[test]
    fn evaluates_a_memory_world_and_embeds_svi_in_one_response() {
        let response = evaluate_web_json(
            r#"{
                "version": 1,
                "entry": "main.sayo",
                "files": {"main.sayo": "add 5 3"}
            }"#,
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "normal");
        assert_eq!(response["normal_form"], "8");
        assert_eq!(response["visualization"]["format"], "sayo-visualization-ir");
        assert_eq!(response["visualization"]["version"], 1);
        assert!(
            response["visualization"]["frames"]
                .as_array()
                .unwrap()
                .len()
                > 1
        );
        assert_eq!(
            response["visualization"]["frames"][0]["scene"]["rows"][0]["items"][0]["term"]["source"],
            "add"
        );
    }

    #[test]
    fn evaluates_a_structural_capture_encoded_as_an_empty_directory() {
        let response = evaluate_web_json(
            r#"{
                "version": 1,
                "entry": "main.sayo",
                "directories": ["Box/Int/Boxed", "Boxed"],
                "files": {
                    "main.sayo": "box 5",
                    "Box/space.sayo": "equal-to \"box\" %0"
                }
            }"#,
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "normal", "{response}");
        assert_eq!(response["normal_form"], "Boxed{5}");
    }

    #[test]
    fn svi_controls_are_complete_and_share_runtime_identity_across_slots() {
        let response = evaluate_web_json(
            r#"{
                "version": 1,
                "entry": "main.sayo",
                "files": {"main.sayo": "for (add 9) 1"}
            }"#,
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        let frames = response["visualization"]["frames"].as_array().unwrap();
        assert!(frames.iter().all(|frame| frame["scene"]["rows"].is_array()));

        let started = frames
            .iter()
            .find(|frame| frame["cause"]["kind"] == "control_started")
            .unwrap();
        let control_id = started["cause"]["control_id"].as_u64().unwrap();
        let slot_started = frames
            .iter()
            .find(|frame| {
                frame["cause"]["kind"] == "control_slot_started"
                    && frame["cause"]["control_id"] == control_id
            })
            .unwrap();
        assert_eq!(slot_started["cause"]["slot"], 1);
        assert!(frames.iter().any(|frame| {
            frame["cause"]["kind"] == "control_completed"
                && frame["cause"]["control_id"] == control_id
        }));
        let completed = frames
            .iter()
            .find(|frame| {
                frame["cause"]["kind"] == "control_completed"
                    && frame["cause"]["control_id"] == control_id
            })
            .unwrap();
        assert_eq!(completed["scene"]["rows"].as_array().unwrap().len(), 3);
        assert_eq!(
            completed["scene"]["rows"][2]["items"][0]["term"]["source"],
            "Plus{9}"
        );
        assert_eq!(
            completed["scene"]["rows"][2]["items"][1]["term"]["source"],
            "10"
        );
        let assembled = frames
            .iter()
            .find(|frame| {
                frame["cause"]["kind"] == "control_assembled"
                    && frame["cause"]["control_id"] == control_id
            })
            .unwrap();
        assert_eq!(assembled["scene"]["rows"].as_array().unwrap().len(), 2);
        assert_eq!(
            assembled["scene"]["rows"][1]["items"][0]["term"]["source"],
            "EachEmit{Plus{9}, 10}"
        );
        let assembled_changes = assembled["changes"].as_array().unwrap();
        assert!(
            assembled_changes
                .iter()
                .filter(|change| {
                    change["kind"] == "preserve"
                        && change["from"][0]
                            .as_str()
                            .is_some_and(|view| view.contains("control-overview"))
                        && change["to"][0]
                            .as_str()
                            .is_some_and(|view| view.contains("control-overview"))
                })
                .count()
                >= 2
        );
        assert!(frames.iter().any(|frame| {
            frame["cause"]["kind"] == "control_returned"
                && frame["cause"]["control_id"] == control_id
        }));
        assert!(
            frames
                .iter()
                .skip(1)
                .all(|frame| !frame["changes"].as_array().unwrap().is_empty())
        );

        let exposed = frames
            .iter()
            .find(|frame| {
                frame["cause"]["kind"] == "control_payload_exposed"
                    && frame["cause"]["control_id"] == control_id
            })
            .unwrap();
        let rows = exposed["scene"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1]["role"], "template");
        assert_eq!(rows[1]["separator"], "space");
        assert_eq!(rows[1]["items"][0]["term"]["kind"], "state");
        assert!(rows[1]["items"][0].get("domain").is_none());
        let reduction = rows.last().unwrap();
        assert_eq!(reduction["separator"], "comma");
        assert_eq!(reduction["items"].as_array().unwrap().len(), 2);
        assert_eq!(reduction["items"][0]["term"]["kind"], "placeholder");
        assert_eq!(reduction["items"][1]["term"]["kind"], "group");

        let exposed_index = frames
            .iter()
            .position(|frame| {
                frame["cause"]["kind"] == "control_payload_exposed"
                    && frame["cause"]["control_id"] == control_id
            })
            .unwrap();
        let next_changes = frames[exposed_index + 1]["changes"].as_array().unwrap();
        assert!(next_changes.iter().any(|change| {
            change["kind"] == "preserve"
                && change["from"].as_array().is_some_and(|from| {
                    from.iter()
                        .any(|view| view.as_str().is_some_and(|view| view.contains("-control-")))
                })
        }));
    }

    #[test]
    fn svi_omits_reducer_events_that_do_not_change_the_visible_scene() {
        let response = evaluate_web_json(
            r#"{
                "version": 1,
                "entry": "main.sayo",
                "files": {"main.sayo": "1 2 3 4 5"}
            }"#,
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        let frames = response["visualization"]["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["cause"]["kind"], "evaluation_started");
        assert_eq!(
            frames[0]["scene"]["rows"][0]["items"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }

    #[test]
    fn svi_marks_duplicate_term_identity_as_copy() {
        let response = evaluate_web_json(
            r#"{
                "version": 1,
                "entry": "main.sayo",
                "files": {"main.sayo": "duplicate 5"}
            }"#,
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        let frames = response["visualization"]["frames"].as_array().unwrap();
        assert!(
            frames.iter().any(|frame| frame["changes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|change| change["kind"] == "copy"
                    && change["to"].as_array().is_some_and(|to| !to.is_empty()))),
            "{response}"
        );
    }

    #[test]
    fn reports_trace_overflow_as_a_limit_not_a_trap() {
        let response = evaluate_web_json(
            r#"{
                "version": 1,
                "entry": "main.sayo",
                "files": {"main.sayo": "5"},
                "options": {"max_trace_events": 0}
            }"#,
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "limit");
        assert_eq!(response["limit"]["kind"], "trace_events");
        assert_eq!(response["limit"]["value"], 0);
        assert!(response.get("trap").is_none());
    }

    #[test]
    fn resolves_referenced_sayo_files_inside_the_memory_snapshot() {
        let response = evaluate_web_json(
            r#"{
                "version": 1,
                "entry": "program/main.sayo",
                "files": {
                    "program/main.sayo": "body.sayo",
                    "program/body.sayo": "add 5 3"
                }
            }"#,
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "normal", "{response}");
        assert_eq!(response["normal_form"], "8");
    }
}
