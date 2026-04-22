use std::{path::PathBuf, sync::{Arc, RwLock}};
use crate::math::{AABB, Vec3};
use crate::processing::surface_wrappers::GgxSolid;

const MAX_BLOCKERS: usize = 2;

#[derive(Debug, Clone)]
pub struct BlockerDef {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
    // Center position of the blocker in the world
    pub pos: Option<Vec3>,
    pub flag: u8,
}

#[derive(Debug, Clone)]
pub enum LightType {
    Point,
    Spot {
        direction: Vec3,
        inner_angle: f32,
        outer_angle: f32,
        exponent: f32,
    },
    Rect {
        direction: Vec3,
        width: f32,
        height: f32,
        bidirectional: bool,
    },
}

impl LightType {
    pub fn name(&self) -> &'static str {
        match self {
            LightType::Point => "Point",
            LightType::Spot { .. } => "Spot",
            LightType::Rect { .. } => "Area",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LightDef {
    // TODO: should I save Entity itself?
    pub id: u64,
    pub target_name: String,
    pub pbr_name: String,
    pub is_named_light: bool,

    pub light_type: LightType,
    pub pos: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
    pub attenuation_k: f32,
    pub fifty_percent_distance: Option<f32>,
    pub blockers: [Option<BlockerDef>; MAX_BLOCKERS],

    /// If true, the light is turned off at map start (spawnflags & 1)
    pub initially_dark: bool,
}

#[derive(Debug, Clone)]
pub struct ParallaxVolume {
    pub cubemap_pos: Vec3, // World space position of the selected env_cubemap
    pub ws_min: Vec3,      // World space AABB Min of the volume
    pub ws_max: Vec3,      // World space AABB Max of the volume
}

/// Represents a collection of lights assigned to a specific surface/material.
/// This will be baked into a single Nx16 LUT texture.
#[derive(Debug)]
pub struct LightCluster {
    pub solids: Vec<Arc<RwLock<GgxSolid>>>,
    pub ggx_surface_name: String,
    pub ggx_surface_id: u64,
    pub ggx_surface_origin: Vec3,

    pub name: String,
    pub bound: AABB,
    pub lights: Vec<(LightDef, f32)>,
    // Initial values for register c4, controlling brightness of the first 4 toggleable named lights
    pub initial_c4: [f32; 4],

    pub pbr_material: String,
    pub surface_material: String,
    pub surface_material_path: PathBuf,

    pub min_cluster_score: f32,
    pub rejected_lights: Vec<(LightDef, f32)>,

    pub pcc_volume: Option<ParallaxVolume>,
    pub cubemap_name: Option<String>,
}

impl LightCluster {
    pub fn dump(&self) {
        println!("---\nCluster: '{}'", self.name);
        println!("   Coordinated: {}", self.bound.center);
        println!("   PBR Material: {:?}", self.pbr_material);
        println!("   LUT Data Material: {:?}", self.surface_material_path.display());
        println!("   GGX_SURFACE entity: {:?} (hammer id: {})", self.ggx_surface_name, self.ggx_surface_id);
        println!("   Min Score Threshold: {:.4}", self.min_cluster_score);
        println!("   Cubemap Name: {:?}", self.cubemap_name.as_deref().unwrap_or("None"));

        println!("   [ACCEPTED LIGHTS] (Count: {})", self.lights.len());
        for (light, score) in &self.lights {
            let score_str = if *score > 10000.0 {
                "FORCE".to_string()
            } else {
                format!("{:.4}", score)
            };

            println!("     + {:<25} | Score: {}", light.id, score_str);
        }

        if !self.rejected_lights.is_empty() {
            println!("   [REJECTED LIGHTS] (Count: {})", self.rejected_lights.len());
            for (light, score) in &self.rejected_lights {
                println!("     - {:<25} | Score: {:.4}", light.id, score);
            }
        } else {
            println!("   [REJECTED LIGHTS] (None)");
        }
    }
}
