//! Native Google Photorealistic 3D Tiles and georeferenced objects for GPUI.
//!
//! Create a controller, pass a clone to the view, and keep another clone in your
//! application's robot-selection callback. Fleet and robot types belong to your app.
//!
//! ```
//! use terra_map::{Map, MapConfig, MapMode, GeoPosition, Color};
//!
//! # fn main() -> Result<(), terra_map::MapError> {
//! let map = Map::new(MapConfig::default())?;
//! let robot = map.add_3d_obj(GeoPosition::new(37.7749, -122.4194, 80.), "robot.glb")?;
//! map.illuminate_3d_object(robot, Color::rgb(255, 160, 50))?;
//! let on_robot_clicked = {
//!     let map = map.clone();
//!     move |latitude, longitude| map.show_lat_lon(latitude, longitude)
//! };
//! on_robot_clicked(37.7749, -122.4194)?;
//! map.set_mode(MapMode::TwoD);
//! # Ok(())
//! # }
//! ```
//!
//! Mount with `cx.new(|cx| MapView::new(map.clone(), cx))` in a GPUI window.
//! Commands are retained before mounting. Local GLB/glTF loading starts when the
//! view receives its first nonzero viewport. One controller represents one view.
//! See the `fleet` example and `src/MAPS.md` for configuration and model conventions.
mod maps;
pub use maps::api::{Color, GeoPosition, Map, MapError, MapEvent, ObjectId, ObjectStatus};
pub use maps::camera::MapMode;
pub use maps::{MapConfig, MapView};
