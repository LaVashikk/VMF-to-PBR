use crate::math::{Vec3, AABB};
use crate::types::{LightCluster, LightDef, LightType};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use serde::{Serialize, Serializer};
use serde_json::Value;

const SANITIZER_FUNC: &str = "::SanitizeName <- function(name) {
    local parts = split(name, \"-. \")
    local result = \"_\"
    foreach(part in parts) result += part
    return result
}";

#[derive(Clone)]
struct Vector(f32, f32, f32);

impl Serialize for Vector {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = format!("__VECTOR__({}, {}, {})", self.0, self.1, self.2);
        serializer.serialize_str(&s)
    }
}
impl From<Vec3> for Vector {
    fn from(v: Vec3) -> Self {
        Vector(v[0], v[1], v[2])
    }
}

#[derive(Serialize)]
struct LightAssociation {
    surface: String,
    rank: usize,
    score: f32,
}

#[derive(Serialize)]
struct PbrSurface {
    id: String,
    min_score: f32,
    center: Vector,
    mins: Vector,
    maxs: Vector,
    material: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cubemap: Option<Vector>,
    lights: Vec<String>,
    rejected: Vec<String>,
}

#[derive(Serialize)]
struct PbrBlocker { // toodo
    pos: Vector,
    mins: Vector,
    maxs: Vector,
}

#[derive(Serialize)]
struct PbrLight {
    pos: Vector,
    #[serde(skip_serializing_if = "Option::is_none")]
    dir: Option<Vector>,
    color: Vector,
    intensity: f32,
    range: f32,
    #[serde(rename = "dist50", skip_serializing_if = "Option::is_none")]
    dist50: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    blockers: Vec<PbrBlocker>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    associations: Vec<LightAssociation>,
    meta: String,
}

#[derive(Serialize)]
struct PbrData {
    surfaces: Vec<PbrSurface>,
    lights: HashMap<String, PbrLight>,
}

pub fn generate_nut(
    path: &Path,
    clusters: &[LightCluster],
    all_lights: &[LightDef],
) -> io::Result<()> {

    // Collect light associations
    let mut light_associations: HashMap<String, Vec<LightAssociation>> = HashMap::new();
    for cluster in clusters {
        for (rank, (light, score)) in cluster.lights.iter().enumerate() {
            light_associations
                .entry(light.debug_id.clone())
                .or_default()
                .push(LightAssociation {
                    surface: cluster.name.clone(),
                    rank,
                    score: *score,
                });
        }
    }

    // Surfaces generation
    let surfaces: Vec<PbrSurface> = clusters
        .iter()
        .map(|cluster| {
            let (center, mins, maxs) = calculate_extent(&cluster.bounds);

            PbrSurface {
                id: cluster.name.clone(),
                min_score: cluster.min_cluster_score,
                center: center.into(),
                mins: mins.into(),
                maxs: maxs.into(),
                material: cluster.material.clone(),
                cubemap: cluster.pcc_volume.as_ref().map(|p| p.cubemap_pos.into()),
                lights: cluster.lights.iter().map(|(l, _)| format!("_{}", l.debug_id)).collect(),
                rejected: cluster.rejected_lights.iter().map(|(l, _)| format!("_{}", l.debug_id)).collect(),
            }
        })
        .collect();

    // Lights generation
    let mut lights_map = HashMap::new();
    for light in all_lights {
        let dir_vec = match light.light_type {
            LightType::Point => None,
            LightType::Spot { direction, .. } | LightType::Rect { direction, .. } => Some(direction.into()),
        };

        let color = Vector(
            (light.color[0] * 255.0).round(),
            (light.color[1] * 255.0).round(),
            (light.color[2] * 255.0).round(),
        );

        let mut blockers = Vec::new();
        for blocker in light.blockers.iter().flatten() {
            let b_pos = blocker.pos.unwrap_or(light.pos);
            let half_w = blocker.width * 0.5;
            let half_h = blocker.height * 0.5;
            let half_d = blocker.depth * 0.5;

            blockers.push(PbrBlocker {
                pos: b_pos.into(),
                mins: Vector(-half_w, -half_h, -half_d),
                maxs: Vector(half_w, half_h, half_d),
            });
        }

        let associations = light_associations
            .remove(&light.debug_id)
            .unwrap_or_default();

        let pbr_light = PbrLight {
            pos: light.pos.into(),
            dir: dir_vec,
            color,
            intensity: light.intensity,
            range: light.range,
            dist50: light.fifty_percent_distance,
            blockers,
            associations,
            meta: generate_meta(light),
        };

        let dict_key = format!("_{}", light.debug_id.replace(".", "_"));
        lights_map.insert(dict_key, pbr_light);
    }

    // Serialize to Squirrel
    let pbr_data = PbrData {
        surfaces,
        lights: lights_map
    };
    let json_ast = serde_json::to_value(&pbr_data).expect("Failed to serialize to JSON AST");
    let squirrel_code = crate::nut_writer::value_to_squirrel(&json_ast, 0);

    // And save to file
    let mut file = File::create(path)?;
    writeln!(file, "{}", SANITIZER_FUNC)?;
    writeln!(file, "::PBR_DATA <- {}", squirrel_code)?;

    Ok(())
}

/// Returns (center, mins, maxs) where mins/maxs are relative extents
fn calculate_extent(aabb: &AABB) -> (Vec3, Vec3, Vec3) {
    let center = aabb.center;
    let half_ext_x = (aabb.max[0] - aabb.min[0]) * 0.5;
    let half_ext_y = (aabb.max[1] - aabb.min[1]) * 0.5;
    let half_ext_z = (aabb.max[2] - aabb.min[2]) * 0.5;

    let maxs = [half_ext_x, half_ext_y, half_ext_z];
    let mins = [-half_ext_x, -half_ext_y, -half_ext_z];

    (center, mins, maxs)
}

fn generate_meta(light: &LightDef) -> String {
    let type_str = match light.light_type {
        LightType::Point => "Point".to_string(),
        LightType::Spot { .. } => "Spot".to_string(),
        LightType::Rect { width, height, .. } => format!("Rect | Size: {}x{}", width, height),
    };

    // Note: 'Shadow' status is not explicitly stored in LightDef in current parser,
    // so we omit it to avoid incorrect data.
    format!("Type: {} | Atten_K: {}", type_str, light.attenuation_k)
}
