//! Water body classification: flood fill over the water flags.
//!
//! A component is a maximally connected set of water cells (4-neighbors,
//! the map seamless along x). A component touching a polar row is the
//! world ocean; enclosed components are seas or lakes by area.

use std::collections::VecDeque;

use vernadsky_core::Anchor;
use vernadsky_core::schema::{CellRecord, WaterBodyKind};

/// Enclosed water bodies at least this large are seas; smaller ones are
/// lakes. On a 200×1000-cell map this is roughly one percent of the
/// surface — a Caspian-scale water body counts as a sea, a Baikal-scale
/// one as a lake.
pub(crate) const SEA_MIN_CELLS: usize = 200;

/// The four orthogonal neighbor offsets, in fixed scan order.
const NEIGHBORS: [(i64, i64); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// One connected water component.
#[derive(Clone, Debug)]
pub(crate) struct Component {
    /// Indices of the member cells, in BFS discovery order.
    pub cells: Vec<usize>,
    /// Sum of the member x coordinates, for the integer centroid.
    pub sum_x: i64,
    /// Sum of the member y coordinates, for the integer centroid.
    pub sum_y: i64,
    /// Whether the component touches a polar row (the world ocean).
    pub touches_pole: bool,
}

impl Component {
    /// The water body kind of this component: ocean when it reaches a
    /// polar row, otherwise sea or lake by area against `sea_min_cells`.
    pub fn kind(&self, sea_min_cells: usize) -> WaterBodyKind {
        if self.touches_pole {
            WaterBodyKind::Ocean
        } else if self.cells.len() >= sea_min_cells {
            WaterBodyKind::Sea
        } else {
            WaterBodyKind::Lake
        }
    }

    /// The integer centroid (floor) of the member cells: the anchor
    /// behind the stable identifier.
    pub fn anchor(&self) -> Anchor {
        let n = self.cells.len() as i64;
        assert!(n > 0, "a component has at least one cell");
        Anchor {
            x: self.sum_x / n,
            y: self.sum_y / n,
        }
    }
}

/// Classifies the water cells into connected components, in discovery
/// order (row-major scan, BFS with the x seam wrapped and the poles
/// disconnected).
pub(crate) fn components(cells: &[CellRecord], width: u32, height: u32) -> Vec<Component> {
    let w = width as usize;
    let h = height as usize;
    let mut visited = vec![false; cells.len()];
    let mut components = Vec::new();
    for start in 0..cells.len() {
        if !cells[start].is_water || visited[start] {
            continue;
        }
        let mut component = Component {
            cells: Vec::new(),
            sum_x: 0,
            sum_y: 0,
            touches_pole: false,
        };
        let mut queue = VecDeque::new();
        visited[start] = true;
        queue.push_back(start);
        while let Some(index) = queue.pop_front() {
            let y = index / w;
            let x = index % w;
            component.cells.push(index);
            component.sum_x += x as i64;
            component.sum_y += y as i64;
            if y == 0 || y == h - 1 {
                component.touches_pole = true;
            }
            for &(dy, dx) in &NEIGHBORS {
                let ny = y as i64 + dy;
                if ny < 0 || ny >= h as i64 {
                    continue; // the poles are not connected
                }
                let nx = (x as i64 + dx).rem_euclid(w as i64) as usize;
                let neighbor = ny as usize * w + nx;
                if cells[neighbor].is_water && !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use vernadsky_core::NO_INDEX;
    use vernadsky_core::quant::HeightM;

    /// Builds a cell grid from rows of `.` (land) and `~` (water).
    fn grid(rows: &[&str]) -> Vec<CellRecord> {
        rows.iter()
            .flat_map(|row| row.chars())
            .map(|symbol| CellRecord {
                height: HeightM(0),
                is_water: symbol == '~',
                territory: NO_INDEX,
                water_body: NO_INDEX,
            })
            .collect()
    }

    #[test]
    fn polar_water_is_the_ocean() {
        let cells = grid(&["~~~~", "....", "~~~~"]);
        let components = components(&cells, 4, 3);
        assert_eq!(components.len(), 2);
        assert_eq!(components[0].kind(SEA_MIN_CELLS), WaterBodyKind::Ocean);
        assert_eq!(components[1].kind(SEA_MIN_CELLS), WaterBodyKind::Ocean);
    }

    #[test]
    fn the_x_seam_wraps_into_one_component() {
        // Water on the left and right edges of the same row, land
        // between: the seam joins them into a single enclosed body.
        let cells = grid(&["....", "~..~", "...."]);
        let components = components(&cells, 4, 3);
        assert_eq!(components.len(), 1, "the seam must merge the halves");
        assert_eq!(components[0].kind(SEA_MIN_CELLS), WaterBodyKind::Lake);
    }

    #[test]
    fn enclosed_water_splits_into_sea_and_lake_by_area() {
        // 12×7: an outer polar-connected ocean; a 1-cell walled lake
        // (top left) and a 9-cell walled sea (right), both enclosed by
        // land borders.
        let mut rows = vec![vec!['~'; 12]; 7];
        for y in 1..=3 {
            for x in 1..=3 {
                if !(x == 2 && y == 2) {
                    rows[y][x] = '.';
                }
            }
        }
        for y in 1..=5 {
            for x in 5..=9 {
                let interior = (2..=4).contains(&y) && (6..=8).contains(&x);
                if !interior {
                    rows[y][x] = '.';
                }
            }
        }
        let cells: Vec<CellRecord> = rows
            .iter()
            .flat_map(|row| {
                row.iter().map(|symbol| CellRecord {
                    height: HeightM(0),
                    is_water: *symbol == '~',
                    territory: NO_INDEX,
                    water_body: NO_INDEX,
                })
            })
            .collect();
        let components = components(&cells, 12, 7);
        assert_eq!(components.len(), 3, "outer water, lake, and sea");
        let mut kinds: Vec<_> = components.iter().map(|c| c.kind(8)).collect();
        kinds.sort_by_key(|kind| kind.to_byte());
        assert_eq!(
            kinds,
            vec![
                WaterBodyKind::Ocean,
                WaterBodyKind::Sea,
                WaterBodyKind::Lake
            ]
        );
    }

    #[test]
    fn centroids_floor_to_the_member_mean() {
        let cells = grid(&["....", ".~~.", "...."]);
        let component = &components(&cells, 4, 3)[0];
        let anchor = component.anchor();
        // Members (1, 1) and (2, 1): the mean x is 1.5, floored to 1.
        assert_eq!((anchor.x, anchor.y), (1, 1));
    }
}
