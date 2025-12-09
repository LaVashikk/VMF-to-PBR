use crate::math::{cross, dot, normalize, sub, AABB, Vec3};
use log::{debug, warn};
use vmf_forge::prelude::{Entity, Solid};

#[derive(Debug, Clone)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
}

#[derive(Debug, Clone)]
pub struct ConvexBrush {
    pub planes: Vec<Plane>,
    pub _bounds: AABB,
}

impl ConvexBrush {
    /// Converts a VMF Solid into a mathematical ConvexBrush
    pub fn from_vmf_solid(solid: &Solid) -> Option<Self> {
        let mut planes = Vec::with_capacity(solid.sides.len());
        let mut aabb = AABB::new();
        let mut valid_points_found = false;

        for side in &solid.sides {
            // Parse 3 points of the plane
            let points = match super::utils::parse_plane_points(&side.plane) {
                Some(pts) => pts,
                None => {
                    warn!("Solid ID {}: Malformed plane definition found. Side has less than 3 points. Side plane: '{}'", solid.id, side.plane);
                    continue; // Broken plane definition
                }
            };

            let p1 = points[0];
            let p2 = points[1];
            let p3 = points[2];

            // Update AABB (Approximately, using plane points. For a precise AABB,
            // one would need to find plane intersections, but for VMF, plane points
            // usually lie on the brush corners, so this is okay).
            aabb.extend(p1);
            aabb.extend(p2);
            aabb.extend(p3);
            valid_points_found = true;

            // Calculate the plane normal
            // Vectors for the triangle sides
            let v1 = sub(p2, p1);
            let v2 = sub(p3, p1);

            // VMF winding order is counter-clockwise, so cross(v1, v2) should point outwards.
            // Flipping to cross(v2, v1) to test if winding order is inverted in the source data.
            let n = normalize(cross(v2, v1));

            // Calculate distance D
            // Equation: dot(N, P) + D = 0  =>  D = -dot(N, P)
            // Note: Some engines use D = dot(N, P). It's a matter of convention.
            // We use: dot(N, P) + dist = 0.
            // If a point is outside (in front of the plane), then dot(N, P) + dist > 0.
            let d = -dot(n, p1);

            planes.push(Plane {
                normal: n,
                dist: d,
            });
        }

        if planes.is_empty() || !valid_points_found {
            warn!("Solid ID {} was skipped because it contains no valid planes.", solid.id);
            return None;
        }

        debug!("Created ConvexBrush for solid ID {} with {} planes. AABB: min={:?}, max={:?}", solid.id, planes.len(), aabb.min, aabb.max);
        Some(ConvexBrush {
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
