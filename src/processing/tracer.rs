use crate::math::{dot, sub, Vec3};
use crate::processing::geometry::ConvexBrush;
use log::debug;

const EPSILON: f32 = 0.001;

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

    for (brush_idx, brush) in brushes.iter().enumerate() {
        // todo: add Broad Phase AABB Check here for optimization

        // Narrow Phase: Slab Method
        if intersect_brush(start, dir, dist, brush) {
            debug!("      - Ray from {:?} to {:?} is occluded by brush #{}", start, end, brush_idx);
            return true; // Shadow found
        }
    }

    false
}

/// Returns true if the ray (origin, dir) intersects the brush at a distance < max_dist
fn intersect_brush(origin: Vec3, dir: Vec3, max_dist: f32, brush: &ConvexBrush) -> bool {
    let mut t_near = 0.0_f32; // Entry into brush
    let mut t_far = max_dist; // Exit from brush

    // Iterate over all planes of the brush (Slabs)
    for plane in &brush.planes {
        // Plane equation: dot(N, P) + d = 0
        // Ray: P = O + t*D
        // Substitute: dot(N, O + t*D) + d = 0
        // dot(N, O) + t*dot(N, D) + d = 0
        // t = -(dot(N, O) + d) / dot(N, D)

        let numer = -(dot(plane.normal, origin) + plane.dist);
        let denom = dot(plane.normal, dir);

        if denom.abs() < 1e-6 {
            // Ray is parallel to the plane
            // If numer < 0, the point is outside this plane -> ray misses the brush
            if numer < 0.0 {
                return false;
            }
            // Otherwise, the ray is inside the "slab", continue
        } else {
            // Calculate intersection t
            let t = numer / denom;

            if denom < 0.0 {
                // We are entering the plane (Normal points against the ray direction)
                if t > t_near {
                    t_near = t;
                }
            } else {
                // We are exiting the plane (Normal points along the ray direction)
                if t < t_far {
                    t_far = t;
                }
            }

            // If entry point is further than exit point -> miss
            if t_near > t_far {
                return false;
            }
            // If exit point is negative (brush is behind) -> miss
            if t_far < 0.0 {
                return false;
            }
        }
    }

    // If we reached here, the entry and exit intervals are mathematically valid.
    // But we need to filter out cases where the ray just touches the surface (exits at point 0).
    if t_near < t_far - EPSILON {
        // The ray actually passed through the "body" of the brush.
        // Additional check: did this happen BEFORE the light source?
        // (t_near is already guaranteed to be >= 0 due to initialization)
        return t_near < max_dist - EPSILON;
    }

    false
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
        planes.push(Plane { normal: [1.0, 0.0, 0.0], dist: -size });
        // -X
        planes.push(Plane { normal: [-1.0, 0.0, 0.0], dist: -size });
        // +Y
        planes.push(Plane { normal: [0.0, 1.0, 0.0], dist: -size });
        // -Y
        planes.push(Plane { normal: [0.0, -1.0, 0.0], dist: -size });
        // +Z
        planes.push(Plane { normal: [0.0, 0.0, 1.0], dist: -size });
        // -Z
        planes.push(Plane { normal: [0.0, 0.0, -1.0], dist: -size });

        let mut _bounds = AABB::new();
        _bounds.extend([-size, -size, -size]);
        _bounds.extend([size, size, size]);

        ConvexBrush { planes, _bounds }
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
