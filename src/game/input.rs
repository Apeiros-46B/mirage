use winit::event::{ElementState, MouseButton};
use winit::keyboard::{KeyCode, PhysicalKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InputBtn {
    PlaceBlock = 0,
    BreakBlock,
    Interact,
    MoveFwd,
    MoveLeft,
    MoveBack,
    MoveRight,
    MoveUp,
    MoveDown,
    DecreaseSpeed,
    IncreaseSpeed,
    CaptureCursor,
    Unrecognized,
}

impl From<PhysicalKey> for InputBtn {
    fn from(key: PhysicalKey) -> Self {
        let PhysicalKey::Code(code) = key else {
            return Self::Unrecognized;
        };

        match code {
            KeyCode::KeyE => Self::Interact,
            KeyCode::KeyW => Self::MoveFwd,
            KeyCode::KeyA => Self::MoveLeft,
            KeyCode::KeyS => Self::MoveBack,
            KeyCode::KeyD => Self::MoveRight,
            KeyCode::KeyJ => Self::DecreaseSpeed,
            KeyCode::KeyK => Self::IncreaseSpeed,
            KeyCode::Space => Self::MoveUp,
            KeyCode::ShiftLeft => Self::MoveDown,
            _ => Self::Unrecognized,
        }
    }
}

impl From<MouseButton> for InputBtn {
    fn from(btn: MouseButton) -> Self {
        match btn {
            MouseButton::Left => Self::BreakBlock,
            MouseButton::Right => Self::PlaceBlock,
            MouseButton::Middle => Self::CaptureCursor,
            _ => Self::Unrecognized,
        }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct InputSet(u32);

impl InputSet {
    pub fn insert(&mut self, btn: InputBtn) {
        self.0 |= 1 << (btn as u32);
    }

    pub fn remove(&mut self, btn: InputBtn) {
        self.0 &= !(1 << (btn as u32));
    }

    pub fn contains(&self, btn: InputBtn) -> bool {
        (self.0 & (1 << (btn as u32))) != 0
    }

    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

#[derive(Default, Debug)]
pub struct InputState {
    pub pressed: InputSet,
    pub just_pressed: InputSet,
    pub just_released: InputSet,
    pub mouse_delta: (f32, f32),
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_pressed(&self, btn: InputBtn) -> bool {
        self.pressed.contains(btn)
    }

    pub fn is_just_pressed(&self, btn: InputBtn) -> bool {
        self.just_pressed.contains(btn)
    }

    pub fn is_just_released(&self, btn: InputBtn) -> bool {
        self.just_released.contains(btn)
    }

    pub fn take_mouse_delta(&mut self) -> (f32, f32) {
        let delta = self.mouse_delta;
        self.mouse_delta = (0.0, 0.0);
        delta
    }

    pub fn reset_keys(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
    }

    pub fn handle_input<T>(&mut self, key: T, state: ElementState) where InputBtn: From<T> {
        let input = InputBtn::from(key);

        dbg!(&input);

        if state == ElementState::Pressed {
            self.pressed.insert(input);
            self.just_pressed.insert(input);
            self.just_released.remove(input);
        } else {
            self.pressed.remove(input);
            self.just_pressed.remove(input);
            self.just_released.insert(input);
        }
    }

    pub fn handle_mouse(&mut self, dx: f32, dy: f32) {
        dbg!(dx, dy);

        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }
}
