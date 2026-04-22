use crate::math::Vec3;
use crate::processing::geometry::get_entity_aabb;
use crate::types::{BlockerDef, LightDef, LightType};
use std::collections::HashMap;
use vmf_forge::prelude::*;

const PBR_INTENSITY_MULT: f32 = 1.0;
const MAX_HDR_OVERBRIGHT: f32 = 16.0;

// Brightness threshold where light is considered "zero" for range calculation.
const LIGHT_CUTOFF_THRESHOLD: f32 = 0.2;

pub fn extract_lights(vmf: &VmfFile) -> anyhow::Result<Vec<LightDef>> {
    let mut lights = Vec::new();
    let mut entity_map: HashMap<String, usize> = HashMap::new();

    // Index entities for Blocker lookups
    for (idx, ent) in vmf.entities.iter().enumerate() {
        if let Some(name) = ent.targetname() {
            entity_map.insert(name.to_string(), idx);
        }
    }

    for i in 0..vmf.entities.len() {
        let ent = &vmf.entities[i];
        let classname = ent.classname().unwrap_or("");

        if classname == "light" || classname == "light_spot" || classname == "func_ggx_area" {
            // Skip disabled lights
            if classname != "func_ggx_area"
                && ent.get("pbr_enabled").map(|v| v.as_str()).unwrap_or("0") == "0"
            {
                log::debug!("skipping {} ({:?}) because pbr_enabled is 0 (class '{}')", ent.id(), ent.targetname(), classname);
                continue;
            }

            // == PHASE 1: BASIC PROPERTIES
            let origin_vec = Vec3::parse(ent.get("origin").unwrap_or(&"0 0 0".to_string()));
            let light_val = ent.get("_light").map(|v| v.as_str()).unwrap_or("255 255 255 200");
            let (mut color, raw_intensity_val) = parse_color_intensity(light_val);

            // Raw Intensity (normalized to 0..MAX_HDR_OVERBRIGHT range)
            let mut intensity = raw_intensity_val / MAX_HDR_OVERBRIGHT * PBR_INTENSITY_MULT;

            if let Some(scale) = ent.get("pbr_intensity_scale") {
                intensity *= scale.parse::<f32>().unwrap_or(1.0);
            }
            if let Some(col_str) = ent.get("pbr_color_override")
                && col_str != "-1 -1 -1" {
                    let (c, _) = parse_color_intensity(col_str);
                    color = c;
                }

            // == PHASE 2: PHYSICS & ATTENUATION
            let range_override = ent.get("pbr_range_override").and_then(|s| s.parse::<f32>().ok());
            let fifty_percent_val = ent.get("_fifty_percent_distance").and_then(|s| s.parse::<f32>().ok()).filter(|&v| v > 0.1);
            let mut final_pos = origin_vec;

            let mut shader_intensity;
            let shader_k;
            let mut range;
            let light_type;

            if classname == "func_ggx_area" {
                // Larger lights = Higher "Virtual Constant" = Softer falloff
                let dir = angles_to_dir(ent.get("angles").unwrap_or(&"0 0 0".to_string()), None);
                let mut width = 0.0;
                let mut height = 0.0;

                if let Some(aabb) = get_entity_aabb(ent) {
                    final_pos = aabb.center;
                    let extent = aabb.max - aabb.min; // Dimensions vector (dx, dy, dz)

                    // Reconstruct Shader Basis
                    let fwd = dir.normalize();
                    let up_base = if fwd[2].abs() > 0.99 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 0.0, 1.0) };
                    let right_vec = fwd.cross(up_base).normalize();
                    let up_vec = right_vec.cross(fwd).normalize();

                    // Project dimensions onto basis
                    let abs_right_vec = Vec3::new(right_vec[0].abs(), right_vec[1].abs(), right_vec[2].abs());
                    let abs_up_vec = Vec3::new(up_vec[0].abs(), up_vec[1].abs(), up_vec[2].abs());
                    width = extent.dot(abs_right_vec);
                    height = extent.dot(abs_up_vec).abs();

                    if width < 1.0 { width = 1.0; }
                    if height < 1.0 { height = 1.0; }
                }

                // Force standard quadratic falloff model for consistency with point lights.
                // This prevents the excessive range and "infinite" falloff behavior of the original area light formula.
                let c = 0.0;
                let l = 0.0;
                let q = 1.0;

                let ratio = c + (100.0 * l) + (10000.0 * q);
                let src_energy = if ratio > 0.001 { intensity * ratio } else { 0.0 };
                let math_c = 1.0;

                shader_intensity = src_energy / math_c;
                shader_k = q / math_c;

                // Normalize intensity to align with standard point light scoring.
                // A factor of 0.25 balances the visual brightness and ensures the light's importance score
                shader_intensity *= 0.25;

                // Solver for Range
                if shader_k > 1e-8 {
                    let val = (shader_intensity / LIGHT_CUTOFF_THRESHOLD - 1.0) / shader_k;
                    range = if val > 0.0 { val.sqrt() } else { 1000.0 };
                } else {
                    range = 10000.0;
                }

                let bidirectional = ent.get("pbr_bidirectional").map(|s| s == "1").unwrap_or(false);
                light_type = LightType::Rect {
                    direction: dir,
                    width,
                    height,
                    bidirectional,
                };
            } else { // POINT & SPOT lights
                let mut c = ent.get("_constant_attn").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                let l = ent.get("_linear_attn").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                let q = ent.get("_quadratic_attn").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);

                if let Some(dist50) = fifty_percent_val {
                    // Modern Falloff
                    shader_k = 1.0 / (dist50 * dist50);
                    shader_intensity = intensity; // Already correct for this mode
                    let dist0 = ent.get("_zero_percent_distance").and_then(|s| s.parse::<f32>().ok()).unwrap_or(dist50 * 5.0);
                    range = dist0;
                // } else {
                //     // Legacy Falloff
                //     if c < 0.0001 && l < 0.0001 && q < 0.0001 { c = 1.0; }

                //     let ratio = c + (100.0 * l) + (10000.0 * q);
                //     let src_energy = if ratio > 0.001 { intensity * ratio } else { 0.0 };

                //     let math_c = if c < 1.0 { 1.0 } else { c };

                //     shader_intensity = src_energy / math_c;
                //     shader_k = q / math_c;

                //     if shader_k > 1e-8 {
                //         let val = (shader_intensity / LIGHT_CUTOFF_THRESHOLD - 1.0) / shader_k;
                //         range = if val > 0.0 { val.sqrt() } else { 1000.0 };
                //     } else {
                //         range = 20000.0;
                //     }
                // }
                } else {
                    // --- LEGACY FALLOFF FIX (Short & Contrast) ---

                    shader_intensity = intensity * 0.75; // todo: СЕЙЧАС В ШЕЙДЕРЕ ЕСТЬ КОМПЕНСАЦИЯ НЕБОЛЬШАЯ DGX!!!!!!! ЛАВАШ СКОРРЕКТИРУЙ ПЖ ПОТОМ, НЕ ЗАБУДЬ - ДУРАК!

                    let target_d50 = 55.0;
                    let dist50_sq = target_d50 * target_d50; // 3025

                    // K ≈ 0.00033
                    let mut k = 1.0 / dist50_sq;

                    if l > 0.001 { k *= 0.2; }
                    if c > 0.001 { k *= 0.01; }

                    shader_k = k;

                    let cutoff_brightness = 0.02;
                    if shader_intensity > cutoff_brightness {
                        let val = (shader_intensity / cutoff_brightness - 1.0) / shader_k;
                        range = val.sqrt();
                    } else {
                        range = 100.0;
                    }

                    range = range.clamp(64.0, 4000.0);
                }

                // Shape & Direction
                if classname == "light_spot" {
                    let dir = angles_to_dir(
                        ent.get("angles").unwrap_or(&"0 0 0".to_string()),
                        ent.get("pitch").map(|s| s.as_str()),
                    );

                    let mut inner = ent.get("_inner_cone").and_then(|s| s.parse().ok()).unwrap_or(30.0);
                    let outer = ent.get("_cone").and_then(|s| s.parse().ok()).unwrap_or(45.0);
                    let spot_expo = ent.get("_exponent").and_then(|s| s.parse().ok()).unwrap_or(1.0);

                    // Clamp Inner <= Outer
                    if inner > outer { inner = outer; }

                    light_type = LightType::Spot {
                        direction: dir,
                        inner_angle: inner,
                        outer_angle: outer,
                        exponent: spot_expo,
                    };
                } else {
                    light_type = LightType::Point;
                }
            }

            // == PHASE 3: Final Common Overrides
            if let Some(r) = range_override
                && r > 0.1 { range = r; }
            range = range.clamp(64.0, 65000.0);

            // Blockers
            let process_blocker = |key: &str| -> Option<BlockerDef> {
                if let Some(name) = ent.get(key)
                    && let Some(&idx) = entity_map.get(name)
                        && let Some(aabb) = get_entity_aabb(&vmf.entities[idx]) {
                            let mut flag = 1; // TODO: move it to fgd!!
                            if let LightType::Rect { bidirectional: true, .. } = light_type {
                                flag = 2; // temp workaround
                            }
                            return Some(BlockerDef {
                                width: aabb.max[0] - aabb.min[0],
                                height: aabb.max[1] - aabb.min[1],
                                depth: aabb.max[2] - aabb.min[2],
                                pos: Some(aabb.center),
                                flag,
                            });
                        }
                None
            };

            let blockers = [
                process_blocker("pbr_blocker_name"),
                process_blocker("pbr_blocker_name_2"),
                // More can be added in the future .-.
            ];

            // == PHASE 4: FINALIZEE
            let spawnflags = ent.get("spawnflags").and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let initially_dark = (spawnflags & 1) != 0;
            let targetname = ent.targetname().unwrap_or_default().to_string();
            let pbr_name = format!("light_{}", ent.id()); // tODO: think, mark, think!


            lights.push(LightDef {
                id: ent.id(),
                target_name: targetname,
                pbr_name,
                is_named_light: ent.targetname().is_some(),
                light_type,
                pos: final_pos,
                color,
                intensity: shader_intensity,
                range,
                attenuation_k: shader_k,
                fifty_percent_distance: fifty_percent_val,
                blockers,
                initially_dark,
            });
        }
    }

    Ok(lights)
}

/// Helper: Clean VMF in-place
pub fn strip_pbr_entities(vmf: &mut VmfFile) {
    vmf.entities.retain(|ent| {
        let class = ent.classname().unwrap_or("").to_lowercase();
        !class.contains("func_ggx") && !class.eq_ignore_ascii_case("func_parallax_volume")
    });

}

/// Helper: Parse Source "_light" string
fn parse_color_intensity(s: &str) -> (Vec3, f32) {
    let parts: Vec<f32> = s.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    if parts.len() >= 4 {
        (
            Vec3::new(parts[0] / 255.0, parts[1] / 255.0, parts[2] / 255.0),
            parts[3],
        )
    } else if parts.len() == 3 {
        (
            Vec3::new(parts[0] / 255.0, parts[1] / 255.0, parts[2] / 255.0),
            200.0,
        )
    } else {
        (Vec3::ONE, 200.0)
    }
}

/// Helper: Convert Source angles to Vector
fn angles_to_dir(angles_str: &str, pitch_override: Option<&str>) -> Vec3 {
    let parts = Vec3::parse(angles_str);
    let mut pitch = parts[0];
    let yaw = parts[1];

    if let Some(p) = pitch_override {
        if let Ok(p_val) = p.parse::<f32>() {
            pitch = p_val;
        }
    } else {
        // Fix: In 'angles' KeyValue, -90 points UP in Hammer/Source logic for lights.
        // We invert it to match math expectation (where -90 is typically down/forward).
        // If 'pitch' key is used explicitly, it usually doesn't need inversion.
        pitch *= -1.0;
    }

    let p_rad = pitch.to_radians();
    let y_rad = yaw.to_radians();

    let x = p_rad.cos() * y_rad.cos();
    let y = p_rad.cos() * y_rad.sin();
    let z = p_rad.sin();

    let clean = |v: f32| if v.abs() < 1e-4 { 0.0 } else { v };
    Vec3::new(clean(x), clean(y), clean(z))
}
