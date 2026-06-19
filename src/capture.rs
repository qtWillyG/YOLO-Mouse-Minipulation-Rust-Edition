//! Screen capture via xcap. Returns an RGBA frame plus the screen-space origin
//! of its top-left corner (so detections can be mapped to cursor coordinates).
use xcap::Monitor;

pub struct Grab {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub origin_x: i32,
    pub origin_y: i32,
}

pub struct Capture {
    monitor: Option<Monitor>,
    pub screen_w: i32,
    pub screen_h: i32,
    pub screen_x: i32,
    pub screen_y: i32,
}

impl Capture {
    pub fn new() -> Self {
        Self {
            monitor: None,
            screen_w: 1920,
            screen_h: 1080,
            screen_x: 0,
            screen_y: 0,
        }
    }

    fn ensure(&mut self) {
        if self.monitor.is_none() {
            if let Ok(ms) = Monitor::all() {
                // Pick the primary monitor, else the first - without needing Clone.
                let mut first = None;
                let mut primary = None;
                for m in ms {
                    if m.is_primary() {
                        primary = Some(m);
                        break;
                    }
                    if first.is_none() {
                        first = Some(m);
                    }
                }
                self.monitor = primary.or(first);
            }
        }
    }

    pub fn grab(&mut self, full_screen: bool, fov: i32) -> Option<Grab> {
        self.ensure();
        let m = self.monitor.as_ref()?;
        let img = match m.capture_image() {
            Ok(i) => i,
            Err(_) => {
                self.monitor = None; // force re-enumerate next time
                return None;
            }
        };
        let mw = img.width();
        let mh = img.height();
        let mx = m.x();
        let my = m.y();
        self.screen_w = mw as i32;
        self.screen_h = mh as i32;
        self.screen_x = mx;
        self.screen_y = my;

        let raw = img.into_raw(); // RGBA, len mw*mh*4

        if full_screen {
            return Some(Grab {
                w: mw as usize,
                h: mh as usize,
                rgba: raw,
                origin_x: mx,
                origin_y: my,
            });
        }

        // centered square crop (manual row copy; no image-crate dep here)
        let f = (fov as u32).clamp(64, mw.min(mh));
        let cx = mw / 2 - f / 2;
        let cy = mh / 2 - f / 2;
        let stride = (mw * 4) as usize;
        let row_bytes = (f * 4) as usize;
        let mut out = vec![0u8; (f * f * 4) as usize];
        for row in 0..f {
            let src = ((cy + row) as usize) * stride + (cx as usize) * 4;
            let dst = (row as usize) * row_bytes;
            out[dst..dst + row_bytes].copy_from_slice(&raw[src..src + row_bytes]);
        }
        Some(Grab {
            w: f as usize,
            h: f as usize,
            rgba: out,
            origin_x: mx + cx as i32,
            origin_y: my + cy as i32,
        })
    }
}
