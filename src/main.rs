use anyhow::Ok;
use clap::Parser;
use log::{info, warn, error};
use simplelog::{LevelFilter, SimpleLogger};
use std::path::PathBuf;
use vmf_forge::prelude::VmfFile;
use pbr_lut_gen::*;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Input VMF file
    #[arg(short, long)]
    input: PathBuf,

    /// Game directory path
    #[arg(long)]
    game: Option<PathBuf>,

    /// Output path for the modified VMF. Defaults to {input}_pbr.vmf if not set
    #[arg(long)]
    output_vmf: Option<PathBuf>,

    /// Modifies VMF and SAVES the VMF to disk
    ///  Use this before compiling the map (VBSP)
    #[arg(long = "final", default_value_t = false)]
    final_mode: bool,

    /// Calculates everything and dumps logs, but doesn't write any files to disk
    #[arg(long, default_value_t = false)]
    draft_run: bool,

    /// Verbose info for debugging
    #[arg(long, default_value_t = false)]
    verbose: bool,

    /// Dump prepared light source data to the console for debugging
    #[arg(long, default_value_t = false)]
    dump_data: bool,

    /// Dump cluster scoring data to the console for debugging
    #[arg(long, default_value_t = false)]
    dump_clusters: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    setup_logging(args.verbose)?;

    if !args.input.exists() {
        error!("Input file does not exist: {:?}", args.input);
        return Ok(());
    }

    let map_name = args.input.file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid input filename"))?
        .to_string();

    let game_dir = args.game.unwrap_or_else(|| std::env::current_dir().unwrap());

    info!("Map Name: {}", map_name);
    info!("Game Directory: {:?}", game_dir);

    // Parse VMF
    let mut file = std::fs::File::open(&args.input)?;
    let mut vmf = VmfFile::parse_file(&mut file)?;

    // Extract Lights
    let all_lights = parser::extract_lights(&vmf)?;
    info!("Found {} PBR lights total", all_lights.len());

    // Associate lights with surfaces using PBR scoring and raytracing :p
    let clusters = processing::process_map_pipeline(
        &mut vmf,
        &all_lights,
        &game_dir,
        &map_name,
        args.draft_run  // Generate assets if not draft-run
    )?;
    info!("Generated {} LUT clusters", clusters.len());

    if args.dump_data {
        warn!("Dumping all extracted light data:");
        for light in &all_lights {
            println!("{:#?}", light);
        }
        println!("----------------------------------------------");
    }
    if args.dump_clusters {
        warn!("Dumping clusters and their scores:");
        for cluster in &clusters {
            println!("---\nCluster: {}", cluster.name);
            println!("   Min Score Threshold: {:.4}", cluster.min_cluster_score);

            println!("   [ACCEPTED] (Count: {})", cluster.lights.len());
            for (light, score) in &cluster.lights {
                let score_str = if *score > 10000.0 {
                    "FORCE".to_string()
                } else {
                    format!("{:.4}", score)
                };

                println!("     + {:<25} | Score: {}", light.debug_id, score_str);
            }

            if !cluster.rejected_lights.is_empty() {
                println!("   [REJECTED] (Count: {})", cluster.rejected_lights.len());
                for (light, score) in &cluster.rejected_lights {
                    println!("     - {:<25} | Score: {:.4}", light.debug_id, score);
                }
            } else {
                println!("   [REJECTED] (None)");
            }
        }
        println!("----------------------------------------------");
    }

    if args.draft_run {
        warn!("Draft run complete. No files written.");
        return Ok(());
    }

    // Generate VScript Data
    let nut_path = game_dir
        .join("scripts/vscripts/_autogen_debug")
        .join(format!("{}_pbr.nut", map_name));

    if let Some(parent) = nut_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    info!("Generating VScripts data file: {:?}", nut_path);
    nut_gen::generate_nut(&nut_path, &clusters, &all_lights)?;

    if !args.final_mode {
        warn!("Assets updated (Use --final to save modified VMF)");
        return Ok(());
    }

    let vmf_out = match args.output_vmf {
        Some(p) => p,
        None => {
            let mut p = args.input.clone();
            if let Some(stem) = p.file_stem() {
                let new_stem = format!("{}_pbr", stem.to_string_lossy());
                p.set_file_name(new_stem);
            }
            p.set_extension("vmf");
            p
        }
    };

    parser::strip_pbr_entities(&mut vmf);
    vmf.save(&vmf_out)?;
    info!("Saved modified VMF to: {:?}", vmf_out);

    Ok(())
}

fn setup_logging(verbose: bool) -> anyhow::Result<()> {
    let level = if verbose { LevelFilter::Debug } else { LevelFilter::Info };
    let config = simplelog::ConfigBuilder::default()
        .set_time_level(LevelFilter::Off)
        .set_thread_level(LevelFilter::Off)
        .build();
    if simplelog::TermLogger::init(level, config.clone(), simplelog::TerminalMode::Mixed, simplelog::ColorChoice::Auto).is_err() {
        SimpleLogger::init(level, config)?;
    }

    Ok(())
}
