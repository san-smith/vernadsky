//! Rasterization of `GeographicWorld` layers into RGB buffers.
//!
//! Climate layers use fixed physical scales (the model bounds of the
//! climate stage), so image series stay comparable across seeds and
//! stages; the height layer is normalized by the world's own min/max,
//! because planetary ranges vary. Categorical layers (territories,
//! biomes) use a deterministic golden-ratio hue palette.

use vernadsky_core::schema::{CellRecord, GeographicWorld, NO_INDEX};

/// Nominal extent of a value scale, in the layer's physical unit.
type Stop = (f64, Rgb);

/// An RGB pixel triple.
pub type Rgb = [u8; 3];

/// A rasterized layer: row-major RGB pixels at grid resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    /// Number of columns.
    pub width: u32,
    /// Number of rows.
    pub height: u32,
    /// Exactly `width * height` pixels, row-major.
    pub pixels: Vec<Rgb>,
}

/// The renderable layers of a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Surface height; sea surface is `0`, negative is depth.
    Height,
    /// Water coverage: water cells blue, land cells pale.
    Water,
    /// Annual-mean temperature, fixed scale −90…+60 °C (RdBu).
    Temperature,
    /// Relative humidity, fixed scale 0…100 % (BrBG).
    Humidity,
    /// Annual precipitation, fixed scale 0…3000 mm/year (YlGnBu).
    Precipitation,
    /// Biome assignment, categorical.
    Biome,
    /// Territory membership, categorical.
    Territory,
}

impl Layer {
    /// All layers, in canonical render order.
    pub const ALL: [Layer; 7] = [
        Layer::Height,
        Layer::Water,
        Layer::Temperature,
        Layer::Humidity,
        Layer::Precipitation,
        Layer::Biome,
        Layer::Territory,
    ];

    /// The file stem of the layer: `{out}.{name}.png`.
    pub fn name(self) -> &'static str {
        match self {
            Layer::Height => "height",
            Layer::Water => "water",
            Layer::Temperature => "temperature",
            Layer::Humidity => "humidity",
            Layer::Precipitation => "precipitation",
            Layer::Biome => "biome",
            Layer::Territory => "territory",
        }
    }
}

/// Water color of the coverage layer.
pub const WATER_COLOR: Rgb = [52, 110, 190];
/// Land color of the coverage layer.
pub const LAND_COLOR: Rgb = [238, 238, 238];
/// Color of cells outside any territory.
pub const NO_TERRITORY_COLOR: Rgb = [60, 60, 60];

/// Temperature scale: ColorBrewer RdBu endpoints around 0 °C.
const TEMPERATURE_STOPS: [Stop; 3] = [
    (-90.0, [49, 54, 149]),
    (0.0, [247, 247, 247]),
    (60.0, [165, 15, 21]),
];

/// Humidity scale: ColorBrewer BrBG endpoints.
const HUMIDITY_STOPS: [Stop; 3] = [
    (0.0, [84, 48, 5]),
    (50.0, [245, 245, 245]),
    (100.0, [0, 136, 55]),
];

/// Precipitation scale: ColorBrewer YlGnBu endpoints.
const PRECIPITATION_STOPS: [Stop; 3] = [
    (0.0, [255, 255, 217]),
    (1500.0, [65, 182, 196]),
    (3000.0, [8, 29, 88]),
];

/// Rasterizes one layer of `world`; `None` when the layer's section is
/// absent from the world (callers surface a warning instead of failing).
pub fn rasterize(world: &GeographicWorld, layer: Layer) -> Option<Raster> {
    let cells = &world.grid.cells;
    let mut pixels = Vec::with_capacity(cells.len());
    match layer {
        Layer::Height => {
            let (min, max) = height_range(cells);
            for cell in cells {
                pixels.push(gradient(
                    &[
                        (f64::from(min), [40, 40, 40]),
                        (f64::from(max), [235, 235, 235]),
                    ],
                    cell.height.to_value(),
                ));
            }
        }
        Layer::Water => {
            for cell in cells {
                pixels.push(if cell.is_water {
                    WATER_COLOR
                } else {
                    LAND_COLOR
                });
            }
        }
        Layer::Temperature => {
            let values = world.climate.temperature.as_ref()?;
            for value in values {
                pixels.push(gradient(&TEMPERATURE_STOPS, value.to_value()));
            }
        }
        Layer::Humidity => {
            let values = world.climate.humidity.as_ref()?;
            for value in values {
                pixels.push(gradient(&HUMIDITY_STOPS, value.to_value()));
            }
        }
        Layer::Precipitation => {
            let values = world.climate.precipitation.as_ref()?;
            for value in values {
                pixels.push(gradient(&PRECIPITATION_STOPS, value.to_value()));
            }
        }
        Layer::Biome => {
            let values = world.biomes.biome.as_ref()?;
            for value in values {
                pixels.push(categorical(u64::from(value.0)));
            }
        }
        Layer::Territory => {
            for cell in cells {
                pixels.push(if cell.territory == NO_INDEX {
                    NO_TERRITORY_COLOR
                } else {
                    categorical(u64::from(cell.territory))
                });
            }
        }
    }
    Some(Raster {
        width: world.grid.width,
        height: world.grid.height,
        pixels,
    })
}

/// Upscales a raster by an integer factor with nearest-neighbor
/// sampling.
pub fn zoom(raster: &Raster, factor: u32) -> Raster {
    let z = factor as usize;
    let w = raster.width as usize * z;
    let mut pixels = Vec::with_capacity(w * raster.height as usize * z);
    for y in 0..raster.height as usize {
        for _ in 0..z {
            for x in 0..raster.width as usize {
                for _ in 0..z {
                    pixels.push(raster.pixels[y * raster.width as usize + x]);
                }
            }
        }
    }
    Raster {
        width: raster.width * factor,
        height: raster.height * factor,
        pixels,
    }
}

/// Linear interpolation over sorted color stops; values outside the
/// scale clamp to the end colors.
fn gradient(stops: &[Stop], value: f64) -> Rgb {
    debug_assert!(!stops.is_empty() && stops.windows(2).all(|w| w[0].0 <= w[1].0));
    if value <= stops[0].0 {
        return stops[0].1;
    }
    if let Some(last) = stops.last()
        && value >= last.0
    {
        return last.1;
    }
    for pair in stops.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        if value <= hi.0 {
            let t = (value - lo.0) / (hi.0 - lo.0);
            return [
                lerp_channel(lo.1[0], hi.1[0], t),
                lerp_channel(lo.1[1], hi.1[1], t),
                lerp_channel(lo.1[2], hi.1[2], t),
            ];
        }
    }
    unreachable!("value is above the first stop and below the last")
}

fn lerp_channel(lo: u8, hi: u8, t: f64) -> u8 {
    (f64::from(lo) + (f64::from(hi) - f64::from(lo)) * t).round() as u8
}

/// The lowest and the highest cell height of the world; a degenerate
/// (constant) field collapses to a single point.
fn height_range(cells: &[CellRecord]) -> (i32, i32) {
    let mut min = i32::MAX;
    let mut max = i32::MIN;
    for cell in cells {
        min = min.min(cell.height.0);
        max = max.max(cell.height.0);
    }
    (min, max)
}

/// Deterministic categorical color: golden-ratio hue walk with fixed
/// saturation and value, so neighboring indices stay distinguishable.
fn categorical(index: u64) -> Rgb {
    let hue = (index as f64 * 0.618_033_988_749_894_9).fract();
    hsv_to_rgb(hue, 0.55, 0.9)
}

fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> Rgb {
    let sector = (hue * 6.0).floor() as i64 % 6;
    let fraction = hue * 6.0 - (hue * 6.0).floor();
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - fraction * saturation);
    let t = value * (1.0 - (1.0 - fraction) * saturation);
    let (r, g, b) = match sector {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    [
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_clamps_out_of_scale_values() {
        assert_eq!(gradient(&TEMPERATURE_STOPS, -120.0), TEMPERATURE_STOPS[0].1);
        assert_eq!(gradient(&TEMPERATURE_STOPS, 90.0), TEMPERATURE_STOPS[2].1);
    }

    #[test]
    fn gradient_interpolates_the_middle() {
        assert_eq!(gradient(&TEMPERATURE_STOPS, 0.0), [247, 247, 247]);
        // Halfway between the dark brown and the white stop.
        assert_eq!(gradient(&HUMIDITY_STOPS, 25.0), [165, 147, 125]);
    }

    #[test]
    fn categorical_colors_stay_distinguishable() {
        let a = categorical(0);
        let b = categorical(1);
        let c = categorical(2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn zoom_doubles_dimensions_and_repeats_pixels() {
        let raster = Raster {
            width: 2,
            height: 1,
            pixels: vec![[1, 2, 3], [4, 5, 6]],
        };
        let zoomed = zoom(&raster, 2);
        assert_eq!(zoomed.width, 4);
        assert_eq!(zoomed.height, 2);
        assert_eq!(
            zoomed.pixels,
            vec![
                [1, 2, 3],
                [1, 2, 3],
                [4, 5, 6],
                [4, 5, 6],
                [1, 2, 3],
                [1, 2, 3],
                [4, 5, 6],
                [4, 5, 6],
            ]
        );
    }
}
