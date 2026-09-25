//! Minimal vector / quaternion math for camera poses.
//!
//! Only what the POV camera needs: a right-handed Unity-compatible `Vector3`
//! and `Quaternion` with a normalised-lerp blend for averaging the two eye
//! rotations. Full slerp is not worth the code here because the inputs are two
//! nearly identical eye rotations.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        }
    }
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn midpoint(a: Self, b: Self) -> Self {
        Self {
            x: (a.x + b.x) * 0.5,
            y: (a.y + b.y) * 0.5,
            z: (a.z + b.z) * 0.5,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Quat {
    pub fn dot(a: Self, b: Self) -> f32 {
        a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w
    }

    /// Normalised lerp with hemisphere correction.
    pub fn nlerp(a: Self, b: Self, t: f32) -> Self {
        let b = if Self::dot(a, b) < 0.0 {
            Self {
                x: -b.x,
                y: -b.y,
                z: -b.z,
                w: -b.w,
            }
        } else {
            b
        };

        let mut r = Self {
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
            z: a.z + (b.z - a.z) * t,
            w: a.w + (b.w - a.w) * t,
        };

        let len = (r.x * r.x + r.y * r.y + r.z * r.z + r.w * r.w).sqrt();
        if len > 1e-6 {
            r.x /= len;
            r.y /= len;
            r.z /= len;
            r.w /= len;
            r
        } else {
            a
        }
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }

    /// Angle between two orientations, in radians.
    pub fn angle_to(a: Self, b: Self) -> f32 {
        2.0 * Self::dot(a, b).abs().min(1.0).acos()
    }

    /// Spherical interpolation, shortest path.
    pub fn slerp(a: Self, b: Self, t: f32) -> Self {
        let mut b = b;
        let mut cos = Self::dot(a, b);
        if cos < 0.0 {
            cos = -cos;
            b = Self {
                x: -b.x,
                y: -b.y,
                z: -b.z,
                w: -b.w,
            };
        }

        // Nearly parallel: fall back to nlerp to avoid dividing by ~0.
        if cos > 0.9995 {
            return Self::nlerp(a, b, t);
        }

        let theta = cos.clamp(-1.0, 1.0).acos();
        let sin_theta = theta.sin();
        if sin_theta.abs() < 1e-6 {
            return a;
        }

        let wa = ((1.0 - t) * theta).sin() / sin_theta;
        let wb = (t * theta).sin() / sin_theta;

        Self {
            x: a.x * wa + b.x * wb,
            y: a.y * wa + b.y * wb,
            z: a.z * wa + b.z * wb,
            w: a.w * wa + b.w * wb,
        }
    }

    /// Rotates `v` by this quaternion (standard `v' = v + 2w(q x v) + 2(q x (q x v))`).
    pub fn rotate_vec(&self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);

        let t = Vec3::new(
            2.0 * (q.y * v.z - q.z * v.y),
            2.0 * (q.z * v.x - q.x * v.z),
            2.0 * (q.x * v.y - q.y * v.x),
        );

        Vec3::new(
            v.x + self.w * t.x + (q.y * t.z - q.z * t.y),
            v.y + self.w * t.y + (q.z * t.x - q.x * t.z),
            v.z + self.w * t.z + (q.x * t.y - q.y * t.x),
        )
    }
}
