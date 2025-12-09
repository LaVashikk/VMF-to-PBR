use crate::math::{dot, normalize, sub, Vec3, AABB};
use crate::processing::geometry::ConvexBrush;
use crate::processing::tracer;
use crate::types::{LightDef, LightType};
use log::debug;

// Tolerance in degrees. Allows the light to "catch" an object if it extends slightly beyond the cone's boundaries.
const CONE_ANGLE_TOLERANCE_DEG: f32 = 10.0;

/// Calculates a "Score" for a (Light, Surface) pair.
/// The higher the score, the more important the light is. 0.0 = light is not needed.
pub fn calculate_score(
    light: &LightDef,
    surface_aabb: &AABB,
    world_brushes: &[ConvexBrush],
) -> f32 {
    let light_pos = light.pos;
    debug!("Calculating score for light '{:?}' on surface with center {:?}", light.debug_id, surface_aabb.center);

    // Quick distance test
    let dist_sq = crate::math::sq_dist_point_aabb(light_pos, surface_aabb);
    let dist = dist_sq.sqrt();
    let max_dist = light.range * 2.0;
    if dist > max_dist {
        debug!("  > Culled by distance: dist={:.2} > max_dist={:.2}", dist, max_dist);
        return 0.0;
    }

    // Shape Check (Spot / Rect Direction)
    if !check_shape_visibility(light, surface_aabb) {
        if light.is_named_light {
             debug!("  > Named light '{}' culled by shape. (Closest Dist: {:.1})", light.debug_id, dist);
        }
        return 0.0;
    }

    // Scoring, using: 'I / (1 + K * d^2)'
    let k = light.attenuation_k;
    let attenuation = 1.0 / (1.0 + k * dist_sq);

    // Windowing `(1 - (d^2 / r^2))^2`
    let range_sq = light.range * light.range;
    let dist_norm_sq = dist_sq / range_sq.max(0.001);
    let window = (1.0 - dist_norm_sq).max(0.0);
    let window_sq = window * window;

    // Estimated surface brightness (no way)
    let estimated_brightness = light.intensity * attenuation * window_sq;
    if estimated_brightness < 0.5 {
        return 0.0;
    }

    //  Raytracing (AABB corners + center)
    let sample_points = get_sample_points(surface_aabb, light_pos);
    let mut visible_samples = 0;

    for point in &sample_points {
        // Check for occlusion: From the surface point TO the light
        // If is_occluded returns false (no obstacle), we can see the light
        if !tracer::is_occluded(*point, light_pos, world_brushes) {
            visible_samples += 1;
        }
    }

    if visible_samples == 0 {
        debug!("  > Culled by visibility: 0/{_total} samples visible", _total = sample_points.len());
        return 0.0; // Fully occluded by walls
    }

    // Visibility factor (0.0 to 1.0)
    let visibility_factor = visible_samples as f32 / sample_points.len() as f32;

    // Final Score
    let score = estimated_brightness * visibility_factor;

    debug!("  > Light {} | Brightness: {:.2} | Vis: {:.2} | Score: {:.2}",
           light.debug_id, estimated_brightness, visibility_factor, score);

    score
}

/// Checks if point of AABB falls within the Spot cone or the front hemisphere of Rect
fn check_shape_visibility(light: &LightDef, aabb: &AABB) -> bool {
    let points = get_sample_points(aabb, light.pos);

    match &light.light_type {
        LightType::Spot { direction, outer_angle, .. } => {
            let light_dir = normalize(*direction);

            // Expand the angle by the tolerance constant
            // outer_angle in Source is the full opening angle, so divide by 2
            let effective_angle_deg = (outer_angle / 2.0) + CONE_ANGLE_TOLERANCE_DEG;
            let limit_cos = effective_angle_deg.to_radians().cos();

            for point in points.iter() {
                let to_target = sub(*point, light.pos);
                let dist = dot(to_target, to_target).sqrt();
                if dist < 0.1 { return true; }

                let dir_to_target = [to_target[0]/dist, to_target[1]/dist, to_target[2]/dist];
                let cos_angle = dot(light_dir, dir_to_target);

                if cos_angle >= limit_cos {
                    return true;
                }
            }
            return false;
        },
        LightType::Rect { direction, bidirectional, .. } => {
            if !bidirectional {
                let light_dir = normalize(*direction);
                for point in points {
                    let to_target = sub(point, light.pos);
                    let dist = dot(to_target, to_target).sqrt();
                    if dist < 0.1 { return true; }

                    let dir_to_target = [to_target[0]/dist, to_target[1]/dist, to_target[2]/dist];
                    if dot(light_dir, dir_to_target) >= -0.1 {
                        return true;
                    }
                }
                return false;
            }
        },
        _ => {} // Point light shines everywhere
    }
    true
}

/// gen point for raytrace-test. 8 corners + center + nearest
fn get_sample_points(aabb: &AABB, target_pos: Vec3) -> Vec<Vec3> {
    let mut points = Vec::with_capacity(10);
    points.push(aabb.center);

    // Corners
    points.push([aabb.min[0], aabb.min[1], aabb.min[2]]);
    points.push([aabb.max[0], aabb.min[1], aabb.min[2]]);
    points.push([aabb.min[0], aabb.max[1], aabb.min[2]]);
    points.push([aabb.max[0], aabb.max[1], aabb.min[2]]);

    points.push([aabb.min[0], aabb.min[1], aabb.max[2]]);
    points.push([aabb.max[0], aabb.min[1], aabb.max[2]]);
    points.push([aabb.min[0], aabb.max[1], aabb.max[2]]);
    points.push([aabb.max[0], aabb.max[1], aabb.max[2]]);

    // todo: sooo, to avoid taking points that might be "under the wall", it's doubtful for now, but.. ok?
    let inset = 1.0;
    let min = [
        (aabb.min[0] + inset).min(aabb.center[0]),
        (aabb.min[1] + inset).min(aabb.center[1]),
        (aabb.min[2] + inset).min(aabb.center[2]),
    ];
    let max = [
        (aabb.max[0] - inset).max(aabb.center[0]),
        (aabb.max[1] - inset).max(aabb.center[1]),
        (aabb.max[2] - inset).max(aabb.center[2]),
    ];

    // Closest Point on AABB to Light
    let closest = [
        target_pos[0].clamp(min[0], max[0]),
        target_pos[1].clamp(min[1], max[1]),
        target_pos[2].clamp(min[2], max[2]),
    ];
    points.push(closest);

    points
}
