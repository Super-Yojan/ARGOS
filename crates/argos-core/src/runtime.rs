//! Vehicle-issued request receipts; publishing alone is never acceptance.
use serde_json::Value;
pub fn vehicle_request_receipt(
    authority: &Value,
    issued_after_ms: u128,
) -> Option<(String, String)> {
    let token = authority["token"].as_str()?;
    if !token.starts_with("gpui-") {
        return None;
    }
    let issued = token.rsplit('-').next()?.parse::<u128>().ok()?;
    if issued < issued_after_ms {
        return None;
    }
    let result = authority["result"].as_str()?;
    let message = match result {
        "accepted" => "Vehicle accepted request".into(),
        "rejected" => format!(
            "Vehicle rejected request: {}",
            authority["request_reason"]
                .as_str()
                .or_else(|| authority["reason"].as_str())
                .unwrap_or("unspecified reason")
        ),
        _ => return None,
    };
    Some((
        format!(
            "{}:{token}:{}:{result}",
            authority["run_id"], authority["revision"]
        ),
        message,
    ))
}
