//! Visual surface picking from loaded map triangles, independent of navigation data.
use super::{camera::ecef, content::TileContent};
use glam::{DMat4, DVec3};
use std::sync::{Arc, Mutex};

pub(super) type SharedSurface = Arc<Mutex<Vec<Arc<Surface>>>>;
pub(super) struct Surface {
    triangles: Vec<[DVec3; 3]>,
    nodes: Vec<Node>,
}
struct Node {
    min: DVec3,
    max: DVec3,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}
impl Surface {
    fn from_triangles(triangles: Vec<[DVec3; 3]>) -> Self {
        let mut surface = Self {
            triangles,
            nodes: vec![],
        };
        if !surface.triangles.is_empty() {
            surface.build(0, surface.triangles.len());
        }
        surface
    }
    fn build(&mut self, start: usize, end: usize) -> usize {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for triangle in &self.triangles[start..end] {
            for vertex in triangle {
                min = min.min(*vertex);
                max = max.max(*vertex);
            }
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let span = max - min;
            let axis = if span.x >= span.y && span.x >= span.z {
                0
            } else if span.y >= span.z {
                1
            } else {
                2
            };
            self.triangles[start..end].sort_unstable_by(|a, b| {
                (a[0][axis] + a[1][axis] + a[2][axis])
                    .total_cmp(&(b[0][axis] + b[1][axis] + b[2][axis]))
            });
            let middle = start + (end - start) / 2;
            let left = self.build(start, middle);
            let right = self.build(middle, end);
            self.nodes[index].children = Some((left, right));
        }
        index
    }
    fn hit(&self, index: usize, origin: DVec3, direction: DVec3, nearest: &mut f64) {
        let node = &self.nodes[index];
        let (mut lo, mut hi) = (0_f64, *nearest);
        for axis in 0..3 {
            if direction[axis].abs() < 1e-12 {
                if origin[axis] < node.min[axis] || origin[axis] > node.max[axis] {
                    return;
                }
            } else {
                let a = (node.min[axis] - origin[axis]) / direction[axis];
                let b = (node.max[axis] - origin[axis]) / direction[axis];
                lo = lo.max(a.min(b));
                hi = hi.min(a.max(b));
                if lo > hi {
                    return;
                }
            }
        }
        if let Some((left, right)) = node.children {
            self.hit(left, origin, direction, nearest);
            self.hit(right, origin, direction, nearest);
            return;
        }
        for &[a, b, c] in &self.triangles[node.start..node.end] {
            let e1 = b - a;
            let e2 = c - a;
            let p = direction.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-12 {
                continue;
            }
            let t = origin - a;
            let u = t.dot(p) / det;
            let q = t.cross(e1);
            let v = direction.dot(q) / det;
            if u < -1e-9 || v < -1e-9 || u + v > 1. + 1e-9 {
                continue;
            }
            let distance = e2.dot(q) / det;
            if distance >= 0. && distance < *nearest {
                *nearest = distance;
            }
        }
    }
    pub fn from_content(content: &TileContent, world: DMat4) -> Self {
        let mut triangles = Vec::new();
        for primitive in &content.primitives {
            let transform = world * primitive.transform;
            for indices in primitive.indices.chunks_exact(3) {
                let triangle = [indices[0], indices[1], indices[2]].map(|index| {
                    transform.transform_point3(DVec3::from_array(
                        primitive.vertices[index as usize].position.map(f64::from),
                    ))
                });
                if triangle.iter().all(|v| v.is_finite()) {
                    triangles.push(triangle);
                }
            }
        }
        Self::from_triangles(triangles)
    }
    pub fn altitude(&self, latitude: f64, longitude: f64) -> Option<f64> {
        if self.nodes.is_empty() || !latitude.is_finite() || !longitude.is_finite() {
            return None;
        }
        let base = ecef(latitude, longitude, 0.);
        let up = (ecef(latitude, longitude, 1.) - base).normalize();
        let mut distance = 40_000.;
        self.hit(0, base + up * 20_000., -up, &mut distance);
        (distance < 40_000.).then_some(20_000. - distance)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn patch(height: f64) -> [DVec3; 3] {
        let base = ecef(0., 0., height);
        [
            base + DVec3::new(0., -10., -10.),
            base + DVec3::new(0., 10., -10.),
            base + DVec3::new(0., 0., 10.),
        ]
    }
    #[test]
    fn picks_visible_surface_above_reference_and_misses_outside() {
        let surface = Surface::from_triangles(vec![patch(140.), patch(160.)]);
        assert!((surface.altitude(0., 0.).unwrap() - 160.).abs() < 1e-5);
        assert!(surface.altitude(1., 1.).is_none());
    }
    #[test]
    fn refinement_replaces_old_height_and_negative_heights_work() {
        let surface = Surface::from_triangles(vec![patch(-30.)]);
        assert!((surface.altitude(0., 0.).unwrap() + 30.).abs() < 1e-5);
        assert!(Surface::from_triangles(vec![]).altitude(0., 0.).is_none());
    }
}
