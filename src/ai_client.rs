// HTTP client for the Python AI microservice.
//
// Sends SMART data to the Python service running on 127.0.0.1:5001 and
// returns a typed AiResult.  All networking is synchronous and runs in a
// background thread so the egui UI thread is never blocked.
//
// If the service is unreachable, every function returns None gracefully —
// the rest of the app continues without AI features.

use crate::models::{AiResult, DiskInfo, SmartAttribute};

/// Base URL for the Python AI microservice.
const AI_BASE_URL: &str = "http://127.0.0.1:5001";

// ------------------------------------------------------------------ //
// SMART payload serialisation
// ------------------------------------------------------------------ //

/// Extract a SMART attribute raw value by name (case-insensitive substring).
fn find_attr_raw(attrs: &[SmartAttribute], name: &str) -> Option<i64> {
    let lower = name.to_lowercase();
    attrs
        .iter()
        .find(|a| a.name.to_lowercase().contains(&lower))
        .and_then(|a| a.raw_value.split_whitespace().next()?.parse::<i64>().ok())
}

/// Serialise the key SMART fields from DiskInfo into a compact JSON string.
/// Written without serde to keep the dependency list minimal.
fn serialise_smart(disk: &DiskInfo) -> String {
    let health  = disk.health_percent.map(|v| v.to_string()).unwrap_or("null".into());
    let temp    = disk.temp_c.map(|v| v.to_string()).unwrap_or("null".into());
    let hours   = disk.power_on_hours.map(|v| v.to_string()).unwrap_or("null".into());
    let unsafe_ = disk.unsafe_shutdowns.map(|v| v.to_string()).unwrap_or("null".into());
    let cycles  = disk.power_cycles.map(|v| v.to_string()).unwrap_or("null".into());
    let written = disk.data_written_tb.map(|v| format!("{:.4}", v)).unwrap_or("null".into());
    let read    = disk.data_read_tb.map(|v| format!("{:.4}", v)).unwrap_or("null".into());
    let model   = disk.model.as_deref().unwrap_or("").replace('"', "\\\"");
    let dev     = disk.dev.replace('"', "\\\"");

    // Extract individual SMART attributes that the AI model uses
    let realloc = find_attr_raw(&disk.smart_attributes, "reallocated_sector")
        .map(|v| v.to_string())
        .unwrap_or("null".into());
    let pending  = find_attr_raw(&disk.smart_attributes, "pending_sector")
        .map(|v| v.to_string())
        .unwrap_or("null".into());
    let uncorr   = find_attr_raw(&disk.smart_attributes, "uncorrectable_sector")
        .or_else(|| find_attr_raw(&disk.smart_attributes, "offline_uncorrectable"))
        .map(|v| v.to_string())
        .unwrap_or("null".into());

    format!(
        r#"{{"model":"{model}","dev":"{dev}","health_percent":{health},"temp_c":{temp},"power_on_hours":{hours},"unsafe_shutdowns":{unsafe_},"power_cycles":{cycles},"data_written_tb":{written},"data_read_tb":{read},"reallocated_sectors":{realloc},"pending_sectors":{pending},"uncorrectable_sectors":{uncorr}}}"#
    )
}

// ------------------------------------------------------------------ //
// HTTP helpers (raw TCP, no extra crate required)
// ------------------------------------------------------------------ //

/// Send a synchronous HTTP POST with a JSON body using the `ureq` crate.
/// Returns the response body as a String, or None on any error.
fn http_post(path: &str, body: &str) -> Option<String> {
    let url = format!("{}{}", AI_BASE_URL, path);
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(body)
        .ok()?;
    resp.into_string().ok()
}

// ------------------------------------------------------------------ //
// JSON parsing helpers (no serde — just basic string extraction)
// ------------------------------------------------------------------ //

/// Extract a JSON string field value from a flat JSON object.
/// e.g. extract_str(r#"{"label":"healthy"}"#, "label") -> Some("healthy")
fn extract_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let search = format!("\"{}\":", key);
    let start = json.find(&search)? + search.len();
    let after = json[start..].trim_start();
    if after.starts_with('"') {
        let inner = &after[1..];
        let end = inner.find('"')?;
        Some(&inner[..end])
    } else {
        None
    }
}

/// Extract a JSON number field value from a flat JSON object.
fn extract_f32(json: &str, key: &str) -> Option<f32> {
    let search = format!("\"{}\":", key);
    let start = json.find(&search)? + search.len();
    let after = json[start..].trim_start();
    let end = after
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .unwrap_or(after.len());
    after[..end].parse::<f32>().ok()
}

// ------------------------------------------------------------------ //
// Public API
// ------------------------------------------------------------------ //

/// Call `/predict` on the AI service with the given DiskInfo.
///
/// Returns `Some(AiResult)` on success, or `None` if the service is
/// unreachable or returns an unexpected response.
pub fn predict(disk: &DiskInfo) -> Option<AiResult> {
    let payload = serialise_smart(disk);
    let response = http_post("/predict", &payload)?;

    let label      = extract_str(&response, "label")?.to_string();
    let confidence = extract_f32(&response, "confidence")?;
    let reason     = extract_str(&response, "reason")?.to_string();
    let next_step  = extract_str(&response, "next_step")?.to_string();

    // Validate the label is one of the expected values
    if !["healthy", "watchlist", "risky"].contains(&label.as_str()) {
        return None;
    }

    Some(AiResult { label, confidence, reason, next_step })
}

/// Call `/ask` on the AI service with a user question and SMART context.
///
/// The `ai_result` (if available from a prior `/predict` call) is embedded
/// into the payload so the NLP engine has full context.
///
/// Returns `Some(answer_string)` on success, or `None` on failure.
pub fn ask(question: &str, disk: &DiskInfo, ai_result: Option<&AiResult>) -> Option<String> {
    if question.trim().is_empty() {
        return None;
    }

    let smart_json = serialise_smart(disk);

    // Inject AI result fields into the smart JSON if available
    let smart_with_ai = if let Some(r) = ai_result {
        // Insert ai_label, ai_confidence, ai_reason into the existing JSON object
        let label     = r.label.replace('"', "\\\"");
        let reason    = r.reason.replace('"', "\\\"");
        let conf      = r.confidence;
        // Splice before closing brace
        smart_json.trim_end_matches('}').to_string()
            + &format!(r#","ai_label":"{label}","ai_confidence":{conf:.2},"ai_reason":"{reason}"}}"#)
    } else {
        smart_json
    };

    let q_escaped = question.replace('"', "\\\"");
    let payload = format!(
        r#"{{"question":"{q_escaped}","smart":{smart_with_ai}}}"#
    );

    let response = http_post("/ask", &payload)?;
    extract_str(&response, "answer").map(|s| s.to_string())
}
