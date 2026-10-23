use argos_core::*;
use serde_json::json;
#[test]
fn published_is_not_acknowledged_and_old_receipts_are_ignored() {
    assert!(
        vehicle_request_receipt(&json!({"token":"gpui-0-1000","result":"pending"}), 0).is_none()
    );
    assert!(
        vehicle_request_receipt(&json!({"token":"gpui-0-1000","result":"accepted"}), 2000)
            .is_none()
    );
    let (_,message)=vehicle_request_receipt(&json!({"run_id":"run","revision":3,"token":"gpui-0-3000","result":"rejected","request_reason":"pose_degraded"}),2000).unwrap();
    assert!(message.contains("rejected"));
    assert!(message.contains("pose_degraded"));
}
