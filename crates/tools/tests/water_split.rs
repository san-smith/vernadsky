//! The area split's water territories: spread seeds relaxed toward
//! their centroids make territories of even size, where the count
//! split's raster-order seeds leave rows of slivers beside large zones.

use vernadsky_tools::{WaterSplit, build_world_with};

fn water_sizes(split: WaterSplit) -> Vec<f64> {
    let climate = vernadsky_climate::ClimateConfig::default()
        .to_params()
        .expect("default climate");
    let world = build_world_with(20_261_012, 256, 128, 120, split, None, Some(&climate), None)
        .expect("pipeline");
    world
        .territories
        .iter()
        .filter(|t| t.is_water)
        .map(|t| f64::from(t.cell_count))
        .collect()
}

/// The coefficient of variation of the sizes.
fn spread(sizes: &[f64]) -> f64 {
    let mean = sizes.iter().sum::<f64>() / sizes.len() as f64;
    let variance = sizes.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / sizes.len() as f64;
    variance.sqrt() / mean
}

#[test]
fn spread_seeds_even_out_the_water_territories() {
    let by_area = water_sizes(WaterSplit::CellsPerTerritory(150));
    let by_count = water_sizes(WaterSplit::Count(by_area.len()));
    assert_eq!(
        by_area.len(),
        by_count.len(),
        "the same number of territories"
    );
    let (area, count) = (spread(&by_area), spread(&by_count));
    assert!(
        area < count * 0.8,
        "size spread: area split {area:.3}, count split {count:.3}"
    );
}
