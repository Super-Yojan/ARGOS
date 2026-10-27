use argos_core::decode_objects;
use serde_json::json;

#[test]
fn keeps_confirmed_objects_with_and_without_gps() {
    let body = json!({"schema_version":1,"rover_id":4,"run_id":"r","revision":9,"objects":[
        {"id":1,"class":"bucket","color":"orange","x":2.0,"y":-1.5,"latitude":38.83,"longitude":-77.3,
         "geo_accuracy_m":3.2,"confidence":0.91,"sightings":5,"confirmed":true,"first_seen":1.0,"last_seen":3.0},
        {"id":2,"class":"ball","color":null,"x":0.5,"y":0.5,"latitude":null,"longitude":null,
         "geo_accuracy_m":null,"confidence":0.7,"sightings":3,"confirmed":true,"first_seen":1.0,"last_seen":2.0},
        {"id":3,"class":"bucket","color":"blue","x":9.0,"y":9.0,"latitude":null,"longitude":null,
         "geo_accuracy_m":null,"confidence":0.6,"sightings":1,"confirmed":false,"first_seen":1.0,"last_seen":1.0}
    ]});
    let objects = decode_objects(&body);
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].label(), "orange bucket");
    assert_eq!(objects[0].latitude, Some(38.83));
    assert_eq!(objects[0].marker_color(), 0xff8a1f);
    assert_eq!(objects[1].label(), "ball");
    assert_eq!(objects[1].latitude, None);
}

#[test]
fn drops_malformed_bodies_and_entries() {
    assert!(decode_objects(&json!({"schema_version":2,"objects":[]})).is_empty());
    assert!(decode_objects(&json!(null)).is_empty());
    let body = json!({"schema_version":1,"objects":[
        {"id":1,"class":"bad class","x":0.0,"y":0.0,"confidence":0.9,"sightings":3,"confirmed":true},
        {"id":2,"class":"bucket","x":"nope","y":0.0,"confidence":0.9,"sightings":3,"confirmed":true},
        {"id":3,"class":"bucket","x":1.0,"y":0.0,"latitude":38.0,"longitude":null,"confidence":0.9,"sightings":3,"confirmed":true}
    ]});
    let objects = decode_objects(&body);
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, 3);
    assert_eq!(objects[0].latitude, None, "half a coordinate is dropped");
}
