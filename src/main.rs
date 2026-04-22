use VMF_to_PBR::{vmt_helper::VmtPbrParams, *};

use anyhow::Context;
use clap::Parser;
use log::{debug, error, info, warn};
use simplelog::{LevelFilter, SimpleLogger};
use source_fs::{DummyVpk, FileSystem, FileSystemOptions, P2GameInfo};
use std::path::PathBuf;
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

    let map_name = vmf_out.file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid output filename"))?
        .to_string();

    let game_dir = args.game.unwrap_or_else(|| std::env::current_dir().unwrap());

    info!("Map Name: {}", map_name);
    info!("Game Directory: {:?}", game_dir);

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
    let light_connection_registry = dynamic::build_connections_registry(&vmf); // todo: maybe move to LIGHT struct?

    let pcc_volumes = cubemaps::process_cubemaps(&vmf);
    info!("Found {} PBR lights total", all_lights.len());
    info!("Registry built. Tracked targets: {}", light_connection_registry.len());
    info!("Found {} PCC volumes.", pcc_volumes.len());

    // == Step 1: Generate STUFF
    let (ggx_ents, retained_ents): (Vec<_>, Vec<_>) = vmf.entities
        .drain(..)
        .partition(|ent| {
            ent.classname().unwrap_or("").to_lowercase() == "func_ggx_surface"
        });

    vmf.entities.0 = retained_ents;

    let mut ggx_surfaces: Vec<GgxSurfaceEnt> = ggx_ents
        .into_iter()
        .map(GgxSurfaceEnt::new)
        .collect();

    let mut clusters: Vec<LightCluster> = ggx_surfaces
        .iter_mut() // todo: use rayon!
        .enumerate()
        .flat_map(|(idx, ggx_surface)| {
            let cluster_name = ggx_surface.targetname()
                .map(String::from)
                .unwrap_or_else(|| {
                    let new_name = format!("surface_{}", idx);
                    ggx_surface.set("targetname".to_string(), new_name.clone());
                    new_name
                });

            LightCluster::from_ggx_surface(
                ggx_surface, &cluster_name, &map_name, &game_dir, &all_lights, &world_brushes, &pcc_volumes
            )
        })
        .collect();

    // todo: optional filtering and skipping of clusters with no lights

    // == Step 2: Process STUFF and Assets
    if args.draft_run {
        warn!("Draft run complete. No files written.");
        return Ok(());
    }

    // Prepare dynamic light handling
    for cluster in &clusters {
        let controllers = dynamic::build_dynamic_controllers(
            &cluster.lights,
            &cluster.ggx_surface_name,
            &cluster.pbr_material,
            &cluster.bound.center.to_origin(),
            &light_connection_registry
        );

        // Add 'modify_control_entities'. These entities are responsible for changing
        // values in c4 register slots, controlling light brightness.
        vmf.entities.extend(controllers.modify_control_entities);

        // Backpatch connections to existing entities. This integrates the new control
        // mechanisms by adding output connections to control created 'modify_control'.
        for (src_id, conns) in controllers.backpatch_connections {
            vmf.entities.find_by_keyvalue_mut("id", &src_id.to_string()).for_each(|ent| {
                let Some(connections) = &mut ent.connections else { return };
                for connect in conns.iter() {
                    connections.push(connect.clone());
                }
            });
        }
    }

    // == Geometry Shifting & UV Fix
    info!("Applying geometry offsets and fixing UVs...");
    for cluster in &clusters {
        // eg. "maps/sp_a2_triple_laser/surface_0_solid_0"
        let patch_material_str = format!("maps/{}/{}", map_name, cluster.surface_material);

        for solid_arc in &cluster.solids {
            let mut solid = solid_arc.write().unwrap();
            let normal = solid.surface_normal;

            // Calc geometry offset
            let max_axis = normal.0.abs().max(normal.1.abs()).max(normal.2.abs());
            let offset = normal * (GEOMETRY_OFFSET_UNITS * max_axis);

            // Raycast for find 'parent' and fix UV
            // TODO: or parse all solids to find it, OR make it in creating GgxSolid!
            // BECAUSE for now it's not a very stable and efficient solution.
            let target_pts = solid.sides.iter()
                .find(|side| side.material.eq_ignore_ascii_case(TARGET_MATERIAL))
                .and_then(|side| text::parse_plane_points(&side.plane));

            let start_pos = if let Some(pts) = target_pts {
                let p0 = pts[0];
                let k = (p0 - solid.bound.center).dot(normal);
                let point_on_plane = solid.bound.center + (normal * k);

                point_on_plane
            } else {
                solid.bound.center
            } + (normal * 5.0);

            debug!("  [UV Fix] Casting ray from {:?} dir {:?} (dist: {})", start_pos, normal, UV_SEARCH_DIST);

            let ray_dir = normal * -1.0;
            let parent_uv = tracer::trace_ray_closest(start_pos, ray_dir, UV_SEARCH_DIST, &world_brushes)
                .inspect(|hit| debug!("    -> x {:.2} (brush id: {}). Copying UVs.", hit.t, hit.id))
                .map(|hit| {
                    (hit.u_axis, hit.v_axis)
                });

            if parent_uv.is_none() {
                debug!("    -> No parent surface found within range.");
            }

            let mut material_updated = false;

            // Modify solid sides
            for side in &mut solid.sides {
                side.plane = text::apply_offset_to_plane(&side.plane, offset);

                // update material
                if side.material.eq_ignore_ascii_case(TARGET_MATERIAL) {
                    if let Some((u, v)) = parent_uv {
                        side.u_axis = u.to_string();
                        side.v_axis = v.to_string();
                    } else {
                        warn!("No parent_uv found for cluster: {}", cluster.name);
                    }

                    side.material = patch_material_str.clone();
                    material_updated = true;
                }
            }

            if !material_updated {
                warn!("The cluster {} (ggx_surface: {}) has no {} texture. PBS will be skipped!", cluster.name, cluster.ggx_surface_name, TARGET_MATERIAL);
            }
        }
    }

    // GENERATE ASSETS
    for cluster in &mut clusters {
        let vtf_lut_name = format!("maps/{}/{}", map_name, cluster.surface_material);
        let vtf_path = cluster.surface_material_path.with_extension("vtf");
        let vmt_path = cluster.surface_material_path.with_extension("vmt");

        // todo: cache template_materials and orig_vmt!
        let mut orig_vmt = match VmtPbrParams::find_and_parse(&vfs, &cluster.pbr_material) {
            Ok(vmt) => vmt,
            Err(m) => {
                error!("Failed to process VMT: {} ({}). Skipping...", cluster.pbr_material, m);
                continue;
            }
        };

        orig_vmt.num_lights = cluster.lights.len() as f32;
        // dbg!(&orig_vmt);

        if let Err(e) = vtf_lut::generate(&cluster, &vtf_path, &orig_vmt) {
            error!("Failed to create VTF for {}: {}", cluster.name, e); // todo: fix it and improve msg
        }

        vmt_patch::generate(
            &vmt_path,
            &vtf_lut_name,
            &orig_vmt,
            &cluster.initial_c4,
            cluster.cubemap_name.as_deref(),
        )?;
    }

    let pbr_surface_entities: Vec<Entity> = ggx_surfaces.into_iter().map(|s| s.convert_to_illusionary()).collect();
    vmf.entities.extend(pbr_surface_entities);

    info!("Generated {} LUT clusters", clusters.len());

    if args.dump_lights {
        warn!("Dumping all extracted light data:");
        for light in &all_lights {
            println!("{:#?}", light);
        }
        println!("----------------------------------------------");
    }
    if args.dump_clusters {
        warn!("Dumping clusters and their scores:");
        clusters.iter().for_each(LightCluster::dump);
        println!("----------------------------------------------");
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

    if !args.final_mode {
        warn!("Assets updated (Use --final to save modified VMF)");
        return Ok(());
    }

    vmf_parser::strip_pbr_entities(&mut vmf);
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
