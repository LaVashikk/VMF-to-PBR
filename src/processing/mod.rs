use derive_more::{Deref, DerefMut};
use geometry::ConvexBrush;
use log::{debug, info, warn, error};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use vmf_forge::prelude::{Entity, VmfFile};
use crate::generator::{self, LUT_WIDTH, VmtParams};
use crate::math::{AABB, Vec3, add, dot, mul, normalize, parse_vector, sub};
use crate::parser::sanitize_name;
use crate::types::{LightCluster, LightDef, ParallaxVolume};
use utils::*;

pub mod geometry;
pub mod tracer;
pub mod scoring;
pub mod utils;

// Defines the material that identifies faces to be patched
const TARGET_MATERIAL: &str = "tools/toolspbr";
const GEOMETRY_OFFSET_UNITS: f32 = 0.975; // for offsets
const UV_SEARCH_DIST: f32 = 16.0;

const MAX_CUSTOM_SLOTS: usize = 4; // for force include/exclude

#[derive(Debug)]
pub struct LightConnection {
    source_entity_idx: usize, // todo: id
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
                    source_entity_idx: ent.id() as usize,
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

#[derive(Debug, Deref, DerefMut, PartialEq)]
pub struct GgxSurfaceEnt<'a> {
    #[deref]
    #[deref_mut]
    pub entity: &'a mut Entity,

    // ggx_surface custom parms
    pub template_material: String,
    pub override_roughness_mult: Option<f32>,
    pub override_base_reflectivity: Option<f32>,
    pub override_reflection_intensity: Option<f32>,
    pub override_normal_intensity: Option<f32>,
    pub override_fade_start: Option<f32>,
    pub override_fade_end: Option<f32>,

    pub surface_normal: Vec3,
    pub bounding_box: AABB,
    pub origin: String,
    pub surface_id: u64,
    pub exclude_lights: HashSet<String>,
    pub force_lights: HashSet<String>,
    pub min_score: f32,
}

impl<'a> GgxSurfaceEnt<'a> {
    pub fn new(entity: &'a mut Entity) -> Self {
        let surface_id = entity.id();

        // PBR material, which uses the corresponding shader
        let Some(template_material) = entity.get("template_material").cloned() else {
            log::warn!("No template_material for func_ggx_surface (hammer id: {}). It's required!", surface_id);
            panic!("Missing required key 'template_material' for func_ggx_surface");
        };

        // Calc surface normal
        let surface_normal = mul({
            entity.solids.as_deref().expect("unreachable").iter()
                .flat_map(|s| &s.sides)
                .find(|side| side.material.eq_ignore_ascii_case(TARGET_MATERIAL))
                .and_then(|side| parse_plane_points(&side.plane))
                .map(|points| calc_face_normal(points))
                .unwrap() // SAFETY: trust me!
        }, -1.0);

        // Calc bounding box
        let bounding_box = geometry::get_entity_aabb(entity).unwrap_or(AABB::new()); // WRONG! WRONG-WRONG-WRONG! One surface can have multiple solids! So process all of them!

        // Get surface origin. If not set, use AABB center
        let c = bounding_box.center;
        let origin = entity.get("origin").cloned().unwrap_or_else(|| format!("{:.2} {:.2} {:.2}", c[0], c[1], c[2]));

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
        // TODO: override parms

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

        // TODO: light_search_radius?

        Self {
            entity,

            template_material,
            override_roughness_mult,
            override_base_reflectivity,
            override_reflection_intensity,
            override_normal_intensity,
            override_fade_start,
            override_fade_end,

            surface_normal,
            origin,
            bounding_box,
            surface_id,
            exclude_lights,
            force_lights,
            min_score
        }
    }

    pub fn convert_to_illusionary(&mut self) {
        self.entity.set("classname".to_string(), "func_illusionary".to_string());
        self.entity.set("renderamt".to_string(), "200".to_string());
        self.entity.set("rendermode".to_string(), "2".to_string());
        self.entity.set("pbr_workaround_shit".to_string(), "0".to_string()); // TODO: temp dev-stuff
    }
}

impl LightCluster<'_> {
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
                            let to_cubemap = sub(c, origin);
                            let dist_sq = dot(to_cubemap, to_cubemap);
                            let mut score = -dist_sq;

                            let dir_to_cubemap = normalize(to_cubemap);
                            let facing = dot(surface_normal, dir_to_cubemap);

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

pub fn process_map_pipeline<'a>(
    vmf: &'a mut VmfFile,
    all_lights: &[LightDef],
    game_dir: &Path,
    map_name: &str,
    is_draft_run: bool
) -> anyhow::Result<Vec<LightCluster<'a>>> {
    let mat_base_rel = Path::new("maps").join(map_name);
    let mat_output_dir = game_dir.join("materials").join(&mat_base_rel);

    // == Pre-pass
    let world_brushes = build_collision_world(vmf);

    let light_connection_registry = build_connections_registry(vmf);
    info!("Registry built. Tracked targets: {}", light_connection_registry.len()); // todo: made it debug?

    let pcc_volumes = process_cubemaps(vmf);
    info!("Found {} PCC volumes.", pcc_volumes.len());

    // == Processing func_ggx_surface
    info!("Processing 'func_ggx_surface' entities...");
    let mut new_entities: Vec<Entity> = Vec::new();

    let mut clusters: Vec<LightCluster> = vmf.entities
        .iter_mut()
        .filter(|e| e.classname().unwrap_or("") == "func_ggx_surface")
        .enumerate()
        .map(|(surface_counter, e)| { // todo: rayon
            // Entity Setup
            let mut ggx_surface = GgxSurfaceEnt::new(e);
            ggx_surface.convert_to_illusionary();

            let cluster_name = if let Some(name) = ggx_surface.targetname() {
                name.to_string()
            } else {
                let new_name = format!("surface_{}", surface_counter);
                ggx_surface.set("targetname".to_string(), new_name.clone());
                new_name
            };


            debug!("Processing surface: {} (hammer id: {})", cluster_name, ggx_surface.surface_id);

            // TODO: PROCESS ALL SOLIDS!

            // == Scoring & Light Selection

            let mut scored_lights: Vec<(usize, f32)> = Vec::new();
            for (idx, light) in all_lights.iter().enumerate() {
                // Check Exclude
                if light.is_named_light && ggx_surface.exclude_lights.contains(&light.debug_id) { // TODo: improve it! add additional fake-naming key
                    debug!("  > Light '{}' manually excluded.", light.debug_id);
                    continue;
                }

                // Check Force
                if light.is_named_light && ggx_surface.force_lights.contains(&light.debug_id) {
                    debug!("  > Light '{}' manually included.", light.debug_id);
                    scored_lights.push((idx, f32::MAX));
                    continue;
                }

                let score = scoring::calculate_score(light, &ggx_surface.bounding_box, &world_brushes);
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
                .partition(|(_, s)| *s >= f32::MAX || *s >= ggx_surface.min_score);

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

            if selected_lights.is_empty() {
                warn!("Surface '{}' (id: {}, pos: '{}') has no active lights.", cluster_name, surface_id, origin.as_deref().unwrap_or_default());
                // continue; // TODO: process it as additional arg
            } else {
                info!("Surface '{}' (id: {}) -> assigned {} lights. (Rejected: {})", cluster_name, surface_id, selected_lights.len(), rejected_lights.len());
                debug!("  -> Selected Lights: {:?}", selected_lights.iter().map(|(v, _)| &v.debug_id).collect::<Vec<_>>());
                if !rejected_lights.is_empty() {
                        debug!("  -> Rejected: {:?}", rejected_lights.iter().map(|(v, s)| format!("{} ({:.2})", v.debug_id, s)).collect::<Vec<_>>());
                }
            }

            // ==  Dynamic Light Handling
            let mut initial_c4 = [1.0f32; 4];
            for (i, (light, _score)) in selected_lights.iter().take(4).enumerate() {
                if light.initially_dark {
                    initial_c4[i] = 0.0;
                }

                if light.is_named_light {
                    let lookup_key = light.debug_id.trim().to_lowercase(); // TODO!!!!!!!!! fix it
                    let ctrl_name = format!("{}_ctrl_{}", cluster_name, i);
                    let p = mat_base_rel.join(&cluster_name);
                    let mat_name = p.to_string_lossy().replace('\\', "/");

                    let id = 9999888 + surface_counter;
                    let mut ctrl_ent = Entity::new("material_modify_control", id as u64);
                    ctrl_ent.set("targetname".to_string(), ctrl_name.clone());
                    ctrl_ent.set("parentname".to_string(), cluster_name.clone());
                    ctrl_ent.set("materialName".to_string(), mat_name);

                    // Map Index to Variable ($c4_x, y, z, w)
                    let var = match i {
                        0 => "$c4_x",
                        1 => "$c4_y",
                        2 => "$c4_z",
                        3 => "$c4_w",
                        _ => unreachable!()
                    };
                    ctrl_ent.set("materialVar".to_string(), var.to_string());
                    ctrl_ent.set("origin".to_string(), ggx_surface.origin.clone());

                    new_entities.push(ctrl_ent);

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

                            if let Some(c_vec) = &mut ggx_surface.connections {
                                c_vec.push((conn.output_name, new_conn_str));
                            } else {
                                ggx_surface.connections = Some(vec![(conn.output_name, new_conn_str)]);
                            }
                        }
                    } else {
                        log::debug!("lights for {} don't have inputs", ctrl_name);
                    }
                }
            }

            // == Match PCC Volume
            let parallax_volume = LightCluster::find_parallax_volume(
                ggx_surface.bounding_box.center,
                ggx_surface.surface_normal,
                &pcc_volumes
            );

            let cubemap_name = if let Some(pcc) = parallax_volume {
                let ox = pcc.cubemap_pos[0] as i32;
                let oy = pcc.cubemap_pos[1] as i32;
                let oz = pcc.cubemap_pos[2] as i32;
                Some(format!("maps/{}/c{}_{}_{}.hdr.vtf", map_name, ox, oy, oz))
            } else {
                None
            };

            LightCluster { // todo! maybe paste ggx_surface here?!
                name: cluster_name,
                entity: ggx_surface,
                material: ggx_surface.template_material.clone(), // or take
                bounds: ggx_surface.bounding_box,
                lights: selected_lights,
                rejected_lights,
                min_cluster_score: min_score,
                pcc_volume: parallax_volume,
                cubemap_name: cubemap_name.clone(),
            }

            // TODO: end here?
        })
        .collect();



    // == Generate Assets
    if !is_draft_run {
        let lut_filename = format!("{}_lut", cluster_name);
        let vtf_path = mat_output_dir.join(format!("{}.vtf", lut_filename));
        let vmt_path = mat_output_dir.join(format!("{}.vmt", cluster_name));

        // todo: cache template_materials and orig_vmt!
        let mut orig_vmt = generator::find_and_process_vmt(game_dir, template_material.as_deref()).unwrap_or_else(|m| {
            error!("Failed to process VMT: {}", m);
            VmtParams::default()
        });

        orig_vmt.num_lights = cluster.lights.len() as f32;
        // dbg!(&orig_vmt);

        // todo: that's fucking bullshit!
        if let Err(e) = generator::generate_vtf(&cluster, &vtf_path, orig_vmt) {
            error!("Failed to create VTF for {}: {}", cluster_name, e);
        }

        let vtf_rel_path = mat_base_rel.join(&lut_filename);
        let vtf_rel_str = vtf_rel_path.to_string_lossy();
        generator::generate_vmt(
            &vmt_path,
            &vtf_rel_str,
            template_material.as_deref(),
            initial_c4,
            surface_id,
            cubemap_name.as_deref(),
        )?;
    } else {
        // it's draft, no need change geometry
        return Ok(clusters);
    }

    // == Update Solids Material
    let patch_material_path = mat_base_rel.join(&cluster_name);
    let patch_material_str = patch_material_path.to_string_lossy().replace('\\', "/");

    // Shifting geometry & UV Fix
    if let Some(solids) = &mut ent.solids {
        let mut material_updated = false;
        let origin_vec = if let Some(o_str) = origin {
            crate::math::parse_vector(&o_str)
        } else {
            surface_aabb.center
        };

        for solid in solids {
            let mut calculated_offset = None;
            let mut parent_uv: Option<(String, String)> = None;

            // Calculate offset based on the "toolspbr" face normal
            for side in &solid.sides {
                if side.material.eq_ignore_ascii_case(TARGET_MATERIAL)
                    && let Some(points) = parse_plane_points(&side.plane) {
                        // let normal = calc_face_normal(points); // todo!!!!!!!!!!!
                        let max_axis = normal[0].abs().max(normal[1].abs()).max(normal[2].abs());
                        calculated_offset = Some(mul(normal, GEOMETRY_OFFSET_UNITS * max_axis));

                        // UV Fix Logic
                        let start = add(origin_vec, mul(normal, 5.)); // todo
                        debug!("  [UV Fix] Casting ray from {:?} dir {:?} (dist: {})", start, normal, UV_SEARCH_DIST);
                        if let Some(hit) = tracer::trace_ray_closest(start, normal, UV_SEARCH_DIST, &world_brushes) {
                            debug!("    -> Hit world brush at dist {:.2} (brush id: {}). Copying UVs ({} | {}).", hit.t, hit.id, hit.u_axis, hit.v_axis);
                            parent_uv = Some((hit.u_axis.to_string(), hit.v_axis.to_string()));
                        } else {
                            debug!("    -> No parent surface found within range.");
                        }

                        break;
                    }
            }

            for side in &mut solid.sides {
                // Apply offset if calculated
                if let Some(offset) = calculated_offset {
                    debug!("  [Geometry] Shifting solid {} by vector {:?}", solid.id, offset);
                    side.plane = apply_offset_to_plane(&side.plane, offset);
                }

                // Update material
                if side.material.eq_ignore_ascii_case(TARGET_MATERIAL) {
                    if let Some((ref u, ref v)) = parent_uv {
                        side.u_axis = u.clone();
                        side.v_axis = v.clone();
                    } else {
                        warn!("No parent_uv for {} (hammer id: {})", cluster_name, surface_id); // todo: improve msg
                    }
                    side.material = patch_material_str.clone();
                    material_updated = true;
                }
            }
        }
        if !material_updated {
            warn!("The cluster with id {} has no {} texture. PBS will be skipped!", surface_id, TARGET_MATERIAL);
        }
    }

    vmf.entities.extend(new_entities);

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

pub type LightConnectionRegistry = HashMap<String, Vec<LightConnection>>;

pub fn build_connections_registry(vmf: &VmfFile,) -> LightConnectionRegistry {
    let mut light_connection_registry: LightConnectionRegistry = HashMap::new();
    for ent in vmf.entities.iter() {
        if let Some(light_connecting) = LightConnection::parse(ent) {
            let key = light_connecting.target_name.clone(); // todo: clone here
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
        .map(|ent| parse_vector(ent.get("origin").unwrap_or(&"0 0 0".to_string())))
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
