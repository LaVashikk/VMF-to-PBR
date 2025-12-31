use crate::math::{Vec3, AABB};
use crate::types::{LightCluster, LightDef, LightType};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

const SANITIZER_FUNC: &str = "::SanitizeName <- function(name) {
    local parts = split(name, \"-. \")
    local result = \"_\"
    foreach(part in parts) result += part
    return result
}";

struct LightAssociation {
    surface: String,
    rank: usize,
    score: f32,
}

pub fn generate_nut(
    path: &Path,
    clusters: &[LightCluster],
    all_lights: &[LightDef],
) -> io::Result<()> {
    let mut file = File::create(path)?;

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

    writeln!(file, "{}", SANITIZER_FUNC)?;
    writeln!(file, "::PBR_DATA <- {{")?;

    // == Generate Surfaces
    writeln!(file, "\tsurfaces = [")?;
    for (i, cluster) in clusters.iter().enumerate() {
        let (center, mins, maxs) = calculate_extent(&cluster.bounds);

        writeln!(file, "\t\t{{")?;
        writeln!(file, "\t\t\tid = {:?},", cluster.name)?;
        writeln!(file, "\t\t\tmin_score = {},", cluster.min_cluster_score)?;
        writeln!(file, "\t\t\tcenter = {},", fmt_vec(center))?;
        writeln!(file, "\t\t\tmins = {},", fmt_vec(mins))?;
        writeln!(file, "\t\t\tmaxs = {},", fmt_vec(maxs))?;

        // List of light IDs
        write!(file, "\t\t\tlights = [")?;
        for (j, (light, _score)) in cluster.lights.iter().enumerate() {
            if j > 0 { write!(file, ", ")?; }
            write!(file, "\"_{}\"", light.debug_id)?;
        }
        writeln!(file, "],")?;

         // List of rejected light IDs (Debug info)
        write!(file, "\t\t\trejected = [")?;
        for (j, (light, _score)) in cluster.rejected_lights.iter().enumerate() {
            if j > 0 { write!(file, ", ")?; }
            write!(file, "\"_{}\"", light.debug_id)?;
        }
        writeln!(file, "]")?;

        if i < clusters.len() - 1 {
            writeln!(file, "\t\t}},")?;
        } else {
            writeln!(file, "\t\t}}")?;
        }
    }
    writeln!(file, "\t],")?;

    // == Generate Lights Dictionary
    writeln!(file, "\tlights = {{")?;
    for (i, light) in all_lights.iter().enumerate() {
        writeln!(file, "\t\t_{} = {{", light.debug_id.replace(".", "_"))?;
        writeln!(file, "\t\t\tpos = {},", fmt_vec(light.pos))?;

        let dir_vec = match light.light_type {
            LightType::Point => None,
            LightType::Spot { direction, .. } => Some(direction),
            LightType::Rect { direction, .. } => Some(direction),
        };

        if let Some(d) = dir_vec {
             writeln!(file, "\t\t\tdir = {},", fmt_vec(d))?;
        }

        // Convert normalized color (0.0-1.0) back to 0-255
        let col = [
            (light.color[0] * 255.0).round(),
            (light.color[1] * 255.0).round(),
            (light.color[2] * 255.0).round(),
        ];
        writeln!(file, "\t\t\tcolor = {},", fmt_vec(col))?;

        writeln!(file, "\t\t\tintensity = {},", light.intensity)?;
        writeln!(file, "\t\t\trange = {},", light.range)?;

        if let Some(d50) = light.fifty_percent_distance {
            writeln!(file, "\t\t\tdist50 = {},", d50)?;
        }

        if light.blockers.iter().any(|b| b.is_some()) {
            writeln!(file, "\t\t\tblockers = [")?;
            for blocker in light.blockers.iter().flatten() {
                let b_pos = blocker.pos.unwrap_or(light.pos);

                let half_w = blocker.width * 0.5;
                let half_h = blocker.height * 0.5;
                let half_d = blocker.depth * 0.5;

                let mins = [-half_w, -half_h, -half_d];
                let maxs = [half_w, half_h, half_d];

                writeln!(file, "\t\t\t\t{{")?;
                writeln!(file, "\t\t\t\t\tpos = {},", fmt_vec(b_pos))?;
                writeln!(file, "\t\t\t\t\tmins = {},", fmt_vec(mins))?;
                writeln!(file, "\t\t\t\t\tmaxs = {},", fmt_vec(maxs))?;
                writeln!(file, "\t\t\t\t}},")?;
            }
            writeln!(file, "\t\t\t],")?;
        }

        // Write Associations
        if let Some(assocs) = light_associations.get(&light.debug_id) {
            writeln!(file, "\t\t\tassociations = [")?;
            for assoc in assocs {
                writeln!(file, "\t\t\t\t{{ surface = {:?}, rank = {}, score = {} }},", assoc.surface, assoc.rank, assoc.score)?;
            }
            writeln!(file, "\t\t\t],")?;
        }

        // == Generate Meta String
        let meta = generate_meta(light);
        writeln!(file, "\t\t\tmeta = {:?}", meta)?;

        if i < all_lights.len() - 1 {
            writeln!(file, "\t\t}},")?;
        } else {
            writeln!(file, "\t\t}}")?;
        }
    }
    writeln!(file, "\t}}")?; // Close lights
    writeln!(file, "}}")?; // Close root

    Ok(())
}

fn fmt_vec(v: Vec3) -> String {
    format!("Vector({}, {}, {})", v[0], v[1], v[2])
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
