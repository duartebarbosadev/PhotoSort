//! Lazy, offline nearest-city lookup. Three-dimensional spherical coordinates
//! avoid the antimeridian discontinuity of a latitude/longitude Euclidean tree.
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Location {
    pub country: String,
    pub region: String,
    pub city: String,
}
struct Place {
    point: [f64; 3],
    location: Location,
}
struct Node {
    place: usize,
    left: Option<usize>,
    right: Option<usize>,
    axis: usize,
}
pub struct Geography {
    places: Vec<Place>,
    nodes: Vec<Node>,
    root: Option<usize>,
}
impl Geography {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let mut places = Vec::new();
        for line in std::fs::read_to_string(path)?.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 5 {
                continue;
            }
            let lat: f64 = fields[0].parse()?;
            let lon: f64 = fields[1].parse()?;
            places.push(Place {
                point: sphere(lat, lon),
                location: Location {
                    country: fields[2].into(),
                    region: fields[3].into(),
                    city: fields[4].into(),
                },
            });
        }
        let mut indices: Vec<_> = (0..places.len()).collect();
        let mut nodes = Vec::with_capacity(places.len());
        let root = build(&mut indices, 0, &places, &mut nodes);
        Ok(Self {
            places,
            nodes,
            root,
        })
    }
    pub fn nearest(&self, lat: f64, lon: f64) -> Option<Location> {
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
            return None;
        }
        let point = sphere(lat, lon);
        let mut best = (f64::INFINITY, 0usize);
        self.search(self.root, &point, &mut best);
        if best.0.is_finite() {
            Some(self.places[best.1].location.clone())
        } else {
            None
        }
    }
    fn search(&self, node: Option<usize>, point: &[f64; 3], best: &mut (f64, usize)) {
        let Some(index) = node else { return };
        let node = &self.nodes[index];
        let place = &self.places[node.place];
        let distance: f64 = place
            .point
            .iter()
            .zip(point)
            .map(|(a, b)| (a - b).powi(2))
            .sum();
        if distance < best.0 {
            *best = (distance, node.place);
        }
        let difference = point[node.axis] - place.point[node.axis];
        let (first, second) = if difference < 0.0 {
            (node.left, node.right)
        } else {
            (node.right, node.left)
        };
        self.search(first, point, best);
        if difference * difference < best.0 {
            self.search(second, point, best);
        }
    }
}
fn sphere(lat: f64, lon: f64) -> [f64; 3] {
    let (lat, lon) = (lat.to_radians(), lon.to_radians());
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}
fn build(
    indices: &mut [usize],
    depth: usize,
    places: &[Place],
    nodes: &mut Vec<Node>,
) -> Option<usize> {
    if indices.is_empty() {
        return None;
    }
    let axis = depth % 3;
    let middle = indices.len() / 2;
    indices.select_nth_unstable_by(middle, |a, b| {
        places[*a].point[axis].total_cmp(&places[*b].point[axis])
    });
    let (left, rest) = indices.split_at_mut(middle);
    let (place, right) = rest.split_first_mut().unwrap();
    let index = nodes.len();
    nodes.push(Node {
        place: *place,
        left: None,
        right: None,
        axis,
    });
    nodes[index].left = build(left, depth + 1, places, nodes);
    nodes[index].right = build(right, depth + 1, places, nodes);
    Some(index)
}
