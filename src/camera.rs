use glam::{Mat4, Vec3};

/// Left-handed fly camera (x right, y up, z forward), 0..1 depth.
pub struct FlyCamera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl FlyCamera {
    pub fn new(pos: Vec3, yaw: f32, pitch: f32) -> Self {
        Self {
            pos,
            yaw,
            pitch,
            fov_y: 65f32.to_radians(),
            near: 0.1,
            far: 3000.0,
        }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        )
    }

    pub fn right(&self) -> Vec3 {
        Vec3::Y.cross(self.forward()).normalize()
    }

    pub fn rotate(&mut self, dx: f32, dy: f32) {
        const SENS: f32 = 0.0032;
        self.yaw += dx * SENS;
        self.pitch = (self.pitch - dy * SENS).clamp(-1.55, 1.55);
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_lh(self.pos, self.pos + self.forward(), Vec3::Y);
        let proj = Mat4::perspective_lh(self.fov_y, aspect, self.near, self.far);
        proj * view
    }
}
