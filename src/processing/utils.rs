use std::sync::LazyLock;
use regex::Regex;

use crate::math::{add, cross, normalize, sub, Vec3};

pub static PLANE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(([\d\.\-eE]+)\s+([\d\.\-eE]+)\s+([\d\.\-eE]+)\)").expect("Invalid Regex")
});

/// Extracts 3 plane points from the VMF string "(x y z) (x y z) (x y z)"
pub fn parse_plane_points(plane_str: &str) -> Option<[Vec3; 3]> {
    let mut points = Vec::with_capacity(3);

    for cap in PLANE_RE.captures_iter(plane_str) {
        let x = cap[1].parse::<f32>().ok()?;
        let y = cap[2].parse::<f32>().ok()?;
        let z = cap[3].parse::<f32>().ok()?;
        points.push([x, y, z]);
    }

    if points.len() == 3 {
        Some([points[0], points[1], points[2]])
    } else {
        None
    }
}

/// Calculates the normal to a boundary given three points
pub fn calc_face_normal(p: [Vec3; 3]) -> Vec3 {
    let v1 = sub(p[1], p[0]);
    let v2 = sub(p[2], p[0]);
    normalize(cross(v1, v2))
}

/// Applies an offset to all points in a plane row
pub fn apply_offset_to_plane(plane_str: &str, offset: Vec3) -> String {
    if let Some(points) = parse_plane_points(plane_str) {
        let p1 = add(points[0], offset);
        let p2 = add(points[1], offset);
        let p3 = add(points[2], offset);

        format!("({:.4} {:.4} {:.4}) ({:.4} {:.4} {:.4}) ({:.4} {:.4} {:.4})",
            p1[0], p1[1], p1[2],
            p2[0], p2[1], p2[2],
            p3[0], p3[1], p3[2]
        )
    } else {
        plane_str.to_string()
    }
}
