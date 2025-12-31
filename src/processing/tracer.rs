use crate::math::{dot, sub, Vec3, AABB};
use crate::processing::geometry::ConvexBrush;
use log::debug;

const EPSILON: f32 = 0.001;

pub struct RayHit<'a> {
    pub t: f32,
    pub u_axis: &'a str,
    pub v_axis: &'a str,
}

/// Checks whether the path from `start` to `end` is blocked by the `brushes` geometry.
/// Returns true if the path is blocked (i.e., there is a shadow)
pub fn is_occluded(start: Vec3, end: Vec3, brushes: &[ConvexBrush]) -> bool {
    let diff = sub(end, start);
    let dist_sq = dot(diff, diff);
    let dist = dist_sq.sqrt();

    // If the points match, there is no overlap
    if dist < EPSILON {
        return false;
    }

    let dir = [diff[0] / dist, diff[1] / dist, diff[2] / dist];

    for brush in brushes.iter() {
        // Broad Phase AABB Check
        if !ray_aabb_intersect(start, dir, dist, &brush._bounds) {
            continue;
        }
        if let Some((_, plane_idx)) = intersect_brush(start, dir, dist, brush) {
            let plane = &brush.planes[plane_idx];
            let mat_lower = plane.material.to_lowercase();

            if mat_lower.contains("glass") {
                debug!("    -> Ignored: Glass texture '{}'", plane.material);
                continue;
            }

            debug!("      - Ray from {:?} to {:?} is occluded by brush #{} ({})", start, end, brush.id, plane.material);
            return true; // Shadow found
         }
    }
    false
}

pub fn trace_ray_closest(start: Vec3, dir: Vec3, max_dist: f32, brushes: &[ConvexBrush]) -> Option<RayHit> {
    let mut closest_t = max_dist;
    let mut hit_data = None;

    for brush in brushes.iter() {
        if !ray_aabb_intersect(start, dir, max_dist, &brush._bounds) {
            continue;
        }

        if let Some((t_near, plane_idx)) = intersect_brush(start, dir, closest_t, brush) {
            // Allow slightly negative t_near to account for starting exactly on surface
            if t_near < closest_t && t_near > -0.1 {
                let effective_t = t_near.max(0.0);
                closest_t = effective_t;
                let plane = &brush.planes[plane_idx];
                debug!("    -> New closest hit! (prev closest: {})", closest_t);

                hit_data = Some(RayHit {
                    t: effective_t,
                    u_axis: &plane.u_axis,
                    v_axis: &plane.v_axis,
                });
            }
        }
    }

    hit_data
}

fn ray_aabb_intersect(origin: Vec3, dir: Vec3, max_dist: f32, aabb: &AABB) -> bool {
    let mut tmin = 0.0_f32;
    let mut tmax = max_dist;
    for i in 0..3 {
        if dir[i].abs() < 1e-6 {
            if origin[i] < aabb.min[i] - EPSILON || origin[i] > aabb.max[i] + EPSILON { return false; }
        } else {
            let ood = 1.0 / dir[i];
            let mut t1 = (aabb.min[i] - origin[i]) * ood;
            let mut t2 = (aabb.max[i] - origin[i]) * ood;
            if t1 > t2 { std::mem::swap(&mut t1, &mut t2); }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax { return false; }
        }
    }
    true
}


fn intersect_brush(origin: Vec3, dir: Vec3, max_dist: f32, brush: &ConvexBrush) -> Option<(f32, usize)> {
    let mut t_near = -std::f32::MAX;
    let mut t_far = max_dist;
    let mut enter_plane_idx = None;

    for (i, plane) in brush.planes.iter().enumerate() {
        let mat_lower = plane.material.to_lowercase();
        // Filter out tools textures - they cannot be hit
        if mat_lower.contains("tools") && !mat_lower.contains("nodraw") && !mat_lower.contains("pbr_block") {
            continue;
        }

        let numer = -(dot(plane.normal, origin) + plane.dist);
        let denom = dot(plane.normal, dir);

        if denom.abs() < 1e-6 {
            if numer < 0.0 { return None; }
        } else {
            let t = numer / denom;
            if denom < 0.0 {
                if t > t_near {
                    t_near = t;
                    enter_plane_idx = Some(i);
                }
            } else if t < t_far { t_far = t; }
            if t_near > t_far || t_far < 0.0 { return None; }
        }
    }

    // Ensure the exit point is in front of the ray start
    if t_near < t_far - EPSILON && t_far > EPSILON && t_near < max_dist {
        if let Some(idx) = enter_plane_idx {
            return Some((t_near, idx));
        } else {
            return Some((0.0, 0));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::AABB;
    use crate::processing::geometry::{ConvexBrush, Plane};

    // Helper to create a cube sized from -size to +size on all axes
    fn create_test_cube(size: f32) -> ConvexBrush {
        let mut planes = Vec::new();
        // Normals point OUTSIDE the cube.
        // Equation: N*P + d = 0.
        // For wall X=size: N=(1,0,0). Point P=(size,0,0). 1*size + d = 0 => d = -size.

        // +X
        planes.push(Plane::new([1.0, 0.0, 0.0], -size));
        // -X
        planes.push(Plane::new([-1.0, 0.0, 0.0], -size));
        // +Y
        planes.push(Plane::new([0.0, 1.0, 0.0], -size));
        // -Y
        planes.push(Plane::new([0.0, -1.0, 0.0], -size));
        // +Z
        planes.push(Plane::new([0.0, 0.0, 1.0], -size));
        // -Z
        planes.push(Plane::new([0.0, 0.0, -1.0], -size));

        let mut _bounds = AABB::new();
        _bounds.extend([-size, -size, -size]);
        _bounds.extend([size, size, size]);

        ConvexBrush { planes, _bounds, id: 0 }
    }

    #[test]
    fn test_direct_hit() {
        // 10x10x10 cube at the center (from -10 to 10)
        let cube = create_test_cube(10.0);
        let world = vec![cube];

        // Ray through the cube: from -20 to +20 along X
        let start = [-20.0, 0.0, 0.0];
        let end = [20.0, 0.0, 0.0];

        // Should be occluded
        assert!(is_occluded(start, end, &world), "Ray through cube center should be occluded");
    }

    #[test]
    fn test_miss_side() {
        let cube = create_test_cube(10.0);
        let world = vec![cube];

        // Ray from the side: from -20 to +20, but Y=15 (misses the cube)
        let start = [-20.0, 15.0, 0.0];
        let end = [20.0, 15.0, 0.0];

        assert!(!is_occluded(start, end, &world), "Ray passing by the side should NOT be occluded");
    }

    #[test]
    fn test_short_ray_before() {
        let cube = create_test_cube(10.0);
        let world = vec![cube];

        // Ray directed at the wall but does not reach it
        // Wall starts at X=-10. Ray from -30 to -15.
        let start = [-30.0, 0.0, 0.0];
        let end = [-15.0, 0.0, 0.0];

        assert!(!is_occluded(start, end, &world), "Short ray before wall should NOT be occluded");
    }

    #[test]
    fn test_inside_out() {
        let cube = create_test_cube(10.0);
        let world = vec![cube];

        // Ray starts INSIDE the cube and goes out.
        // This is a debatable case (light inside a wall?), but technically it crosses a boundary.
        // Our logic t_near < max_dist should work if t_near > 0.
        // But if we are inside, t_near can be negative.
        // In our code:
        // if denom < 0 (entry): t will update t_near.
        // If we are inside looking out, we don't intersect "entering" planes (they are behind).
        // So is_occluded will likely return false, which is logical (light is not occluded by "entry").

        // Let's check shooting through from inside.
        let start = [0.0, 0.0, 0.0];
        let end = [20.0, 0.0, 0.0];

        // In the current Slab method implementation for a convex volume, if Origin is inside,
        // t_near will remain 0.0 (or negative), and t_far will be positive.
        // The intersect_brush logic returns true if t_near < max_dist.
        // But since t_near is initialized to 0.0, it will return true.
        // Let's verify this behavior. For a light baker, this is fine (light inside a wall should not shine).

        assert!(is_occluded(start, end, &world), "Ray from inside should be occluded (technically)");
    }

    #[test]
    fn test_grazing_miss() {
        let cube = create_test_cube(10.0);
        let world = vec![cube];

        // Ray runs parallel to the face, but slightly above (Y=10.001)
        let start = [-20.0, 10.1, 0.0];
        let end = [20.0, 10.1, 0.0];

        assert!(!is_occluded(start, end, &world), "Grazing ray should miss");
    }

    #[test]
    fn test_ray_starts_on_surface_and_goes_away() {
        let cube = create_test_cube(10.0);
        let world = vec![cube];

        // Ray starts on the surface (X=-10) and goes outward (towards -X)
        let start = [-10.0, 0.0, 0.0];
        let end = [-20.0, 0.0, 0.0];

        // Such a ray should NOT be considered occluded
        assert!(!is_occluded(start, end, &world), "Ray starting on surface and moving away should NOT be occluded");
    }
}
