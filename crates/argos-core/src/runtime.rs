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
            "Vehicle refused: {}",
            refusal_text(
                authority["request_reason"]
                    .as_str()
                    .or_else(|| authority["reason"].as_str())
                    .unwrap_or("")
            )
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

/// Plain words for a Terra refusal reason (`autonomy/status.request_reason`),
/// saying what the operator should do where there is something to do.
/// Unknown reasons are shown as they are, with spaces for underscores.
pub fn refusal_text(reason: &str) -> String {
    match reason.trim() {
        // Terra since the refusal-reasons change: a goal, search or task while stopped.
        "safety_stopped" => "rover is stopped. Press Reset stop first".into(),
        // Older Terra builds also send this for a stopped rover.
        "authority_required" => "rover is stopped, or its level does not take this".into(),
        "still_tilted" => "still tilted. Right the rover, then press Reset stop".into(),
        "pose_not_ready" => "pose is not ready. Wait for tracking, then press Reset stop".into(),
        // Older Terra builds: any refused reset.
        "unhealthy_reset" => "rover is not ready to reset (still tilted, or pose not ready)".into(),
        "current_authority_required" => {
            "request was out of date (another command went first)".into()
        }
        "goal_out_of_bounds" => "goal is outside the map".into(),
        "invalid_goal" => "goal was not valid".into(),
        "" => "no reason given".into(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_read_as_plain_words() {
        assert_eq!(
            refusal_text("still_tilted"),
            "still tilted. Right the rover, then press Reset stop"
        );
        assert_eq!(
            refusal_text("safety_stopped"),
            "rover is stopped. Press Reset stop first"
        );
        assert_eq!(refusal_text("search_active"), "search active");
        assert_eq!(refusal_text(""), "no reason given");
    }

    #[test]
    fn refused_reset_receipt_says_what_to_do() {
        let authority = serde_json::json!({
            "token": "gpui-safety-1-200",
            "result": "rejected",
            "request_reason": "still_tilted",
            "run_id": "run",
            "revision": 4,
        });
        let (_, message) = vehicle_request_receipt(&authority, 100).unwrap();
        assert_eq!(
            message,
            "Vehicle refused: still tilted. Right the rover, then press Reset stop"
        );
    }
}
