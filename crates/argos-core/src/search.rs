use crate::{Result, invalid, valid_token};
/// Validate user intent without accepting sensor evidence from an operator client.
pub fn validate_search_command(kind: &str, payload: &str) -> Result<serde_json::Value> {
    if payload.len() > 2048 {
        return Err(invalid("search request too large"));
    }
    let v: serde_json::Value =
        serde_json::from_str(payload).map_err(|_| invalid("invalid search JSON"))?;
    let o = v
        .as_object()
        .ok_or_else(|| invalid("search object required"))?;
    if v["version"] != 1
        || ["run_id", "search_id", "token"]
            .iter()
            .any(|k| v[k].as_str().is_none_or(|s| !valid_token(s)))
    {
        return Err(invalid("invalid search session"));
    }
    let allowed: &[&str] = match kind {
        "search" => {
            let class = v["target_class"]
                .as_str()
                .ok_or_else(|| invalid("target class required"))?;
            if !(1..=64).contains(&class.len())
                || !class
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            {
                return Err(invalid("invalid target class"));
            }
            let b = &v["bounds"];
            if b.as_object().is_none_or(|b| b.len() != 4)
                || ["min_x", "min_y", "max_x", "max_y"]
                    .iter()
                    .any(|k| b[k].as_f64().is_none_or(|n| !n.is_finite()))
            {
                return Err(invalid("invalid search bounds"));
            }
            for (a, c) in [("min_x", "max_x"), ("min_y", "max_y")] {
                let width = b[c].as_f64().unwrap() - b[a].as_f64().unwrap();
                if width <= 0. || width > 100. {
                    return Err(invalid("search area must have sides up to 100 metres"));
                }
            }
            if v["time_budget_s"]
                .as_f64()
                .is_none_or(|n| !n.is_finite() || n <= 0. || n > 3600.)
            {
                return Err(invalid("invalid search budget"));
            }
            &[
                "version",
                "run_id",
                "search_id",
                "token",
                "target_class",
                "bounds",
                "time_budget_s",
            ]
        }
        "search/action" => {
            if !matches!(v["action"].as_str(), Some("pause" | "resume" | "cancel")) {
                return Err(invalid("invalid search action"));
            }
            &["version", "run_id", "search_id", "token", "action"]
        }
        "search/report/ack" => {
            if v["report_id"].as_str().is_none_or(|s| {
                s.is_empty()
                    || s.len() > 128
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            }) {
                return Err(invalid("invalid report id"));
            }
            &["version", "run_id", "search_id", "report_id", "token"]
        }
        _ => return Err(invalid("unsupported search command")),
    };
    if o.len() != allowed.len() || o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid("unexpected search field"));
    }
    Ok(v)
}
