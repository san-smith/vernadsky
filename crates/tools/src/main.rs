//! Debug CLI of `vernadsky-tools`: renders PNG maps of world layers and
//! writes canonical export files.
//!
//! Debug tool: not part of the generator contract. Heavy dependencies
//! (image, argument parsing) live in this crate only; the generator
//! crates stay free of them.

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use image::{ImageBuffer, Rgb};

use vernadsky_tools::render::{Layer, Raster, rasterize, zoom};
use vernadsky_tools::{build_world, climate_params_from_toml};

/// How the world under rendering is produced.
#[derive(Clone, Debug)]
enum Source {
    /// A canonical export file.
    File(PathBuf),
    /// A synthetic world built from a seed through the current pipeline.
    Seed {
        seed: u64,
        no_climate: bool,
        config: Option<PathBuf>,
    },
}

#[derive(Parser)]
#[command(
    name = "vernadsky-tools",
    about = "Debug visualization and export dumping for Vernadsky worlds.\n\nDebug tool: not part of the generator contract."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render PNG maps of the world's layers as `{out}.{layer}.png`.
    Render {
        /// World seed: synthetic world built through the current pipeline.
        #[arg(long, required_unless_present = "input")]
        seed: Option<u64>,
        /// Path to a canonical export file (.gwb).
        #[arg(long, required_unless_present = "seed", conflicts_with = "seed")]
        input: Option<PathBuf>,
        /// Output path prefix; one PNG per available layer.
        #[arg(long, default_value = "map")]
        out: PathBuf,
        /// Build the --seed world without the climate stage.
        #[arg(long, conflicts_with = "config")]
        no_climate: bool,
        /// TOML configuration of the climate stage (with --seed).
        #[arg(long)]
        config: Option<PathBuf>,
        /// Nearest-neighbor upscale factor for readability.
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=16))]
        zoom: u32,
    },
    /// Build a world from a seed and write it as a canonical export file.
    Dump {
        /// World seed: synthetic world built through the current pipeline.
        #[arg(long)]
        seed: u64,
        /// Output file path.
        #[arg(long)]
        out: PathBuf,
        /// TOML configuration of the climate stage; without it the
        /// default configuration applies.
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Render {
            seed,
            input,
            out,
            no_climate,
            config,
            zoom: factor,
        } => {
            let source = match (input, seed) {
                (Some(path), _) => Source::File(path),
                (None, Some(seed)) => Source::Seed {
                    seed,
                    no_climate,
                    config,
                },
                (None, None) => unreachable!("clap enforces seed or input"),
            };
            cmd_render(&source, &out, factor)
        }
        Command::Dump { seed, out, config } => cmd_dump(seed, &out, config.as_deref()),
    }
}

fn load_world(source: &Source) -> anyhow::Result<vernadsky_core::GeographicWorld> {
    match source {
        Source::File(path) => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("cannot read export file {}", path.display()))?;
            vernadsky_core::GeographicWorld::from_bytes(&bytes)
                .with_context(|| format!("cannot decode export file {}", path.display()))
        }
        Source::Seed {
            seed,
            no_climate,
            config,
        } => {
            let climate = if *no_climate {
                None
            } else {
                Some(match config {
                    Some(path) => {
                        let source = std::fs::read_to_string(path).with_context(|| {
                            format!("cannot read climate config {}", path.display())
                        })?;
                        climate_params_from_toml(&source)?
                    }
                    None => vernadsky_climate::ClimateConfig::default()
                        .to_params()
                        .context("default climate config")?,
                })
            };
            build_world(*seed, climate.as_ref())
        }
    }
}

fn cmd_render(source: &Source, out: &std::path::Path, factor: u32) -> anyhow::Result<()> {
    let world = load_world(source)?;
    let mut rendered = 0;
    for layer in Layer::ALL {
        let Some(raster) = rasterize(&world, layer) else {
            eprintln!(
                "warning: layer '{}' skipped (section absent from the world)",
                layer.name()
            );
            continue;
        };
        let path = path_with_layer(out, layer.name());
        save_png(&zoom(&raster, factor), &path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        rendered += 1;
    }
    println!("{rendered} layer(s) written");
    Ok(())
}

fn cmd_dump(
    seed: u64,
    out: &std::path::Path,
    config: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let climate = match config {
        Some(path) => {
            let source = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read climate config {}", path.display()))?;
            Some(climate_params_from_toml(&source)?)
        }
        None => Some(
            vernadsky_climate::ClimateConfig::default()
                .to_params()
                .context("default climate config")?,
        ),
    };
    let world = build_world(seed, climate.as_ref())?;
    std::fs::write(out, world.to_bytes())
        .with_context(|| format!("cannot write {}", out.display()))?;
    println!("world written to {}", out.display());
    Ok(())
}

fn path_with_layer(out: &std::path::Path, layer: &str) -> PathBuf {
    let mut name = out.as_os_str().to_owned();
    name.push(format!(".{layer}.png"));
    PathBuf::from(name)
}

fn save_png(raster: &Raster, path: &std::path::Path) -> anyhow::Result<()> {
    let flat: Vec<u8> = raster
        .pixels
        .iter()
        .flat_map(|pixel| pixel.iter().copied())
        .collect();
    let image: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(raster.width, raster.height, flat)
            .context("pixel buffer does not match the raster dimensions")?;
    image.save(path)?;
    Ok(())
}
