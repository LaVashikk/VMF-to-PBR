use geometry::ConvexBrush;
use log::{debug, info, warn, error};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use vmf_forge::prelude::{Entity, VmfFile};
use crate::generator::{self, LUT_WIDTH, VmtParams};
use crate::math::{AABB, Vec3, add, dot, mul, normalize, parse_vector, sub};
use crate::parser::sanitize_name;
use crate::types::{LightCluster, LightDef};
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
struct LightConnection {
    source_entity_idx: usize,
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


pub fn process_map_pipeline(
    vmf: &mut VmfFile,
    all_lights: &[LightDef],
    game_dir: &Path,
    map_name: &str,
    is_draft_run: bool
) -> anyhow::Result<Vec<LightCluster>> {
    let world_brushes = build_collision_world(vmf);
    let mut clusters = Vec::new();
    let mat_base_rel = Path::new("maps").join(map_name);
    let mat_output_dir = game_dir.join("materials").join(&mat_base_rel);

    // == Connection Registry (Pre-pass)
    let mut light_connection_registry: HashMap<String, Vec<LightConnection>> = HashMap::new();
    for (idx, ent) in vmf.entities.iter().enumerate() {
        if let Some(connections) = &ent.connections {
            for (output, value) in connections {
                // Parse VMF connection string: "TargetEntity;Input;Param;Delay;Limit"
                let parts: Vec<&str> = value.split([',', '\x1B']).collect();
                let target = parts[0].trim();
                let input = parts[1].trim();
                let delay = parts.get(3).and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(0.0);

                let input_type = match input.to_lowercase().as_str() {
                    "turnon" => Some(LightInputType::TurnOn),
                    "turnoff" => Some(LightInputType::TurnOff),
                    // todo
                    _ => None
                };

                if let Some(it) = input_type {
                    // if target is "!self" -> then use "ent.targetname()"
                    let maybe_key = if target.eq_ignore_ascii_case("!self") {
                        ent.targetname()
                    } else {
                        Some(target)
                    };
                    let Some(raw_key) = maybe_key else { continue };
                    let key = raw_key.to_lowercase();

                    debug!("  Found: Ent[{}] {} -> {}.{:?} (Delay: {})",
                            idx, output, key, it, delay);

                    light_connection_registry
                        .entry(key)
                        .or_default()
                        .push(LightConnection {
                            source_entity_idx: idx,
                            output_name: output.clone(),
                            input_type: it,
                            delay,
                        });
                }
            }
        }
    }

    info!("Registry built. Tracked targets: {}", light_connection_registry.len());

    // == Processing env_cubemap
    info!("Processing 'env_cubemap' entities...");
    let mut cubemaps: Vec<Vec3> = Vec::new();
    for ent in vmf.entities.iter() {
        if ent.classname().unwrap_or("") == "env_cubemap" {
            let origin = parse_vector(ent.get("origin").unwrap_or(&"0 0 0".to_string()));
            cubemaps.push(origin);
        }
    }
    info!("Found {} env_cubemaps.", cubemaps.len());

    // == Processing func_parallax_volume
    info!("Processing 'func_parallax_volume' entities...");
    struct InternalVolume {
        ws_min: Vec3,
        ws_max: Vec3,
        cubemaps_inside: Vec<Vec3>,
    }
    let mut pcc_volumes: Vec<InternalVolume> = Vec::new();
    let mut pcc_ents_to_remove = Vec::new();

    for (idx, ent) in vmf.entities.iter().enumerate() {
        if ent.classname().unwrap_or("") == "func_parallax_volume" {
            if let Some(aabb) = geometry::get_entity_aabb(ent) {
                let mut inside = Vec::new();
                for &c_pos in &cubemaps {
                    if c_pos[0] >= aabb.min[0] && c_pos[0] <= aabb.max[0] &&
                       c_pos[1] >= aabb.min[1] && c_pos[1] <= aabb.max[1] &&
                       c_pos[2] >= aabb.min[2] && c_pos[2] <= aabb.max[2] {
                        inside.push(c_pos);
                    }
                }

                pcc_volumes.push(InternalVolume {
                    ws_min: aabb.min,
                    ws_max: aabb.max,
                    cubemaps_inside: inside,
                });
                pcc_ents_to_remove.push(idx);
            }
        }
    }
    info!("Found {} PCC volumes.", pcc_volumes.len());

    // == Processing func_ggx_surface
    info!("Processing 'func_ggx_surface' entities...");
    let mut new_entities: Vec<Entity> = Vec::new();
    let mut new_connections: HashMap<usize, Vec<(String, String)>> = HashMap::new();
    let mut surface_counter = 0;

    for ent in vmf.entities.iter_mut() { // todo: the execution time can be improved with 'rayon'
        if ent.classname().unwrap_or("") != "func_ggx_surface" {
            continue;
        }
        surface_counter += 1;

        // Entity Setup
        ent.set("classname".to_string(), "func_illusionary".to_string());
        ent.set("renderamt".to_string(), "200".to_string());
        ent.set("rendermode".to_string(), "2".to_string());
        ent.set("pbr_workaround_shit".to_string(), "0".to_string());

        let template_material = ent.get("template_material").cloned();
        let origin = ent.get("origin").cloned();
        let surface_id = ent.id();
        let cluster_name = if let Some(name) = ent.targetname() {
            name.to_string()
        } else {
            let new_name = format!("surface_{}", surface_counter);
            ent.set("targetname".to_string(), new_name.clone());
            new_name
        };

        // TODO: PROCESS ALL SOLIDS!

        // == Scoring & Light Selection
        debug!("Processing surface: {} (hammer id: {})", cluster_name, surface_id);
        let surface_aabb = geometry::get_entity_aabb(ent).unwrap_or(AABB::new());

        let mut exclude_lights: HashSet<String> = HashSet::new();
        let mut force_lights: HashSet<String> = HashSet::new();

        for i in 1..=MAX_CUSTOM_SLOTS {
            if let Some(name) = ent.get(&format!("exclude_light_{}", i))
                && !name.is_empty() {
                    exclude_lights.insert(sanitize_name(name));
                }
            if let Some(name) = ent.get(&format!("force_light_{}", i))
                && !name.is_empty() {
                    force_lights.insert(sanitize_name(name));
                }
        }

        let mut scored_lights: Vec<(usize, f32)> = Vec::new();
        for (idx, light) in all_lights.iter().enumerate() {
            // Check Exclude
            if light.is_named_light && exclude_lights.contains(&light.debug_id) { // TODo: improve it! add additional fake-naming key
                debug!("  > Light '{}' manually excluded.", light.debug_id);
                continue;
            }

            // Check Force
            if light.is_named_light && force_lights.contains(&light.debug_id) {
                debug!("  > Light '{}' manually included.", light.debug_id);
                scored_lights.push((idx, f32::MAX));
                continue;
            }

            let score = scoring::calculate_score(light, &surface_aabb, &world_brushes);
            if score > 0.0 {
                scored_lights.push((idx, score));
            }
        }

        // Normalization
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
        scored_lights.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let min_score = ent.get("min_score").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.10);

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
                let lookup_key = light.debug_id.trim().to_lowercase();
                let ctrl_name = format!("{}_ctrl_{}", cluster_name, i);
                let p = mat_base_rel.join(&cluster_name);
                let mat_name = p.to_string_lossy().replace('\\', "/");

                let mut ctrl_ent = Entity::new("material_modify_control", 9999899 + surface_counter);
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
                let center = surface_aabb.center;
                ctrl_ent.set("origin".to_string(), format!("{} {} {}", center[0], center[1], center[2]));

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

                        new_connections.entry(conn.source_entity_idx)
                            .or_default()
                            .push((conn.output_name.clone(), new_conn_str));
                    }
                } else {
                    log::debug!("lights for {} don't have inputs", ctrl_name);
                }
            }
        }

        // == Match PCC Volume
        let mut best_pcc_data = None;
        let center = surface_aabb.center;

        let surface_normal = mul({ // todo: DRY dude. But im lazy :<
            ent.solids.as_deref().expect("unreachable behaivor!!").iter()
                .flat_map(|s| &s.sides)
                .find(|side| side.material.eq_ignore_ascii_case(TARGET_MATERIAL))
                .and_then(|side| parse_plane_points(&side.plane))
                .map(|points| calc_face_normal(points))
                .unwrap() // SAFETY: trust me!
        }, -1.0);

        // todo: made better way to find closest cubemap
        for vol in &pcc_volumes {
            // if center[0] >= vol.ws_min[0] && center[0] <= vol.ws_max[0] &&
            //    center[1] >= vol.ws_min[1] && center[1] <= vol.ws_max[1] &&
            //    center[2] >= vol.ws_min[2] && center[2] <= vol.ws_max[2]
            // {
                    if !vol.cubemaps_inside.is_empty() {
                        let mut best_score = f32::MIN;
                        let mut best_c = vol.cubemaps_inside[0];

                        for &c in &vol.cubemaps_inside {
                            let to_cubemap = sub(c, center);
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

        let cubemap_name = if let Some(pcc) = &best_pcc_data {
            let ox = pcc.cubemap_pos[0] as i32;
            let oy = pcc.cubemap_pos[1] as i32;
            let oz = pcc.cubemap_pos[2] as i32;
            Some(format!("maps/{}/c{}_{}_{}.hdr.vtf", map_name, ox, oy, oz))
        } else {
            None
        };

        let cluster = LightCluster {
            name: cluster_name.clone(),
            material: template_material.clone().unwrap_or_default(),
            bounds: surface_aabb,
            lights: selected_lights,
            rejected_lights,
            min_cluster_score: min_score,
            pcc_volume: best_pcc_data,
            cubemap_name: cubemap_name.clone(),
        };

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
            clusters.push(cluster);
            continue;
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
                            let normal = calc_face_normal(points);
                            let max_axis = normal[0].abs().max(normal[1].abs()).max(normal[2].abs());
                            calculated_offset = Some(mul(normal, GEOMETRY_OFFSET_UNITS * max_axis));

                            // UV Fix Logic
                            let start = add(origin_vec, mul(normal, -5.)); // todo
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

        clusters.push(cluster);
    }

    vmf.entities.0.extend(new_entities);

    // Append new connections to existing entities
    for (idx, conns) in new_connections {
        if let Some(ent) = vmf.entities.0.get_mut(idx) {
            for (output, value) in conns {
                if let Some(c_vec) = &mut ent.connections {
                    c_vec.push((output, value));
                } else {
                    ent.connections = Some(vec![(output, value)]);
                }
            }
        }
    }


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
