use crate::constants::TARGET_MATERIAL;
use crate::math::{AABB, Vec3};
use crate::types::{LightCluster, LightDef};
use super::cubemaps::{self, InternalVolume};
use super::geometry::{self, ConvexBrush};
use super::scoring::select_and_score_lights;
use crate::text::{calc_face_normal, parse_plane_points, sanitize_name};

use derive_more::{Deref, DerefMut};
use log::{debug, warn};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, RwLock};
use vmf_forge::prelude::{Entity, Solid};

pub const MAX_CUSTOM_SLOTS: usize = 4; // for force include/exclude

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
            let parallax_volume = cubemaps::find_parallax_volume(bound.center, normal, pcc_volumes);

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
