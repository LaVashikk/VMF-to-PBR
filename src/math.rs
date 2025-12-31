#![allow(dead_code)]

pub type Vec3 = [f32; 3];

#[derive(Debug, Clone, Copy)]
pub struct AABB {
    pub min: Vec3,
    pub max: Vec3,
    pub center: Vec3,
}

impl Default for AABB {
    fn default() -> Self {
        Self::new()
    }
}

impl AABB {
    pub fn new() -> Self {
        Self {
            min: [f32::MAX, f32::MAX, f32::MAX],
            max: [f32::MIN, f32::MIN, f32::MIN],
            center: [0.0; 3],
        }
    }

    pub fn extend(&mut self, p: Vec3) {
        self.min[0] = self.min[0].min(p[0]);
        self.min[1] = self.min[1].min(p[1]);
        self.min[2] = self.min[2].min(p[2]);

        self.max[0] = self.max[0].max(p[0]);
        self.max[1] = self.max[1].max(p[1]);
        self.max[2] = self.max[2].max(p[2]);

        self.center = [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ];
    }

    // Checking the intersection of two AABBs
    pub fn intersects(&self, _other: &AABB) -> bool { // TODO: unused now, for tracer optimize?..
        todo!()
    //     self.min[0] <= other.max[0] && self.max[0] >= other.min[0] &&
    //     self.min[1] <= other.max[1] && self.max[1] >= other.min[1] &&
    //     self.min[2] <= other.max[2] && self.max[2] >= other.min[2]
    }
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn mul(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn normalize(a: Vec3) -> Vec3 {
    let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if len == 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        [a[0] / len, a[1] / len, a[2] / len]
    }
}

pub fn parse_vector(s: &str) -> Vec3 {
    let parts: Vec<f32> = s.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    if parts.len() >= 3 {
        [parts[0], parts[1], parts[2]]
    } else {
        [0.0, 0.0, 0.0]
    }
}

pub fn sq_dist_point_aabb(point: Vec3, aabb: &AABB) -> f32 {
    let mut sq_dist = 0.0;

    for i in 0..3 {
        let v = point[i];
        if v < aabb.min[i] {
            sq_dist += (aabb.min[i] - v) * (aabb.min[i] - v);
        }
        if v > aabb.max[i] {
            sq_dist += (v - aabb.max[i]) * (v - aabb.max[i]);
        }
    }

    sq_dist
}
