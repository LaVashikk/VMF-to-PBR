use crate::{constants::LUT_WIDTH, types::{LightDef, LightType}};
use super::geometry::ConvexBrush;
use crate::math::{Vec3, AABB};
use super::tracer;
use log::debug;
use std::collections::HashSet;

// Tolerance in degrees. Allows the light to "catch" an object if it extends slightly beyond the cone's boundaries.
const CONE_ANGLE_TOLERANCE_DEG: f32 = 10.0;


/// Scoring & Light Selection
pub fn select_and_score_lights(
    all_lights: &[LightDef],
    bounds: &AABB,
    world_brushes: &[ConvexBrush],
    exclude_lights: &HashSet<String>,
    force_lights: &HashSet<String>,
    min_score: f32,
) -> (Vec<(LightDef, f32)>, Vec<(LightDef, f32)>) {
    let mut scored_lights: Vec<(usize, f32)> = Vec::new();

    for (idx, light) in all_lights.iter().enumerate() {
        // Check Exclude
        if light.is_named_light && exclude_lights.contains(&light.target_name) { // TODo: improve it! add additional fake-naming key
            debug!("  > Light '{}' (id: {}) manually excluded.", light.target_name, light.id);
            continue;
        }

        // Check Force
        if light.is_named_light && force_lights.contains(&light.target_name) { // TODo: improve it! add additional fake-naming key
            debug!("  > Light '{}' (id: {}) manually included.", light.target_name, light.id);
            scored_lights.push((idx, f32::MAX));
            continue;
        }

        let score = calculate_score(light, bounds, &world_brushes);
        if score > 0.0 {
            scored_lights.push((idx, score));
        }
    }

    // Normalization of scores
    let max_score = scored_lights.iter()
        .filter(|(_, s)| *s < f32::MAX) // Ignore forced lights
        .map(|(_, s)| *s)
        .fold(0.0, f32::max);
    if max_score > 0.0 {
        for (_, score) in scored_lights.iter_mut() {
            if *score < f32::MAX {
                *score /= max_score;
            }
        }
    }

    // Sort lights by score in descending order
    scored_lights.sort_by(|a, b| b.1.partial_cmp(&a.1).expect("NaN, its a bug"));

    let (mut accepted_candidates, mut rejected_candidates): (Vec<_>, Vec<_>) = scored_lights.into_iter()
        .partition(|(_, s)| *s >= f32::MAX || *s >= min_score);

    if accepted_candidates.len() > LUT_WIDTH {
        let overflow = accepted_candidates.split_off(LUT_WIDTH);
        rejected_candidates.extend(overflow);
    }

    // Stable sort to prefer named lights
    accepted_candidates.sort_by_key(|(idx, _)| !all_lights[*idx].is_named_light);

    let selected_lights: Vec<(LightDef, f32)> = accepted_candidates.into_iter()
        .map(|(idx, score)| (all_lights[idx].clone(), score))
        .collect();

    let rejected_lights: Vec<(LightDef, f32)> = rejected_candidates.into_iter()
        .map(|(idx, score)| (all_lights[idx].clone(), score))
        .collect();

    (selected_lights, rejected_lights)
}

/// Calculates a "Score" for a (Light, Surface) pair.
/// The higher the score, the more important the light is. 0.0 = light is not needed.
pub fn calculate_score(
    light: &LightDef,
    surface_aabb: &AABB,
    world_brushes: &[ConvexBrush],
) -> f32 {
    let light_pos = light.pos;
    debug!("Calculating score for light {:?} (id: {}) on surface with center {:?}", light.target_name, light.id, surface_aabb.center);

    // Quick distance test
    let dist_sq = crate::math::sq_dist_point_aabb(light_pos, surface_aabb);
    let dist = dist_sq.sqrt();
    let max_dist = light.range * 2.0;
    if dist > max_dist {
        debug!("  > Culled by distance: dist={:.2} > max_dist={:.2}", dist, max_dist);
        return 0.0;
    }

    // Shape Check (Spot / Rect Direction)
    // TODO: broken for now, FUCKING SHIT
    // if !check_shape_visibility(light, surface_aabb) {
    //     if light.is_named_light {
    //          debug!("  > Named light '{}' (id: {}) culled by shape. (Closest Dist: {:.1})", light.target_name, light.id, dist);
    //     } else {
    //         debug!("  > Shape check failed for light '{}' (id: {}) on surface with center {:?}", light.target_name, light.id, surface_aabb.center);
    //     }
    //     return 0.0;
    // }

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
    if estimated_brightness < 0.001 { // todo: think about it a bit more.. im not sure rn
        debug!("  > Culled by estimated_brightness ({} < 0.001)", estimated_brightness);
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

    debug!("  > Light {} (id: {}, type: {}) | Brightness: {:.2} | Vis: {:.2} | Score: {:.2}",
           light.target_name, light.id, light.light_type.name(), estimated_brightness, visibility_factor, score);

    score
}

/// Checks if point of AABB falls within the Spot cone or the front hemisphere of Rect
fn check_shape_visibility(light: &LightDef, aabb: &AABB) -> bool {
    let points = get_sample_points(aabb, light.pos);

    match &light.light_type {
        LightType::Spot { direction, outer_angle, .. } => {
            let light_dir = direction.normalize();

            // Expand the angle by the tolerance constant
            // outer_angle in Source is the full opening angle, so divide by 2
            let effective_angle_deg = (outer_angle / 2.0) + CONE_ANGLE_TOLERANCE_DEG;
            let limit_cos = effective_angle_deg.to_radians().cos();

            for point in points.iter() {
                let to_target = *point - light.pos;
                let dist = to_target.dot(to_target).sqrt();
                if dist < 0.1 { return true; }

                let dir_to_target = Vec3::new(to_target.0 / dist, to_target.1 / dist, to_target.2 / dist);
                let cos_angle = light_dir.dot(dir_to_target);

                if cos_angle >= limit_cos {
                    return true;
                }
            }
            return false;
        },
        LightType::Rect { direction, bidirectional, .. } => {
            if !bidirectional {
                let light_dir = direction.normalize();
                for point in points {
                    let to_target = point - light.pos;
                    let dist = to_target.dot(to_target).sqrt();
                    if dist < 0.1 { return true; }

                    let dir_to_target = Vec3::new(to_target.0 / dist, to_target.1 / dist, to_target.2 / dist);
                    if light_dir.dot(dir_to_target) >= -0.1 {
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
    points.push(Vec3::new(aabb.min[0], aabb.min[1], aabb.min[2]));
    points.push(Vec3::new(aabb.max[0], aabb.min[1], aabb.min[2]));
    points.push(Vec3::new(aabb.min[0], aabb.max[1], aabb.min[2]));
    points.push(Vec3::new(aabb.max[0], aabb.max[1], aabb.min[2]));

    points.push(Vec3::new(aabb.min[0], aabb.min[1], aabb.max[2]));
    points.push(Vec3::new(aabb.max[0], aabb.min[1], aabb.max[2]));
    points.push(Vec3::new(aabb.min[0], aabb.max[1], aabb.max[2]));
    points.push(Vec3::new(aabb.max[0], aabb.max[1], aabb.max[2]));

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
    let closest = Vec3::new(
        target_pos[0].clamp(min[0], max[0]),
        target_pos[1].clamp(min[1], max[1]),
        target_pos[2].clamp(min[2], max[2]),
    );
    points.push(closest);

    points
}
