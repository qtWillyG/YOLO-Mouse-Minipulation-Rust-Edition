//! Worker thread: capture -> detect -> pick target -> smooth -> move.
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::backends::{get_cursor, key_down, MouseBackend, MouseButton, SerialMouse, WindowsMouse};
// (screen size comes from Capture's primary-monitor geometry)
use crate::capture::Capture;
use crate::config::{Activation, Backend, Preview, Settings, Shared, TargetMode};
use crate::detector::{Detection, Detector};

pub fn run(shared: Arc<Shared>) {
    let mut cap = Capture::new();
    let mut det = Detector::new();
    let mut win = WindowsMouse;
    let mut ser = SerialMouse::new();

    let mut ema: Option<(f32, f32)> = None;
    let (mut acc_x, mut acc_y) = (0f64, 0f64);
    let mut toggle = false;
    let mut prev_key = false;
    let mut in_dz = false;
    let mut frames = 0u32;
    let mut fps_clock = Instant::now();

    while shared.running.load(Ordering::Relaxed) {
        let tick = Instant::now();
        let s: Settings = shared.settings.lock().unwrap().clone();

        // ---- drain GUI commands ----
        let (load, connect, disconnect) = {
            let mut c = shared.commands.lock().unwrap();
            (c.load_model.take(), c.connect.take(), std::mem::take(&mut c.disconnect))
        };
        if let Some(path) = load {
            set_msg(&shared, "Loading model...");
            match det.load(&path, s.use_gpu) {
                Ok(msg) => set_status(&shared, |st| {
                    st.model_loaded = true;
                    st.provider = det.provider.clone();
                    st.message = msg;
                }),
                Err(e) => set_status(&shared, |st| {
                    st.model_loaded = false;
                    st.message = format!("Model load FAILED: {e}");
                }),
            }
        }
        if let Some(port) = connect {
            let opened = ser.connect(&port);
            let verified = ser.verified;
            set_status(&shared, |st| {
                st.serial_connected = opened;
                st.serial_verified = verified;
                st.message = if opened && verified {
                    format!("Connected + verified on {port}")
                } else if opened {
                    format!("Opened {port} (no firmware reply - check sketch)")
                } else {
                    format!("Failed to open {port}")
                };
            });
        }
        if disconnect {
            ser.disconnect();
            set_status(&shared, |st| {
                st.serial_connected = false;
                st.serial_verified = false;
                st.message = "Serial disconnected".into();
            });
        }

        // ---- capture ----
        let grab = cap.grab(s.full_screen, s.fov_size);

        // ---- detect ----
        let dets: Vec<Detection> = match (&grab, det.is_loaded()) {
            (Some(g), true) => det.infer(&g.rgba, g.w, g.h, s.conf),
            _ => Vec::new(),
        };
        set_status(&shared, |st| st.det_count = dets.len());

        // ---- choose target (screen coords) ----
        let mut target: Option<(f32, f32)> = None;
        if let Some(g) = &grab {
            if !dets.is_empty() {
                let (cx, cy) = get_cursor();
                let reference = match s.target_mode {
                    TargetMode::Center => (
                        (cap.screen_x + cap.screen_w / 2) as f32,
                        (cap.screen_y + cap.screen_h / 2) as f32,
                    ),
                    _ => (cx as f32, cy as f32),
                };
                let mut best_metric = f32::INFINITY;
                for d in &dets {
                    let ox = g.origin_x as f32 + d.x + d.w * 0.5;
                    let oy = g.origin_y as f32 + d.y + d.h * 0.5;
                    let metric = match s.target_mode {
                        TargetMode::Score => -d.score,
                        _ => (ox - reference.0).powi(2) + (oy - reference.1).powi(2),
                    };
                    if metric < best_metric {
                        best_metric = metric;
                        target = Some((ox, oy));
                    }
                }
            }
        }

        // ---- EMA jitter filter ----
        match target {
            Some(t) => {
                ema = Some(match ema {
                    None => t,
                    Some(e) => {
                        let a = s.target_ema.clamp(0.0, 0.95);
                        (a * e.0 + (1.0 - a) * t.0, a * e.1 + (1.0 - a) * t.1)
                    }
                });
            }
            None => ema = None,
        }

        // ---- activation ----
        let kd = key_down(s.activation_vk);
        let mut act = match s.activation {
            Activation::Always => true,
            Activation::Hold => kd,
            Activation::Toggle => {
                if kd && !prev_key {
                    toggle = !toggle;
                }
                toggle
            }
        };
        prev_key = kd;

        let mouse: &mut dyn MouseBackend = match s.backend {
            Backend::Serial => &mut ser,
            Backend::Windows => &mut win,
        };
        act = act && shared.mover_enabled.load(Ordering::Relaxed) && ema.is_some() && mouse.ok();
        set_status(&shared, |st| st.active = act);

        // ---- smoothing / movement ----
        if act {
            let (cx, cy) = get_cursor();
            let e = ema.unwrap();
            let dx = (e.0 - cx as f32) as f64;
            let dy = (e.1 - cy as f32) as f64;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= s.deadzone as f64 {
                if s.click_on_target && !in_dz {
                    mouse.click(MouseButton::Left);
                }
                in_dz = true;
                acc_x = 0.0;
                acc_y = 0.0;
            } else {
                in_dz = false;
                let step = (1.0 - s.smoothing.clamp(0.0, 0.99) as f64) * s.gain as f64;
                let mut mvx = dx * step;
                let mut mvy = dy * step;
                let mlen = (mvx * mvx + mvy * mvy).sqrt();
                let maxs = s.max_speed as f64;
                if mlen > maxs {
                    let k = maxs / mlen;
                    mvx *= k;
                    mvy *= k;
                }
                mvx += acc_x;
                mvy += acc_y;
                let imx = mvx.trunc() as i32;
                let imy = mvy.trunc() as i32;
                acc_x = mvx - imx as f64;
                acc_y = mvy - imy as f64;
                if imx != 0 || imy != 0 {
                    mouse.move_relative(imx, imy);
                }
            }
        } else {
            acc_x = 0.0;
            acc_y = 0.0;
            in_dz = false;
        }

        // ---- preview ----
        if shared.preview_enabled.load(Ordering::Relaxed) {
            if let Some(g) = &grab {
                let mut buf = g.rgba.clone();
                for d in &dets {
                    draw_rect(
                        &mut buf, g.w, g.h, d.x as i32, d.y as i32, d.w as i32, d.h as i32,
                        [0, 255, 0],
                    );
                }
                let (rgba, pw, ph) = downscale(&buf, g.w, g.h, 480);
                *shared.preview.lock().unwrap() = Some(Preview { w: pw, h: ph, rgba });
            }
        }

        // ---- fps + pacing ----
        frames += 1;
        if frames >= 15 {
            let secs = fps_clock.elapsed().as_secs_f32();
            set_status(&shared, |st| st.fps = frames as f32 / secs);
            frames = 0;
            fps_clock = Instant::now();
        }
        let hz = s.tick_hz.clamp(30, 1000) as u64;
        let budget = Duration::from_micros(1_000_000 / hz);
        let spent = tick.elapsed();
        if spent < budget {
            std::thread::sleep(budget - spent);
        }
    }

    ser.disconnect();
}

fn set_status<F: FnOnce(&mut crate::config::Status)>(shared: &Arc<Shared>, f: F) {
    let mut st = shared.status.lock().unwrap();
    f(&mut st);
}
fn set_msg(shared: &Arc<Shared>, m: &str) {
    shared.status.lock().unwrap().message = m.to_string();
}

// ---- tiny RGBA helpers (no image-crate dependency) ----
fn put(buf: &mut [u8], w: usize, h: usize, x: i32, y: i32, c: [u8; 3]) {
    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
        return;
    }
    let i = (y as usize * w + x as usize) * 4;
    buf[i] = c[0];
    buf[i + 1] = c[1];
    buf[i + 2] = c[2];
    buf[i + 3] = 255;
}

fn draw_rect(buf: &mut [u8], w: usize, h: usize, x: i32, y: i32, bw: i32, bh: i32, c: [u8; 3]) {
    for t in 0..2 {
        for dx in 0..bw {
            put(buf, w, h, x + dx, y + t, c);
            put(buf, w, h, x + dx, y + bh - 1 - t, c);
        }
        for dy in 0..bh {
            put(buf, w, h, x + t, y + dy, c);
            put(buf, w, h, x + bw - 1 - t, y + dy, c);
        }
    }
}

/// Nearest-neighbor downscale to a max width, preserving aspect ratio.
fn downscale(src: &[u8], sw: usize, sh: usize, max_w: usize) -> (Vec<u8>, usize, usize) {
    if sw <= max_w {
        return (src.to_vec(), sw, sh);
    }
    let dw = max_w;
    let dh = (sh * max_w / sw).max(1);
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let sy = y * sh / dh;
        for x in 0..dw {
            let sx = x * sw / dw;
            let si = (sy * sw + sx) * 4;
            let di = (y * dw + x) * 4;
            out[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    (out, dw, dh)
}
