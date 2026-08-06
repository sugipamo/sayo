use wasm_bindgen::prelude::*;

/// Evaluate one complete, versioned JSON request inside the browser process.
/// The returned string is always one JSON response document.
#[wasm_bindgen(js_name = evaluateJson)]
pub fn evaluate_json(request: &str) -> String {
    crate::web_api::evaluate_web_json(request)
}
