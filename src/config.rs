//! Shared settings + cross-thread state.
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Windows,
    Serial,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    Hold,
    Toggle,
    Always,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    Cursor,
    Center,
    Score,
}

#[derive(Clone)]
pub struct Settings {
    pub backend: Backend,

    // detection
    pub conf: f32,
    pub use_gpu: bool,

    // capture
    pub full_screen: bool,
    pub fov_size: i32,

    // targeting
    pub target_mode: TargetMode,
    pub target_ema: f32,

    // smoothing / movement
    pub smoothing: f32,
    pub max_speed: f32,
    pub deadzone: f32,
    pub gain: f32,

    // activation
    pub activation: Activation,
    pub activation_vk: i32,
    pub click_on_target: bool,

    // loop
    pub tick_hz: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            backend: Backend::Windows,
            conf: 0.35,
            use_gpu: true,
            full_screen: true,
            fov_size: 640,
            target_mode: TargetMode::Cursor,
            target_ema: 0.40,
            smoothing: 0.70,
            max_speed: 60.0,
            deadzone: 3.0,
            gain: 1.0,
            activation: Activation::Hold,
            activation_vk: 0x02, // VK_RBUTTON
            click_on_target: false,
            tick_hz: 144,
        }
    }
}

#[derive(Clone)]
pub struct Status {
    pub model_loaded: bool,
    pub provider: String,
    pub serial_connected: bool,
    pub serial_verified: bool,
    pub active: bool,
    pub fps: f32,
    pub det_count: usize,
    pub message: String,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            model_loaded: false,
            provider: "-".into(),
            serial_connected: false,
            serial_verified: false,
            active: false,
            fps: 0.0,
            det_count: 0,
            message: "Idle. Load a model to begin.".into(),
        }
    }
}

#[derive(Default)]
pub struct Commands {
    pub load_model: Option<String>,
    pub connect: Option<String>,
    pub disconnect: bool,
}

/// A downscaled RGBA preview frame produced by the worker.
pub struct Preview {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub status: Mutex<Status>,
    pub commands: Mutex<Commands>,
    pub preview: Mutex<Option<Preview>>,
    pub running: AtomicBool,
    pub mover_enabled: AtomicBool,
    pub preview_enabled: AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            settings: Mutex::new(Settings::default()),
            status: Mutex::new(Status::default()),
            commands: Mutex::new(Commands::default()),
            preview: Mutex::new(None),
            running: AtomicBool::new(true),
            mover_enabled: AtomicBool::new(false),
            preview_enabled: AtomicBool::new(true),
        }
    }
}
