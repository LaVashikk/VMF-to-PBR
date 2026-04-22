use derive_more::{Deref, DerefMut};
use geometry::ConvexBrush;
use log::{debug, info, warn, error};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};
use vmf_forge::prelude::{Entity, Solid, VmfFile};
use crate::generator::{self, LUT_WIDTH};
use crate::math::{AABB, Vec3};
use crate::parser::sanitize_name;
use crate::types::{LightCluster, LightDef, ParallaxVolume};
use utils::*;

pub mod geometry;
pub mod tracer;
pub mod scoring;
pub mod utils;

// Defines the material that identifies faces to be patched
const TARGET_MATERIAL: &str = "tools/toolspbr";
const GEOMETRY_OFFSET_UNITS: f32 = 0.8; // for offsets
const UV_SEARCH_DIST: f32 = 16.0;

const MAX_CUSTOM_SLOTS: usize = 4; // for force include/exclude

#[derive(Debug)]
pub struct LightConnection {
    source_entity_id: usize, // todo: id
    target_name: String,
    output_name: String,
    input_type: LightInputType,
    delay: f32,
}

#[derive(Debug, PartialEq)]
enum LightInputType {
    TurnOn,
    TurnOff,
    // todo: Toggle and SetPattern is complex to handle
}

impl LightConnection {
    fn parse(ent: &Entity) -> Option<Self> {
        let Some(connections) = &ent.connections else { return None; };

        for (output, value) in connections {
            // Parse VMF connection string: "TargetEntity,Input,Param,Delay,Limit"
            let parts: Vec<&str> = value.split([',', '\x1B']).collect(); // TODO: move to vmf-forge!
            let target = parts[0].trim();
            let input = parts[1].trim();
            let delay = parts.get(3).and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(0.0);

            let input_type = match input.to_lowercase().as_str() {
                "turnon" => Some(LightInputType::TurnOn),
                "turnoff" => Some(LightInputType::TurnOff),
                _ => None
            };

            if let Some(it) = input_type {
                // if target is "!self" -> then use "ent.targetname()"
                let raw_name = if target.eq_ignore_ascii_case("!self") {
                    let Some(name) = ent.targetname() else { continue };
                    name
                } else {
                    target
                };

                let target_name = raw_name.to_lowercase();

                return Some(LightConnection {
                    source_entity_id: ent.id() as usize,
                    target_name,
                    output_name: output.clone(),
                    input_type: it,
                    delay,
                });
            }
        }

        None
    }
}

const OVERRIDE_PARMS: [&str; 6] = [
    "ovr_roughness_mult",
    "ovr_base_reflectivity",
    "ovr_reflection_intensity",
    "ovr_normal_intensity",
    "ovr_fade_start",
    "ovr_fade_end",
];

#[derive(Debug, Deref, DerefMut)]
pub struct GgxSurfaceEnt {
    #[deref]
    #[deref_mut]
    pub entity: Entity,
    pub ggx_solids: Vec<Arc<RwLock<GgxSolid>>>,

    pub origin: String,
    pub bounding_box: AABB,
    pub id: u64,

    // ggx_surface custom parms
    pub exclude_lights: HashSet<String>,
    pub force_lights: HashSet<String>,
    pub min_score: f32,
    pub merge_solids: bool,

    pub template_material: String,
    pub override_roughness_mult: Option<f32>,
    pub override_base_reflectivity: Option<f32>,
    pub override_reflection_intensity: Option<f32>,
    pub override_normal_intensity: Option<f32>,
    pub override_fade_start: Option<f32>,
    pub override_fade_end: Option<f32>,
}

#[derive(Debug, Deref, DerefMut)]
pub struct GgxSolid {
    #[deref]
    #[deref_mut]
    pub solid: Solid,
    pub surface_normal: Vec3,
    pub surface_center: Vec3,
    pub bound: AABB,
}

impl GgxSolid {
    pub fn new(solid: Solid) -> Self {
        let target_points = solid.sides.iter()
            .find(|side| side.material.eq_ignore_ascii_case(TARGET_MATERIAL))
            .and_then(|side| parse_plane_points(&side.plane))
            .unwrap();  // SAFETY: trust me!

        // Calc surface normal
        let surface_normal = calc_face_normal(target_points) * -1.0;

        // Center of face surface // todo: fix and use in UV-parent finder?
        let surface_center = (target_points[0] + target_points[1] + target_points[2]) / 3.0;

        // Calc bounding box
        let bounding_box = geometry::get_solid_aabb(&solid).unwrap_or(AABB::new());

        Self {
            solid,
            surface_normal,
            surface_center,
            bound: bounding_box,
        }
    }
}

impl GgxSurfaceEnt {
    pub fn new(mut entity: Entity) -> Self {
        let id = entity.id();

        // Get surface origin. If not set, use AABB center
        let bounding_box = geometry::get_entity_aabb(&entity).unwrap_or(AABB::new());
        let origin = entity.get("origin").cloned().unwrap_or_else(|| {
            let c = bounding_box.center;
            format!("{:.2} {:.2} {:.2}", c[0], c[1], c[2])
        });

        // PBR material, which uses the corresponding shader
        let Some(template_material) = entity.get("template_material").cloned() else {
            log::warn!("No template_material for func_ggx_surface (hammer id: {}, origin: {}). It's required!", id, origin);
            panic!("Missing required key 'template_material' for func_ggx_surface");
        };

        // Custom light filtering, processing exclude/force lights
        let mut exclude_lights: HashSet<String> = HashSet::new();
        let mut force_lights: HashSet<String> = HashSet::new();
        for i in 1..=MAX_CUSTOM_SLOTS {
            if let Some(name) = entity.get(&format!("exclude_light_{}", i))
                && !name.is_empty() {
                    exclude_lights.insert(sanitize_name(name));
                }
            if let Some(name) = entity.get(&format!("force_light_{}", i))
                && !name.is_empty() {
                    force_lights.insert(sanitize_name(name));
                }
        }

        let min_score = entity.get("min_score").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.10);
        let merge_solids = entity.get("merge_solids").map(|s| s != "0").unwrap_or(false);

        // TODO: use override parms
        let [
            override_roughness_mult,
            override_base_reflectivity,
            override_reflection_intensity,
            override_normal_intensity,
            override_fade_start,
            override_fade_end,
        ] = OVERRIDE_PARMS.map(|parm| {
            entity.get(parm)
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|&v| v >= 0.0)
        });

        let solids = std::mem::take(entity.solids.as_mut().unwrap());
        let ggx_solids: Vec<Arc<RwLock<GgxSolid>>> = solids
            .into_iter()
            .map(|solid| Arc::new(RwLock::new(GgxSolid::new(solid)))) // fck my life, im awesome! :>
            .collect();

        // TODO: light_search_radius?

        Self {
            entity,
            ggx_solids,

            id,
            origin,
            bounding_box,

            template_material,
            min_score,
            merge_solids,

            override_roughness_mult,
            override_base_reflectivity,
            override_reflection_intensity,
            override_normal_intensity,
            override_fade_start,
            override_fade_end,

            exclude_lights,
            force_lights,
        }
    }

    pub fn convert_to_illusionary(mut self) -> Entity {
        // self.entity.set("classname".to_string(), "func_brush".to_string());
        self.entity.set("classname".to_string(), "func_illusionary".to_string());
        self.entity.set("renderamt".to_string(), "200".to_string());
        self.entity.set("rendermode".to_string(), "5".to_string());

        self.entity.set("disableflashlight".to_string(), "1".to_string());
        self.entity.set("disableshadows".to_string(), "1".to_string());
        self.entity.set("disablereceiveshadows".to_string(), "1".to_string());
        self.entity.set("disableshadowdepth".to_string(), "1".to_string());
        self.entity.set("rendertocubemaps".to_string(), "0".to_string());

        let original_solids: Vec<Solid> = self.ggx_solids
            .iter()
            .map(|arc| arc.read().unwrap().clone()) // yeah, i need to clone all data here :<
            .collect();

        self.entity.solids = Some(original_solids);

        self.entity
    }
}

impl LightCluster {
    fn find_parallax_volume(origin: Vec3, surface_normal: Vec3, pcc_volumes: &[InternalVolume]) -> Option<ParallaxVolume> {
        let mut best_pcc_data = None;

        // todo: made better way to find closest cubemap
        for vol in pcc_volumes {
            // if origin[0] >= vol.ws_min[0] && origin[0] <= vol.ws_max[0] &&
            //    origin[1] >= vol.ws_min[1] && origin[1] <= vol.ws_max[1] &&
            //    origin[2] >= vol.ws_min[2] && origin[2] <= vol.ws_max[2]
            // {
                    if !vol.cubemaps_inside.is_empty() {
                        let mut best_score = f32::MIN;
                        let mut best_c = vol.cubemaps_inside[0];

                        for &c in &vol.cubemaps_inside {
                            let to_cubemap = c - origin;
                            let dist_sq = to_cubemap.dot(to_cubemap);
                            let mut score = -dist_sq;

                            let dir_to_cubemap = to_cubemap.normalize();
                            let facing = surface_normal.dot(dir_to_cubemap);

                            // Cubemap behind ur back? GFY!
                            if facing < 0.0 {
                                score -= 1_000_000.0;
                            } else {
                                score += facing * 100.0;
                            }

                            if score > best_score {
                                best_score = score;
                                best_c = c;
                            }
                        }

                        best_pcc_data = Some(crate::types::ParallaxVolume {
                            cubemap_pos: best_c,
                            ws_min: vol.ws_min,
                            ws_max: vol.ws_max,
                        });
                    }
                //    break;
            //    }
        }

        best_pcc_data
    }
}

pub fn process_map_pipeline(
    vmf: &mut VmfFile,
    all_lights: &[LightDef],
    game_dir: &Path,
    map_name: &str,
    is_draft_run: bool
) -> anyhow::Result<Vec<LightCluster>> {
    // == Pre-pass
    let world_brushes = build_collision_world(vmf);

    let light_connection_registry = build_connections_registry(vmf); // todo: maybe move to LIGHT struct?
    info!("Registry built. Tracked targets: {}", light_connection_registry.len());

    let pcc_volumes = process_cubemaps(vmf);
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
                ggx_surface, &cluster_name, map_name, game_dir, all_lights, &world_brushes, &pcc_volumes
            )
        })
        .collect();

    // == Step 2: Process STUFF and Assets
    if is_draft_run {
        // it's draft, no need change geometry
        return Ok(clusters);
    }

    // Prepare dynamic light handling
    for cluster in &clusters {
        let controllers = build_dynamic_controllers(
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
                .and_then(|side| parse_plane_points(&side.plane));

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
                side.plane = apply_offset_to_plane(&side.plane, offset);

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
        let mut orig_vmt = match generator::find_and_process_vmt(game_dir, &cluster.pbr_material) {
            Ok(vmt) => vmt,
            Err(m) => {
                error!("Failed to process VMT: {} ({}). Skipping...", cluster.pbr_material, m);
                continue;
            }
        };

        orig_vmt.num_lights = cluster.lights.len() as f32;
        // dbg!(&orig_vmt);

        if let Err(e) = generator::generate_vtf(&cluster, &vtf_path, &orig_vmt) {
            error!("Failed to create VTF for {}: {}", cluster.name, e); // todo: fix it and improve msg
        }

        generator::generate_vmt(
            &vmt_path,
            &vtf_lut_name,
            &orig_vmt,
            &cluster.initial_c4,
            cluster.cubemap_name.as_deref(),
        )?;
    }

    let pbr_surface_entities: Vec<Entity> = ggx_surfaces.into_iter().map(|s| s.convert_to_illusionary()).collect();
    vmf.entities.extend(pbr_surface_entities);

    Ok(clusters)
}

/// Builds the collision world from VMF solids and func_details
pub fn build_collision_world(vmf: &VmfFile) -> Vec<ConvexBrush> {
    debug!("Building collision world...");
    let mut brushes = Vec::new();

    // World Solids (worldspawn)
    debug!("Processing {} world solids...", vmf.world.solids.len());
    for solid in &vmf.world.solids {
        if let Some(brush) = ConvexBrush::from_vmf_solid(solid) {
            brushes.push(brush);
        }
    }

    // Func Detail
    for ent in vmf.entities.iter() {
        let classname = ent.classname().unwrap_or("");
        if let Some(should_skip) = ent.get("pbr_geometry_ignore") {  // todo
            if should_skip != "0" { continue }
        }

        if classname == "func_detail" { // ? i think we should ignore any dynamic ents
            debug!("Found collidable entity: class='{}', targetname='{}'", classname, ent.targetname().unwrap_or("N/A"));
            if let Some(solids) = &ent.solids {
                for solid in solids {
                    if let Some(brush) = ConvexBrush::from_vmf_solid(solid) {
                        brushes.push(brush);
                    }
                }
            }
        }
    }

    info!("Built collision world with {} brushes.", brushes.len());
    brushes
}

pub type LightConnectionRegistry = HashMap<u64, Vec<LightConnection>>;

pub fn build_connections_registry(vmf: &VmfFile,) -> LightConnectionRegistry {
    let mut light_connection_registry: LightConnectionRegistry = HashMap::new();
    for ent in vmf.entities.iter() {
        if let Some(light_connecting) = LightConnection::parse(ent) {
            let key = ent.id();
            light_connection_registry
                .entry(key)
                .or_default()
                .push(light_connecting);
        }
    }

    light_connection_registry
}

pub struct InternalVolume {
    ws_min: Vec3,
    ws_max: Vec3,
    cubemaps_inside: Vec<Vec3>,
}

pub fn process_cubemaps(vmf: &VmfFile,) -> Vec<InternalVolume> {
    let cubemaps_origin: Vec<Vec3> = vmf.entities
        .iter()
        .filter(|ent| ent.classname().unwrap_or("") == "env_cubemap")
        .map(|ent| Vec3::parse(ent.get("origin").unwrap_or(&"0 0 0".to_string())))
        .collect();

    if cubemaps_origin.is_empty() {
        warn!("No env_cubemaps found on the map!");
        return Vec::new();
    } else {
        info!("Found {} env_cubemaps.", cubemaps_origin.len());
    }

    vmf.entities
        .iter()
        .filter(|ent| ent.classname().unwrap_or("") == "func_parallax_volume")
        .filter_map(|ent| {
            let aabb = geometry::get_entity_aabb(ent)?;
            let mut inside = Vec::new();
            for &c_pos in &cubemaps_origin {
                if c_pos[0] >= aabb.min[0] && c_pos[0] <= aabb.max[0] &&
                    c_pos[1] >= aabb.min[1] && c_pos[1] <= aabb.max[1] &&
                    c_pos[2] >= aabb.min[2] && c_pos[2] <= aabb.max[2] {
                    inside.push(c_pos);
                }
            }

            Some(InternalVolume {
                ws_min: aabb.min,
                ws_max: aabb.max,
                cubemaps_inside: inside,
            })
        })
        .collect()
}

/// Scoring & Light Selection
fn select_and_score_lights(
    all_lights: &[LightDef],
    bounds: &AABB,
    world_brushes: &[ConvexBrush],
    exclude_lights: &HashSet<String>,
    force_lights: &HashSet<String>,
    min_score: f32,
) -> (Vec<(LightDef, f32)>, Vec<(LightDef, f32)>) {
    let mut scored_lights: Vec<(usize, f32)> = Vec::new();

    for (idx, light) in all_lights.iter().enumerate() {
        // Check Exclude
        if light.is_named_light && exclude_lights.contains(&light.target_name) { // TODo: improve it! add additional fake-naming key
            debug!("  > Light '{}' (id: {}) manually excluded.", light.target_name, light.id);
            continue;
        }

        // Check Force
        if light.is_named_light && force_lights.contains(&light.target_name) { // TODo: improve it! add additional fake-naming key
            debug!("  > Light '{}' (id: {}) manually included.", light.target_name, light.id);
            scored_lights.push((idx, f32::MAX));
            continue;
        }

        let score = scoring::calculate_score(light, bounds, &world_brushes);
        if score > 0.0 {
            scored_lights.push((idx, score));
        }
    }

    // Normalization of scores
    let max_score = scored_lights.iter()
        .filter(|(_, s)| *s < f32::MAX) // Ignore forced lights
        .map(|(_, s)| *s)
        .fold(0.0, f32::max);
    if max_score > 0.0 {
        for (_, score) in scored_lights.iter_mut() {
            if *score < f32::MAX {
                *score /= max_score;
            }
        }
    }

    // Sort lights by score in descending order
    scored_lights.sort_by(|a, b| b.1.partial_cmp(&a.1).expect("NaN, its a bug"));

    let (mut accepted_candidates, mut rejected_candidates): (Vec<_>, Vec<_>) = scored_lights.into_iter()
        .partition(|(_, s)| *s >= f32::MAX || *s >= min_score);

    if accepted_candidates.len() > LUT_WIDTH {
        let overflow = accepted_candidates.split_off(LUT_WIDTH);
        rejected_candidates.extend(overflow);
    }

    // Stable sort to prefer named lights
    accepted_candidates.sort_by_key(|(idx, _)| !all_lights[*idx].is_named_light);

    let selected_lights: Vec<(LightDef, f32)> = accepted_candidates.into_iter()
        .map(|(idx, score)| (all_lights[idx].clone(), score))
        .collect();

    let rejected_lights: Vec<(LightDef, f32)> = rejected_candidates.into_iter()
        .map(|(idx, score)| (all_lights[idx].clone(), score))
        .collect();

    (selected_lights, rejected_lights)
}

#[derive(Debug)]
struct DynamicControllers {
    pub modify_control_entities: Vec<Entity>,
    pub backpatch_connections: HashMap<usize, Vec<(String, String)>>, // entity_id -> vec of connections
}

fn build_dynamic_controllers(
    selected_lights: &[(LightDef, f32)],
    cluster_name: &str,
    mat_name: &str,
    origin: &str,
    light_connection_registry: &LightConnectionRegistry,
) -> DynamicControllers {
    let mut modify_control_entities = Vec::new();
    let mut backpatch_connections: HashMap<usize, Vec<(String, String)>> = HashMap::new();

    for (i, (light, _score)) in selected_lights.iter().take(4).enumerate() {
        if !light.is_named_light {
            continue;
        }

        let lookup_key = light.id; // TODO!!!!!!!!! fix it
        let ctrl_name = format!("{}_ctrl_{}", cluster_name, i);

        let mut ctrl_ent = Entity::new("material_modify_control", 100_000);
        ctrl_ent.set("targetname".to_string(), ctrl_name.clone());
        ctrl_ent.set("parentname".to_string(), cluster_name.to_string());
        ctrl_ent.set("materialName".to_string(), mat_name.to_string());

        // Map Index to Variable ($c4_x, y, z, w)
        let var = match i {
            0 => "$c4_x",
            1 => "$c4_y",
            2 => "$c4_z",
            3 => "$c4_w",
            _ => unreachable!()
        };
        ctrl_ent.set("materialVar".to_string(), var.to_string());
        ctrl_ent.set("origin".to_string(), origin.to_string());

        modify_control_entities.push(ctrl_ent);

        if let Some(conns) = light_connection_registry.get(&lookup_key) {
            // Back-patching connections
            log::debug!("Back-patching connections for {}. {:?}", ctrl_name, conns);
            for conn in conns {
                let val = match conn.input_type {
                    LightInputType::TurnOn => "1",
                    LightInputType::TurnOff => "0",
                    // todo: SetPattern
                };
                let new_conn_str = format!("{},SetMaterialVar,{},{},-1", ctrl_name, val, conn.delay);

                backpatch_connections
                    .entry(conn.source_entity_id)
                    .or_default()
                    .push((conn.output_name.clone(), new_conn_str));
            }
        } else {
            log::debug!("lights for {} don't have inputs", ctrl_name);
        }
    }

    DynamicControllers { modify_control_entities, backpatch_connections }
}

impl LightCluster {
    pub fn from_ggx_surface(
        ggx_surface: &GgxSurfaceEnt,
        ggx_surface_name: &str,
        map_name: &str,
        game_dir: &Path,
        all_lights: &[LightDef],
        world_brushes: &[ConvexBrush],
        pcc_volumes: &[InternalVolume],
    ) -> Vec<LightCluster> {
        let mat_base_rel = Path::new("maps").join(map_name);
        let mat_output_dir = game_dir.join("materials").join(&mat_base_rel);

        debug!("Processing ggx_surface {:#?} id: {}", ggx_surface_name, ggx_surface.id);
        let build_cluster = |cluster_name: String, ggx_surface: &GgxSurfaceEnt, solids: Vec<Arc<RwLock<GgxSolid>>>, bound: AABB, normal: Vec3| -> LightCluster {
            let surface_material_path = mat_output_dir.join(&cluster_name);

            let (selected_lights, rejected_lights) = select_and_score_lights(
                all_lights,
                &bound,
                world_brushes,
                &ggx_surface.exclude_lights,
                &ggx_surface.force_lights,
                ggx_surface.min_score
            );

            if !selected_lights.is_empty() {
                debug!("  -> Selected Lights: {:?}", selected_lights.iter().map(|(v, _)| &v.id).collect::<Vec<_>>());
                if !rejected_lights.is_empty() {
                    debug!("  -> Rejected: {:?}", rejected_lights.iter().map(|(v, s)| format!("{} ({:.2})", v.id, s)).collect::<Vec<_>>());
                }
            } else {
                warn!("Surface '{}' (id: {}, pos: '{}') has no active lights.", cluster_name, ggx_surface.id, ggx_surface.origin);
            }

            // == Match PCC Volume
            let parallax_volume = LightCluster::find_parallax_volume(bound.center, normal, pcc_volumes);

            let cubemap_name = parallax_volume.as_ref().map(|pcc| {
                format!("maps/{}/c{}_{}_{}.hdr.vtf", map_name, pcc.cubemap_pos[0] as i32, pcc.cubemap_pos[1] as i32, pcc.cubemap_pos[2] as i32)
            });

            let mut initial_c4 =[1.0f32; 4];
            for (i, (light, _score)) in selected_lights.iter().take(4).enumerate() {
                if light.initially_dark {
                    initial_c4[i] = 0.0;
                }
            }

            LightCluster {
                solids,
                ggx_surface_name: ggx_surface_name.to_string(),
                ggx_surface_id: ggx_surface.id,
                ggx_surface_origin: Vec3::parse(&ggx_surface.origin),

                name: cluster_name.clone(),
                bound,
                pbr_material: ggx_surface.template_material.clone(),
                surface_material: cluster_name,
                surface_material_path,
                lights: selected_lights,
                initial_c4,
                rejected_lights,
                min_cluster_score: ggx_surface.min_score,
                pcc_volume: parallax_volume,
                cubemap_name,
            }
        };

        if ggx_surface.merge_solids && !ggx_surface.ggx_solids.is_empty() {
            let first_solid = ggx_surface.ggx_solids[0].read().unwrap();
            let normal = first_solid.surface_normal;

            let cluster_name = format!("{}_merged", ggx_surface_name);

            vec![build_cluster(cluster_name, ggx_surface, ggx_surface.ggx_solids.clone(), ggx_surface.bounding_box, normal)]
        } else {
            ggx_surface.ggx_solids.iter().enumerate().map(|(i, solid_arc)| {
                let solid = solid_arc.read().unwrap();
                let cluster_name = format!("{}_solid_{}", ggx_surface_name, i);

                build_cluster(cluster_name, ggx_surface, vec![solid_arc.clone()], solid.bound, solid.surface_normal)
            }).collect()
        }
    }
}
