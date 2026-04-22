use log::{info, warn};
use vmf_forge::prelude::VmfFile;
use crate::math::Vec3;
use super::geometry;
use crate::types::ParallaxVolume;


pub fn find_parallax_volume(origin: Vec3, surface_normal: Vec3, pcc_volumes: &[InternalVolume]) -> Option<ParallaxVolume> {
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
