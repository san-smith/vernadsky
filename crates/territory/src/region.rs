//! Region grouping: continents and sea basins.
//!
//! A region is a connected component of the territory adjacency graph:
//! two territories are adjacent when a pair of their cells are
//! 4-neighbors (the map seamless along x, the poles clamped) **on the
//! same surface** — land territories group into continents, water
//! territories into sea basins, and a coastline never merges the two.
//!
//! The grouping uses `petgraph` (an undirected graph over the
//! territories, `UnionFind` for the components) — the graph stack the
//! research review kept for this project. The region identifier anchors
//! at the smallest member territory anchor through the stable identifier
//! strategy, and the regions are assigned in the canonical anchor order.
//! The stage consumes no randomness.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use petgraph::unionfind::UnionFind;
use petgraph::visit::EdgeRef;
use vernadsky_core::schema::{GeographicWorld, NO_INDEX, RegionRecord};
use vernadsky_core::{Anchor, RegionId, idgen::IdAssigner};

use crate::TerritoryError;

/// Groups the partitioned territories into regions and fills the
/// `regions` section and every `territory.region` reference.
///
/// A world without partitioned territories produces no regions and
/// stays unchanged otherwise. Pure with respect to `world`.
pub fn generate_regions(world: &mut GeographicWorld) -> Result<(), TerritoryError> {
    let territory_count = world.territories.len();
    if territory_count == 0 {
        return Ok(());
    }

    // --- Adjacency graph: one node per territory, an edge per cell pair
    // of different territories on the same surface ---
    let mut graph = petgraph::Graph::new_undirected();
    let nodes: Vec<NodeIndex> = (0..territory_count).map(|_| graph.add_node(())).collect();
    let row = world.grid.width as usize;
    let height = world.grid.height as usize;
    for index in 0..world.grid.cells.len() {
        let y = index / row;
        let x = index % row;
        // Right and down neighbors only: the undirected graph needs each
        // adjacent pair once, and the wrap edge (last column → first)
        // comes from the right neighbor of the last column.
        for (dy, dx) in [(0i64, 1i64), (1, 0)] {
            let ny = y as i64 + dy;
            if ny < 0 || ny >= height as i64 {
                continue;
            }
            let nx = (x as i64 + dx).rem_euclid(row as i64) as usize;
            let neighbor = ny as usize * row + nx;
            let territory = world.grid.cells[index].territory;
            let neighbor_territory = world.grid.cells[neighbor].territory;
            if territory != NO_INDEX
                && neighbor_territory != NO_INDEX
                && territory != neighbor_territory
                && world.grid.cells[index].is_water == world.grid.cells[neighbor].is_water
            {
                graph.add_edge(
                    nodes[territory as usize],
                    nodes[neighbor_territory as usize],
                    (),
                );
            }
        }
    }

    // --- Connected components through UnionFind ---
    let mut components = UnionFind::new(territory_count);
    for edge in graph.edge_references() {
        components.union(edge.source(), edge.target());
    }

    // --- Group territories by component root ---
    let mut members: HashMap<NodeIndex, Vec<usize>> = HashMap::new();
    for (position, node) in nodes.iter().enumerate() {
        members
            .entry(components.find(*node))
            .or_default()
            .push(position);
    }

    // --- Region anchor: the smallest member anchor — and the canonical
    // assignment order ---
    let mut anchored: Vec<(Anchor, Vec<usize>)> = members
        .into_values()
        .map(|members| {
            let anchor = members
                .iter()
                .map(|&position| world.territories[position].anchor.clone())
                .min_by_key(|anchor| (anchor.y, anchor.x))
                .expect("a region has at least one member territory");
            (anchor, members)
        })
        .collect();
    anchored.sort_by_key(|(anchor, _)| (anchor.y, anchor.x));

    let mut assigner = IdAssigner::new();
    let mut regions = Vec::with_capacity(anchored.len());
    let mut region_of_territory: Vec<Option<RegionId>> = vec![None; territory_count];
    for (anchor, members) in anchored.iter() {
        let id = assigner
            .region(anchor.x, anchor.y)
            .map_err(TerritoryError::IdGeneration)?;
        regions.push(RegionRecord { id });
        for &member in members {
            region_of_territory[member] = Some(id);
        }
    }

    for (territory, region) in world.territories.iter_mut().zip(region_of_territory) {
        territory.region = region;
    }
    world.regions = regions;
    Ok(())
}
