//! YOLOv10 ONNX inference via the `ort` crate.
//!
//! YOLOv10 is end-to-end / NMS-free: the output is already filtered and shaped
//! [1, N, 6] = (x1, y1, x2, y2, score, classId) in input-pixel space.
//!
//! NOTE: the `ort` 2.0 API still moves between release candidates. This targets
//! ort = "=2.0.0-rc.10". If you bump the version, the three calls most likely to
//! need a tweak are: `Session::builder`, `Tensor::from_array`, and
//! `try_extract_raw_tensor` (extraction).
use image::{imageops, Rgb, RgbImage};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;

const INPUT_SIZE: u32 = 640; // YOLOv10 default imgsz; change if you exported another

pub struct Detection {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub class: i32,
}

pub struct Detector {
    session: Option<Session>,
    input_name: String,
    pub provider: String,
}

impl Detector {
    pub fn new() -> Self {
        Self {
            session: None,
            input_name: String::new(),
            provider: "none".into(),
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.session.is_some()
    }

    pub fn load(&mut self, path: &str, use_gpu: bool) -> Result<String, String> {
        let mut builder = Session::builder()
            .map_err(|e| e.to_string())?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| e.to_string())?;

        if use_gpu {
            use ort::execution_providers::{CUDAExecutionProvider, DirectMLExecutionProvider};
            // Registration of an unavailable EP is non-fatal: ort falls back to CPU.
            builder = builder
                .with_execution_providers([
                    DirectMLExecutionProvider::default().build(),
                    CUDAExecutionProvider::default().build(),
                ])
                .map_err(|e| e.to_string())?;
        }

        let session = builder.commit_from_file(path).map_err(|e| e.to_string())?;
        self.input_name = session
            .inputs
            .get(0)
            .map(|i| i.name.clone())
            .ok_or_else(|| "model has no inputs".to_string())?;
        self.session = Some(session);
        self.provider = if use_gpu {
            "GPU (DirectML/CUDA if available, else CPU)".into()
        } else {
            "CPU".into()
        };
        Ok(format!(
            "Model loaded ({}, input {}x{})",
            self.provider, INPUT_SIZE, INPUT_SIZE
        ))
    }

    /// `rgba` is tightly packed RGBA, `w`x`h`. Returns boxes in that image's px.
    pub fn infer(&self, rgba: &[u8], w: usize, h: usize, conf: f32) -> Vec<Detection> {
        let session = match &self.session {
            Some(s) => s,
            None => return Vec::new(),
        };
        if rgba.len() < w * h * 4 {
            return Vec::new();
        }

        // RGBA -> RgbImage
        let mut src = RgbImage::new(w as u32, h as u32);
        for (i, px) in src.pixels_mut().enumerate() {
            *px = Rgb([rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]]);
        }

        // letterbox to INPUT_SIZE
        let (w0, h0) = (w as f32, h as f32);
        let scale = (INPUT_SIZE as f32 / w0).min(INPUT_SIZE as f32 / h0);
        let nw = (w0 * scale).round() as u32;
        let nh = (h0 * scale).round() as u32;
        let padx = (INPUT_SIZE - nw) / 2;
        let pady = (INPUT_SIZE - nh) / 2;

        let resized = imageops::resize(&src, nw, nh, imageops::FilterType::Triangle);
        let mut canvas = RgbImage::from_pixel(INPUT_SIZE, INPUT_SIZE, Rgb([114, 114, 114]));
        imageops::overlay(&mut canvas, &resized, padx as i64, pady as i64);

        // HWC u8 -> CHW f32 normalized
        let area = (INPUT_SIZE * INPUT_SIZE) as usize;
        let mut input = vec![0f32; 3 * area];
        for y in 0..INPUT_SIZE {
            for x in 0..INPUT_SIZE {
                let p = canvas.get_pixel(x, y);
                let idx = (y * INPUT_SIZE + x) as usize;
                input[idx] = p[0] as f32 / 255.0;
                input[area + idx] = p[1] as f32 / 255.0;
                input[2 * area + idx] = p[2] as f32 / 255.0;
            }
        }

        let tensor = match Tensor::from_array((
            [1usize, 3, INPUT_SIZE as usize, INPUT_SIZE as usize],
            input,
        )) {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };

        let inputs = match ort::inputs![self.input_name.as_str() => tensor] {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        let outputs = match session.run(inputs) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };
        let (shape, data) = match outputs[0].try_extract_raw_tensor::<f32>() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        // Normalize the shape into a Vec<i64> regardless of ort's exact return type.
        let dims: Vec<i64> = shape.iter().copied().collect();
        decode(&dims, data, scale, padx as f32, pady as f32, conf)
    }
}

fn decode(
    shape: &[i64],
    data: &[f32],
    scale: f32,
    padx: f32,
    pady: f32,
    conf: f32,
) -> Vec<Detection> {
    let mut out = Vec::new();
    let mut push = |x1: f32, y1: f32, x2: f32, y2: f32, score: f32, cls: f32| {
        if score < conf {
            return;
        }
        out.push(Detection {
            x: (x1 - padx) / scale,
            y: (y1 - pady) / scale,
            w: (x2 - x1) / scale,
            h: (y2 - y1) / scale,
            score,
            class: cls.round() as i32,
        });
    };

    if shape.len() == 3 && shape[2] == 6 {
        // [1, N, 6]
        let n = shape[1] as usize;
        for i in 0..n {
            let r = &data[i * 6..i * 6 + 6];
            push(r[0], r[1], r[2], r[3], r[4], r[5]);
        }
    } else if shape.len() == 3 && shape[1] == 6 {
        // [1, 6, N] (transposed)
        let n = shape[2] as usize;
        for i in 0..n {
            push(
                data[i],
                data[n + i],
                data[2 * n + i],
                data[3 * n + i],
                data[4 * n + i],
                data[5 * n + i],
            );
        }
    } else if shape.len() == 2 && shape[1] == 6 {
        // [N, 6]
        let n = shape[0] as usize;
        for i in 0..n {
            let r = &data[i * 6..i * 6 + 6];
            push(r[0], r[1], r[2], r[3], r[4], r[5]);
        }
    }
    out
}
