use VMF_to_PBR::{vmt_helper::VmtPbrParams, *};

use anyhow::Context;
use clap::Parser;
use log::{debug, error, info, warn};
use simplelog::{LevelFilter, SimpleLogger};
use source_fs::{DummyVpk, FileSystem, FileSystemOptions, P2GameInfo};
use std::{collections::HashMap, path::PathBuf};
use vmf_forge::prelude::{Entity, VmfFile};


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

    /// Discards surfaces that have no active lights affecting them
    #[arg(long, default_value_t = false)]
    drop_unlit: bool, // todo: implement

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
    dump_lights: bool,

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

    let vmf_output = match args.output_vmf {
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

    let map_name = vmf_output.file_stem()
        .and_then(|s| s.to_str())
        .context("Invalid output filename")?
        .to_string();

    let game_dir = args.game
        .context("Game directory not provided. Use '--game <path>'")?;

    info!("VMF-to-PBR compiler by laVashik. Version: {}", env!("CARGO_PKG_VERSION"));
    info!("Map Name: {}", map_name);
    debug!("Game Directory: {:?}", game_dir);

    // == PRE-PROCESSING ==

    // Load Valve FileSystem for using with VMT parsing
    let options = FileSystemOptions::default();
    let vfs = FileSystem::<DummyVpk>::load_from_path::<P2GameInfo>(&game_dir, &options)
        .context("Failed to load filesystem. Check if gameinfo.txt exists")?;

    // Parse VMF
    let mut file = std::fs::File::open(&args.input)?;
    let mut vmf = VmfFile::parse_file(&mut file)?;

    // Extract Lights
    let all_lights = vmf_parser::extract_lights(&vmf)?;
    let world_brushes = geometry::build_collision_world(&vmf);
    let pcc_volumes = cubemaps::process_cubemaps(&vmf);
    let light_connection_registry = dynamic::build_connections_registry(&vmf); // todo: maybe move to LIGHT struct?

    info!("Found {} PBR lights total", all_lights.len());
    info!("Registry built. Tracked targets: {}", light_connection_registry.len());
    info!("Found {} PCC volumes.", pcc_volumes.len());

    // == Step 1: Generate STUFF ==
    let (ggx_ents, retained_ents): (Vec<_>, Vec<_>) = vmf.entities
        .drain(..)
        .partition(|ent| {
            ent.classname().unwrap_or("").to_lowercase() == "func_ggx_surface"
        });

    vmf.entities.0 = retained_ents;

    let mut ggx_surfaces: Vec<GgxSurfaceEnt> = ggx_ents
        .into_iter() // todo: use rayon!
        .map(GgxSurfaceEnt::new)
        .collect();

    let mut clusters: Vec<LightCluster> = ggx_surfaces
        .iter_mut() // todo: use rayon!
        .flat_map(|ggx_surface| {
            LightCluster::from_ggx_surface(
                ggx_surface, &ggx_surface.name, &map_name, &game_dir, &all_lights, &world_brushes, &pcc_volumes
            )
        })
        .collect();

    // optional filtering and skipping of clusters with no lights
    if args.drop_unlit {
        clusters.retain(|c| !c.lights.is_empty());
    }

    dumping_generated_data(args.dump_lights, args.dump_clusters, &all_lights, &clusters);
    info!("Generated {} LUT clusters", clusters.len());

    // == Step 2: Generate Assets ==
    if args.draft_run {
        warn!("Draft run complete. No files written.");
        return Ok(());
    }

    // GENERATE ASSETS
    let mut materials_cache: HashMap<&str, VmtPbrParams> = HashMap::new();
    for cluster in &mut clusters {
        let vtf_lut_name = format!("maps/{}/{}", map_name, cluster.surface_material);
        let vtf_path = cluster.surface_material_path.with_extension("vtf");
        let vmt_path = cluster.surface_material_path.with_extension("vmt");

        let orig_vmt = materials_cache.entry(&cluster.pbr_material).or_insert_with(|| {
            debug!("Parsing VMT for material: {}", cluster.pbr_material);
            match VmtPbrParams::find_and_parse(&vfs, &cluster.pbr_material) {
                Ok(vmt) => vmt,
                Err(m) => {
                    error!("Failed to process VMT: {} ({}). Skipping...", cluster.pbr_material, m);
                    VmtPbrParams::default()
                }
            }
        });

        // dbg!(&orig_vmt);

        if let Err(e) = vtf_lut::generate(&cluster, &vtf_path, &orig_vmt) {
            error!("Failed to create VTF for {:?}: {}", cluster.name, e);
        }

        vmt_patch::generate(
            &vmt_path,
            &vtf_lut_name,
            &orig_vmt,
            &cluster.initial_c4,
            cluster.cubemap_name.as_deref(),
        )?;
    }

    // Generate VScript Data
    let nut_path = game_dir
        .join("scripts/vscripts/_autogen_debug")
        .join(format!("{}.nut", map_name));

    if let Some(parent) = nut_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    info!("Generating VScripts data file: {:?}", nut_path);
    vscript::generate(&nut_path, &clusters, &all_lights)?;

    // == Step 3: Apply changes to VMF and save ==
    if !args.final_mode {
        warn!("Assets updated (Use --final to save modified VMF)");
        return Ok(());
    }

    info!("Integrating dynamic lighting controllers into the VMF...");
    dynamic::apply_dynamic_controllers(&mut vmf, &clusters, &light_connection_registry);

    info!("Applying geometry offsets and fixing UVs...");
    geometry::apply_offsets_and_uv_fixes(&clusters, &map_name, &world_brushes);

    // Convert GGX surfaces to real entities
    let pbr_surface_entities: Vec<Entity> = ggx_surfaces.into_iter().map(|s| s.convert_to_illusionary()).collect();
    vmf.entities.extend(pbr_surface_entities);

    // remove fake PBR entities
    vmf_parser::strip_pbr_entities(&mut vmf);
    vmf.save(&vmf_output)?; // and saving!
    info!("Saved modified VMF to: {:?}", vmf_output);

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

fn dumping_generated_data(dump_lights: bool, dump_clusters: bool, all_lights: &[LightDef], clusters: &[LightCluster]) {
    if dump_lights {
        println!("\nDumping all extracted light data:");
        all_lights.iter().for_each(|light| println!("{:#?}", light));
        println!("----------------------------------------------");
    }
    if dump_clusters {
        println!("\nDumping all light-clusters:");
        clusters.iter().for_each(LightCluster::dump);
        println!("----------------------------------------------");
    }
}
