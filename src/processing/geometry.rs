use crate::{math::{cross, dot, mul, normalize, sub, Vec3, AABB}, processing::utils};
use log::{debug, warn};
use vmf_forge::prelude::{Entity, Solid};

#[derive(Debug, Clone)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
    pub u_axis: String,
    pub v_axis: String,
    pub material: String,
}

impl Plane {
    pub fn new(normal: Vec3, dist: f32) -> Self {
        Self {
            normal,
            dist,
            u_axis: String::new(),
            v_axis: String::new(),
            material: String::from("default"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConvexBrush {
    pub id: u64,
    pub planes: Vec<Plane>,
    pub _bounds: AABB,
}

impl ConvexBrush {
    /// Converts a VMF Solid into a mathematical ConvexBrush
    pub fn from_vmf_solid(solid: &Solid) -> Option<Self> {
        let mut planes = Vec::with_capacity(solid.sides.len());
        let mut aabb = AABB::new();
        let mut valid_points_found = false;

        // Check if this is a displacement brush
        // In Source, if a brush has a displacement face, only that face "exists" for physics/vis usually.
        let is_displacement = solid.sides.iter().any(|s| s.dispinfo.is_some());
        if is_displacement {
            return None
        }

        for side in &solid.sides {
            // Parse 3 points of the plane
            let points = match super::utils::parse_plane_points(&side.plane) {
                Some(pts) => pts,
                None => {
                    warn!("Solid ID {}: Malformed plane definition found. Side has less than 3 points. Side plane: '{}'", solid.id, side.plane);
                    continue; // Broken plane definition
                }
            };

            // Approximately, using plane points. For a precise AABB,
            // one would need to find plane intersections, but for VMF, plane points
            // usually lie on the brush corners, so this is okay
            aabb.extend(points[0]);
            aabb.extend(points[1]);
            aabb.extend(points[2]);
            valid_points_found = true;

            // Calculate the plane normal
            let n = mul(utils::calc_face_normal(points), -1.0); // todo haha
            let d = -dot(n, points[0]);

            planes.push(Plane {
                normal: n,
                dist: d,
                u_axis: side.u_axis.clone(),
                v_axis: side.v_axis.clone(),
                material: side.material.clone(),
            });
        }

        if planes.is_empty() || !valid_points_found {
            warn!("Solid ID {} was skipped because it contains no valid planes.", solid.id);
            return None;
        }

        debug!("Created ConvexBrush for solid ID {} with {} planes. AABB: min={:?}, max={:?}", solid.id, planes.len(), aabb.min, aabb.max);
        Some(ConvexBrush {
            id: solid.id,
            planes,
            _bounds: aabb,
        })
    }
}

pub fn get_entity_aabb(ent: &Entity) -> Option<AABB> {
    let solids = ent.solids.as_ref()?;
    if solids.is_empty() { return None; }

    // Re-use logic from ConvexBrush parsing but for AABB
    let mut aabb = AABB::new();
    let mut found = false;

    for solid in solids {
        for side in &solid.sides {
            if let Some(points) = super::utils::parse_plane_points(&side.plane) {
                for p in points {
                    aabb.extend(p);
                }
                found = true;
            }
        }
    }

    if !found { return None; }
    Some(aabb)
}
