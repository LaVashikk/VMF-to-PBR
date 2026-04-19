#![allow(dead_code)]

use std::fmt;
use std::ops::{Add, AddAssign, Div, DivAssign, Index, IndexMut, Mul, MulAssign, Sub, SubAssign};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3(pub f32, pub f32, pub f32);

impl Vec3 {
    pub const ZERO: Self = Self(0.0, 0.0, 0.0);
    pub const ONE: Self = Self(1.0, 1.0, 1.0);
    pub const MIN: Self = Self(f32::MIN, f32::MIN, f32::MIN);
    pub const MAX: Self = Self(f32::MAX, f32::MAX, f32::MAX);

    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self(x, y, z)
    }

    pub fn dot(self, other: Self) -> f32 {
        self.0 * other.0 + self.1 * other.1 + self.2 * other.2
    }

    pub fn cross(self, other: Self) -> Self {
        Self(
            self.1 * other.2 - self.2 * other.1,
            self.2 * other.0 - self.0 * other.2,
            self.0 * other.1 - self.1 * other.0,
        )
    }

    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    pub fn normalize(self) -> Self {
        let len = self.length();
        if len == 0.0 {
            Self::ZERO
        } else {
            self / len
        }
    }

    pub fn distance(self, other: Self) -> f32 {
        (self - other).length()
    }

    pub fn min(self, other: Self) -> Self {
        Self(
            self.0.min(other.0),
            self.1.min(other.1),
            self.2.min(other.2),
        )
    }

    pub fn max(self, other: Self) -> Self {
        Self(
            self.0.max(other.0),
            self.1.max(other.1),
            self.2.max(other.2),
        )
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }

    pub fn parse(s: &str) -> Self {
        let parts: Vec<f32> = s
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        if parts.len() >= 3 {
            Self(parts[0], parts[1], parts[2])
        } else {
            Self::ZERO
        }
    }

    pub fn to_origin(&self) -> String {
        format!("{:.3} {:.3} {:.3}", self.0, self.1, self.2)
    }
}

impl fmt::Display for Vec3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{:.3}, {:.3}, {:.3}>", self.0, self.1, self.2)
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0, self.1 + other.1, self.2 + other.2)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0, self.1 - other.1, self.2 - other.2)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self(self.0 * s, self.1 * s, self.2 * s)
    }
}

impl Div<f32> for Vec3 {
    type Output = Self;
    fn div(self, s: f32) -> Self {
        Self(self.0 / s, self.1 / s, self.2 / s)
    }
}

impl Mul<Vec3> for Vec3 {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self(self.0 * other.0, self.1 * other.1, self.2 * other.2)
    }
}

impl Index<usize> for Vec3 {
    type Output = f32;
    fn index(&self, index: usize) -> &Self::Output {
        match index {
            0 => &self.0,
            1 => &self.1,
            2 => &self.2,
            _ => panic!("Index out of bounds for Vec3"),
        }
    }
}

impl IndexMut<usize> for Vec3 {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        match index {
            0 => &mut self.0,
            1 => &mut self.1,
            2 => &mut self.2,
            _ => panic!("Index out of bounds for Vec3"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
            min: Vec3::MAX,
            max: Vec3::MIN,
            center: Vec3::ZERO,
        }
    }

    pub fn extend(&mut self, p: Vec3) {
        self.min[0] = self.min[0].min(p[0]);
        self.min[1] = self.min[1].min(p[1]);
        self.min[2] = self.min[2].min(p[2]);

        self.max[0] = self.max[0].max(p[0]);
        self.max[1] = self.max[1].max(p[1]);
        self.max[2] = self.max[2].max(p[2]);

        self.center = Vec3::new(
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        );
    }

    // Checking the intersection of two AABBs
    pub fn intersects(&self, other: &AABB) -> bool {
        self.min.0 <= other.max.0 && self.max.0 >= other.min.0 &&
        self.min.1 <= other.max.1 && self.max.1 >= other.min.1 &&
        self.min.2 <= other.max.2 && self.max.2 >= other.min.2
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
