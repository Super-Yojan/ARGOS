use super::{MapConfig, camera::MapMode};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// WGS84 degrees and height in meters above the WGS84 ellipsoid.
/// Heights are absolute; the map does not clamp objects to terrain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoPosition {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
}
impl GeoPosition {
    pub const fn new(latitude: f64, longitude: f64, altitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            altitude,
        }
    }
    fn validate(self) -> Result<(), MapError> {
        if self.latitude.is_finite()
            && self.longitude.is_finite()
            && self.altitude.is_finite()
            && self.latitude.abs() <= 90.
            && self.longitude.abs() <= 180.
        {
            Ok(())
        } else {
            Err(MapError::InvalidPosition)
        }
    }
}
/// An opaque instance handle, scoped to the Map that created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId {
    owner: u64,
    index: u64,
}
/// An sRGB highlight color. A highlight affects only the selected instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}
impl Color {
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
    pub(super) fn linear(self) -> [f32; 4] {
        let channel = |v: u8| {
            let x = v as f32 / 255.;
            if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        };
        [
            channel(self.red),
            channel(self.green),
            channel(self.blue),
            1.,
        ]
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectStatus {
    Loading,
    Ready,
    Failed(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapEvent {
    ObjectLoaded { id: ObjectId },
    ObjectLoadFailed { id: ObjectId, message: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapError {
    InvalidPosition,
    InvalidConfig(&'static str),
    InvalidTransform,
    UnknownObject,
    UnsupportedModel,
    CapacityReached,
    InvalidPath,
    WorkerUnavailable(String),
}
impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPosition => {
                f.write_str("Expected finite WGS84 latitude, longitude, and altitude")
            }
            Self::InvalidConfig(reason) => write!(f, "Invalid map config: {reason}"),
            Self::InvalidTransform => {
                f.write_str("Expected a finite heading and a finite positive scale")
            }
            Self::UnknownObject => f.write_str("Object was removed or belongs to another map"),
            Self::UnsupportedModel => {
                f.write_str("Supported model formats are local .glb and .gltf files")
            }
            Self::CapacityReached => f.write_str("MapConfig.max_objects reached"),
            Self::InvalidPath => f.write_str("Unable to resolve model path"),
            Self::WorkerUnavailable(reason) => write!(f, "Map worker unavailable: {reason}"),
        }
    }
}
impl std::error::Error for MapError {}

#[derive(Clone)]
pub(super) struct ObjectRecord {
    pub id: ObjectId,
    pub path: PathBuf,
    pub position: GeoPosition,
    pub heading: f64,
    pub scale: f64,
    pub highlight: Option<Color>,
}
#[derive(Clone)]
pub(super) struct Snapshot {
    pub revision: u64,
    pub scene_revision: u64,
    pub objects: Arc<Vec<ObjectRecord>>,
    pub navigation: Option<(u64, GeoPosition)>,
    pub mode: Option<(u64, MapMode)>,
}
struct State {
    revision: u64,
    scene_revision: u64,
    next_id: u64,
    worker_generation: u64,
    worker_failure: Option<String>,
    objects: HashMap<ObjectId, ObjectRecord>,
    statuses: HashMap<ObjectId, ObjectStatus>,
    navigation: Option<(u64, GeoPosition)>,
    mode: Option<(u64, MapMode)>,
    events: VecDeque<MapEvent>,
}
/// Cloneable, thread-safe controller. Create one MapView per controller.
/// Commands can be sent before the view is mounted or from fleet callbacks.
#[derive(Clone)]
pub struct Map {
    owner: u64,
    config: Arc<MapConfig>,
    state: Arc<Mutex<State>>,
}
impl Map {
    pub fn new(config: MapConfig) -> Result<Self, MapError> {
        GeoPosition::new(config.latitude, config.longitude, config.altitude).validate()?;
        if !config.distance.is_finite()
            || config.distance <= 0.
            || !config.heading.is_finite()
            || !config.pitch.is_finite()
            || !config.screen_space_error.is_finite()
            || config.screen_space_error <= 0.
        {
            return Err(MapError::InvalidConfig(
                "camera and detail values must be finite; distance and screen_space_error must be positive",
            ));
        }
        if config.max_objects == 0
            || config.max_model_bytes == 0
            || config.cache_bytes == 0
            || config.max_resident_tiles == 0
            || config.max_frame_pixels == 0
        {
            return Err(MapError::InvalidConfig("resource budgets must be positive"));
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            owner: NEXT.fetch_add(1, Ordering::Relaxed),
            config: Arc::new(config),
            state: Arc::new(Mutex::new(State {
                revision: 1,
                scene_revision: 1,
                next_id: 0,
                worker_generation: 0,
                worker_failure: None,
                objects: HashMap::new(),
                statuses: HashMap::new(),
                navigation: None,
                mode: None,
                events: VecDeque::new(),
            })),
        })
    }
    pub fn config(&self) -> &MapConfig {
        &self.config
    }
    /// Smoothly recenter once, preserving zoom, target altitude, and 2D/3D mode.
    pub fn show_lat_lon(&self, latitude: f64, longitude: f64) -> Result<(), MapError> {
        // NaN altitude is an internal sentinel for preserving the current camera altitude.
        GeoPosition::new(latitude, longitude, 0.).validate()?;
        self.navigate(GeoPosition::new(latitude, longitude, f64::NAN));
        Ok(())
    }
    /// Recenter and change the target's absolute ellipsoid height.
    pub fn show_position(&self, position: GeoPosition) -> Result<(), MapError> {
        position.validate()?;
        self.navigate(position);
        Ok(())
    }
    fn navigate(&self, position: GeoPosition) {
        let mut s = self.state.lock().unwrap();
        s.revision += 1;
        s.navigation = Some((s.revision, position));
    }
    pub fn set_mode(&self, mode: MapMode) {
        let mut s = self.state.lock().unwrap();
        s.revision += 1;
        s.mode = Some((s.revision, mode));
    }
    /// Schedule a local GLB/glTF load. Inspect object_status or poll_event for its result.
    pub fn add_3d_obj(
        &self,
        position: GeoPosition,
        path: impl AsRef<Path>,
    ) -> Result<ObjectId, MapError> {
        position.validate()?;
        let path = path.as_ref();
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("glb") || s.eq_ignore_ascii_case("gltf"))
        {
            return Err(MapError::UnsupportedModel);
        }
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|_| MapError::InvalidPath)?
                .join(path)
        };
        let mut s = self.state.lock().unwrap();
        if let Some(reason) = &s.worker_failure {
            return Err(MapError::WorkerUnavailable(reason.clone()));
        }
        if s.objects.len() >= self.config.max_objects {
            return Err(MapError::CapacityReached);
        }
        s.next_id += 1;
        let id = ObjectId {
            owner: self.owner,
            index: s.next_id,
        };
        s.objects.insert(
            id,
            ObjectRecord {
                id,
                path,
                position,
                heading: 0.,
                scale: 1.,
                highlight: None,
            },
        );
        s.statuses.insert(id, ObjectStatus::Loading);
        s.revision += 1;
        s.scene_revision += 1;
        Ok(id)
    }
    fn edit(&self, id: ObjectId, change: impl FnOnce(&mut ObjectRecord)) -> Result<(), MapError> {
        let mut s = self.state.lock().unwrap();
        let object = s.objects.get_mut(&id).ok_or(MapError::UnknownObject)?;
        change(object);
        s.revision += 1;
        s.scene_revision += 1;
        Ok(())
    }
    pub fn set_object_position(&self, id: ObjectId, position: GeoPosition) -> Result<(), MapError> {
        position.validate()?;
        self.edit(id, |o| o.position = position)
    }
    /// Clockwise degrees from the model's initial orientation about local up.
    pub fn set_object_heading(&self, id: ObjectId, degrees: f64) -> Result<(), MapError> {
        if !degrees.is_finite() {
            return Err(MapError::InvalidTransform);
        }
        self.edit(id, |o| o.heading = degrees.rem_euclid(360.))
    }
    pub fn set_object_scale(&self, id: ObjectId, scale: f64) -> Result<(), MapError> {
        if !scale.is_finite() || scale <= 0. {
            return Err(MapError::InvalidTransform);
        }
        self.edit(id, |o| o.scale = scale)
    }
    /// Apply an unlit emissive color tint; this does not cast light onto terrain.
    pub fn illuminate_3d_object(&self, id: ObjectId, color: Color) -> Result<(), MapError> {
        self.edit(id, |o| o.highlight = Some(color))
    }
    pub fn clear_illumination(&self, id: ObjectId) -> Result<(), MapError> {
        self.edit(id, |o| o.highlight = None)
    }
    pub fn remove_object(&self, id: ObjectId) -> Result<(), MapError> {
        let mut s = self.state.lock().unwrap();
        s.objects.remove(&id).ok_or(MapError::UnknownObject)?;
        s.statuses.remove(&id);
        s.events.retain(|e| match e {
            MapEvent::ObjectLoaded { id: i } | MapEvent::ObjectLoadFailed { id: i, .. } => *i != id,
        });
        s.revision += 1;
        s.scene_revision += 1;
        Ok(())
    }
    pub fn object_status(&self, id: ObjectId) -> Result<ObjectStatus, MapError> {
        self.state
            .lock()
            .unwrap()
            .statuses
            .get(&id)
            .cloned()
            .ok_or(MapError::UnknownObject)
    }
    /// Drain load notifications. At most 256 recent events are retained; status is authoritative.
    pub fn poll_event(&self) -> Option<MapEvent> {
        self.state.lock().unwrap().events.pop_front()
    }
    pub(super) fn revision(&self) -> u64 {
        self.state.lock().unwrap().revision
    }
    pub(super) fn snapshot(&self) -> Snapshot {
        let s = self.state.lock().unwrap();
        Snapshot {
            revision: s.revision,
            scene_revision: s.scene_revision,
            objects: Arc::new(s.objects.values().cloned().collect()),
            navigation: s.navigation,
            mode: s.mode,
        }
    }
    pub(super) fn worker_started(&self) -> u64 {
        let mut s = self.state.lock().unwrap();
        s.worker_generation += 1;
        s.worker_failure = None;
        for status in s.statuses.values_mut() {
            *status = ObjectStatus::Loading;
        }
        s.events.clear();
        s.revision += 1;
        s.scene_revision += 1;
        s.worker_generation
    }
    pub(super) fn worker_failed(&self, generation: u64, reason: String) {
        let mut s = self.state.lock().unwrap();
        if s.worker_generation != generation {
            return;
        }
        s.worker_failure = Some(reason.clone());
        let ids: Vec<_> = s.objects.keys().copied().collect();
        for id in ids {
            report(&mut s, id, ObjectStatus::Failed(reason.clone()));
        }
    }
    pub(super) fn report(&self, id: ObjectId, status: ObjectStatus) {
        let mut s = self.state.lock().unwrap();
        if s.worker_failure.is_none() {
            report(&mut s, id, status);
        }
    }
}

fn report(s: &mut State, id: ObjectId, status: ObjectStatus) {
    let Some(previous) = s.statuses.get_mut(&id) else {
        return;
    };
    if *previous == status {
        return;
    }
    *previous = status.clone();
    let event = match status {
        ObjectStatus::Ready => MapEvent::ObjectLoaded { id },
        ObjectStatus::Failed(message) => MapEvent::ObjectLoadFailed { id, message },
        ObjectStatus::Loading => return,
    };
    if s.events.len() == 256 {
        s.events.pop_front();
    }
    s.events.push_back(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fatal_worker_errors_reject_later_additions_until_restart() {
        let map = Map::new(MapConfig::default()).unwrap();
        let position = GeoPosition::new(0., 0., 0.);
        let id = map.add_3d_obj(position, "robot.glb").unwrap();
        let generation = map.worker_started();
        map.worker_failed(generation, "GPU unavailable".into());
        assert!(matches!(
            map.object_status(id).unwrap(),
            ObjectStatus::Failed(_)
        ));
        assert!(matches!(
            map.add_3d_obj(position, "robot.glb"),
            Err(MapError::WorkerUnavailable(_))
        ));
        map.worker_started();
        // A retired worker must not poison a replacement worker's controller.
        map.worker_failed(generation, "Old worker".into());
        assert_eq!(map.object_status(id).unwrap(), ObjectStatus::Loading);
        assert!(map.add_3d_obj(position, "robot.glb").is_ok());
    }
    #[test]
    fn callback_controller_and_validation() {
        let map = Map::new(MapConfig::default()).unwrap();
        let callback_map = map.clone();
        std::thread::spawn(move || callback_map.show_lat_lon(40., -74.).unwrap())
            .join()
            .unwrap();
        assert_eq!(map.snapshot().navigation.unwrap().1.latitude, 40.);
        assert!(map.show_lat_lon(91., 0.).is_err());
        assert!(map.show_lat_lon(0., f64::NAN).is_err());
    }
    #[test]
    fn handles_are_map_scoped_and_removed_objects_stay_removed() {
        let map = Map::new(MapConfig::default()).unwrap();
        let other = Map::new(MapConfig::default()).unwrap();
        let id = map
            .add_3d_obj(GeoPosition::new(0., 0., 10.), "robot.glb")
            .unwrap();
        assert!(other.remove_object(id).is_err());
        map.illuminate_3d_object(id, Color::rgb(255, 0, 0)).unwrap();
        assert_eq!(
            map.snapshot().objects[0].highlight,
            Some(Color::rgb(255, 0, 0))
        );
        map.remove_object(id).unwrap();
        map.report(id, ObjectStatus::Ready);
        assert!(map.object_status(id).is_err());
        assert!(map.poll_event().is_none());
    }
    #[test]
    fn capacity_transform_and_model_validation() {
        let map = Map::new(MapConfig {
            max_objects: 1,
            ..Default::default()
        })
        .unwrap();
        assert!(
            map.add_3d_obj(GeoPosition::new(0., 0., 0.), "robot.obj")
                .is_err()
        );
        let id = map
            .add_3d_obj(GeoPosition::new(0., 0., 0.), "robot.gltf")
            .unwrap();
        assert!(map.set_object_scale(id, 0.).is_err());
        assert!(map.set_object_heading(id, f64::INFINITY).is_err());
        assert!(
            map.add_3d_obj(GeoPosition::new(0., 0., 0.), "another.glb")
                .is_err()
        );
        assert_eq!(map.object_status(id).unwrap(), ObjectStatus::Loading);
        map.report(id, ObjectStatus::Ready);
        assert!(
            matches!(map.poll_event(), Some(MapEvent::ObjectLoaded { id: actual }) if actual == id)
        );
    }
}
