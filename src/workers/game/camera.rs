use glam::{Mat3, Mat4, Vec2, Vec3};
use winit::dpi::PhysicalSize;

use crate::workers::msgs::CameraState;
use super::{Polled, input::{InputBtn, InputState}};

#[derive(Clone, Debug)]
pub struct Camera {
    vertical_fov: f32,
    near_clip: f32,
    far_clip: f32,

    pub pos: Vec3,
    pub forward_dir: Vec3,

    pub pitch: f32,
    pub yaw: f32,

    pub inv_proj: Mat4,
    pub inv_view: Mat4,
    pub proj: Mat4,
    pub view: Mat4,

    scr_sz: Vec2,
}

const UP_DIR: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const BASE_SPEED: f32 = 50.0;
const BASE_ROT_SPEED: f32 = 0.005;

const DEFAULT_POS: Vec3 = Vec3::new(128.0, 128.0, 512.0);
const DEFAULT_YAW: f32 = -std::f32::consts::FRAC_PI_2; // facing -Z
const DEFAULT_PITCH: f32 = 0.0;

pub const DEFAULT_FOV_DEG: f32 = 70.0_f32;

impl Camera {
    pub fn new(scr_sz: PhysicalSize<u32>) -> Self {
        let vertical_fov = DEFAULT_FOV_DEG.to_radians();
        let near_clip = 0.1;
        let far_clip = 1000.0;

        let pos = DEFAULT_POS;

        let mut this = Self {
            vertical_fov,
            near_clip,
            far_clip,

            pos,
            forward_dir: Vec3::ZERO,
            pitch: DEFAULT_PITCH,
            yaw: DEFAULT_YAW,

            inv_proj: Mat4::IDENTITY,
            inv_view: Mat4::IDENTITY,
            proj: Mat4::IDENTITY,
            view: Mat4::IDENTITY,

            scr_sz: Vec2::ZERO,
        };

        this.resize(scr_sz);
        this.recalc_forward();
        this.recalc_proj();
        this.recalc_view();

        this
    }

    pub fn get_state(&self) -> CameraState {
        let view_rot = Mat4::from_mat3(Mat3::from_mat4(self.view));
        CameraState {
            rel_view_proj: self.proj * view_rot,
            inv_proj: self.inv_proj,
            inv_view: self.inv_view,
            pos: self.pos,
            _pad: 0.0,
        }
    }

    pub fn set_fov(&mut self, new_fov: f32) {
        if (new_fov - self.vertical_fov).abs() <= f32::EPSILON {
            return;
        }

        self.vertical_fov = new_fov;
        self.recalc_proj();
    }

    pub fn resize(&mut self, new_sz: PhysicalSize<u32>) {
        self.scr_sz = Vec2::new(new_sz.width as f32, new_sz.height as f32);
        self.recalc_proj();
    }

    fn recalc_proj(&mut self) {
        self.proj = Mat4::perspective_rh(
            self.vertical_fov,
            self.scr_sz.x / self.scr_sz.y,
            self.near_clip,
            self.far_clip,
        );
        self.inv_proj = self.proj.inverse();
    }

    fn recalc_view(&mut self) {
        self.view = Mat4::look_at_rh(self.pos, self.pos + self.forward_dir, UP_DIR);
        self.inv_view = self.view.inverse();
    }

    fn recalc_forward(&mut self) {
        let (yaw_sin, yaw_cos) = self.yaw.sin_cos();
        let (pitch_sin, pitch_cos) = self.pitch.sin_cos();
        self.forward_dir = Vec3::new(
            yaw_cos * pitch_cos,
            pitch_sin,
            yaw_sin * pitch_cos,
        ).normalize();
    }
}

impl Polled for Camera {
    fn poll(&mut self, input: &mut InputState, dt: f32) {
        let mut moving = false;

        let delta = input.take_mouse_delta();

        if delta.0 != 0.0 || delta.1 != 0.0 {
            self.yaw += delta.0 * BASE_ROT_SPEED;
            self.pitch += delta.1 * BASE_ROT_SPEED;

            // prevent camera flipping
            let max_pitch = std::f32::consts::FRAC_PI_2 - 0.01;
            self.pitch = self.pitch.clamp(-max_pitch, max_pitch);

            self.recalc_forward();

            moving = true;
        }

        let right_dir = self.forward_dir.cross(UP_DIR).normalize();
        let speed = BASE_SPEED;

        if input.is_pressed(InputBtn::MoveFwd) {
            self.pos += self.forward_dir * speed * dt;
            moving = true;
        } else if input.is_pressed(InputBtn::MoveBack) {
            self.pos -= self.forward_dir * speed * dt;
            moving = true;
        }

        if input.is_pressed(InputBtn::MoveLeft) {
            self.pos -= right_dir * speed * dt;
            moving = true;
        } else if input.is_pressed(InputBtn::MoveRight) {
            self.pos += right_dir * speed * dt;
            moving = true;
        }

        // has to be inverse for some reason... idk why
        if input.is_pressed(InputBtn::MoveDown) {
            self.pos += UP_DIR * speed * dt;
            moving = true;
        } else if input.is_pressed(InputBtn::MoveUp) {
            self.pos -= UP_DIR * speed * dt;
            moving = true;
        }

        if moving {
            self.recalc_view();
        }
    }
}
