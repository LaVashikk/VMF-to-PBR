const MAX_BLOCKERS: usize = 2;

#[derive(Debug, Clone)]
pub struct BlockerDef {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
    // Center position of the blocker in the world
    pub pos: Option<[f32; 3]>,
    pub flag: u8,
}

#[derive(Debug, Clone)]
pub enum LightType {
    Point,
    Spot {
        direction: [f32; 3],
        inner_angle: f32,
        outer_angle: f32,
        exponent: f32,
    },
    Rect {
        direction: [f32; 3],
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
    pub debug_id: String,
    pub is_named_light: bool,
    pub light_type: LightType,
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
    pub attenuation_k: f32,
    pub fifty_percent_distance: Option<f32>,
    pub blockers: [Option<BlockerDef>; MAX_BLOCKERS],
    /// If true, the light is turned off at map start (spawnflags & 1)
    pub initially_dark: bool,
}

/// Represents a collection of lights assigned to a specific surface/material.
/// This will be baked into a single Nx8 LUT texture.
#[derive(Debug)]
pub struct LightCluster {
    pub name: String,
    pub lights: Vec<(LightDef, f32)>,

    pub bounds: crate::math::AABB,
    pub min_cluster_score: f32,
    pub rejected_lights: Vec<(LightDef, f32)>,
}
