# YoloMouse (Rust)

Rust port of the tool. It captures the screen, detects an object with a
**YOLOv10 `.onnx`** model, and moves the mouse onto it with adjustable
**smoothing**. Same two output backends:

1. **Windows mouse (SendInput)** — pure software, nothing to plug in.
2. **RP2040 / RP2350 USB-HID** — the included Arduino firmware turns the board
   into a real USB mouse; the PC sends it move/click packets over USB-serial.

GUI is **egui/eframe**; inference is the **`ort`** ONNX Runtime crate.

> The firmware and 7-byte serial protocol are identical to the C++ / Python
> versions — a board flashed for any of them works here.

---

## 1. Layout

```
YoloMouseRs/
├─ Cargo.toml
├─ firmware/mouse_hid.ino   <- flash to the RP2040 / RP2350
└─ src/
   ├─ main.rs               <- eframe GUI + entry
   ├─ config.rs             <- Settings + shared state
   ├─ detector.rs           <- YOLOv10 ONNX (ort)
   ├─ capture.rs            <- screen capture (xcap)
   ├─ backends.rs           <- SendInput + serial + Win32 helpers
   └─ worker.rs             <- capture->detect->target->smooth->move loop
```

---

## 2. Prerequisites

- **Windows 10/11 (x64)**
- **Rust** (stable, MSVC toolchain): install from https://rustup.rs
- A C++ build environment is pulled in by the **MSVC** Rust toolchain
  (install "Desktop development with C++" via the Visual Studio Build Tools if
  `rustup` prompts for it).

No manual ONNX Runtime install needed: the `ort` crate's default
`download-binaries` feature fetches a CPU ONNX Runtime at build time.

---

## 3. Build & run

```powershell
cd YoloMouseRs
cargo run --release
```

For a **GPU** (DirectML — any AMD/NVIDIA/Intel GPU on Windows) build:

```powershell
cargo run --release --features gpu
```

First build downloads crates + the ONNX Runtime binary, so it takes a while.

---

## 4. Get a YOLOv10 `.onnx` model

```bash
pip install ultralytics
yolo export model=yolov10n.pt format=onnx opset=13   # or your own best.pt
```

The app expects YOLOv10's end-to-end output `[1, N, 6]`
(`x1,y1,x2,y2,score,class`) at **640×640** input (the default export). If you
exported a different `imgsz`, change `INPUT_SIZE` in `src/detector.rs`.

---

## 5. Flash the firmware (only for the RP2040 / RP2350 backend)

1. Install the **Arduino IDE**.
2. *Preferences → Additional Boards Manager URLs*:
   `https://github.com/earlephilhower/arduino-pico/releases/download/global/package_rp2040_index.json`
   then *Boards Manager* → install **"Raspberry Pi Pico/RP2040/RP2350"**.
3. *Manage Libraries* → install **Adafruit TinyUSB Library**.
4. Pick your board (*Raspberry Pi Pico* or *Pico 2* for RP2350).
5. **Important:** *Tools → USB Stack → "Adafruit TinyUSB"**.
6. Open `firmware/mouse_hid.ino`, hold BOOTSEL while first plugging in, Upload.

---

## 6. Use it

1. **Model:** *Browse* to your `.onnx`, *Load model*.
2. **Output backend:** *Windows* (ready), or *RP2040/RP2350 HID* → pick the COM
   port → *Connect* ("verified" = firmware answered the ping).
3. **Activation:** tick **MOVER ENABLED**, choose *Hold key* (default Right
   Mouse), *Toggle key*, or *Always on*.
4. **Smoothing & movement:** tune to taste (below).
5. Aim at a screen with the target (e.g. a white dot on black). The preview
   shows green detection boxes; the cursor eases onto the chosen target.

### Smoothing controls
| Control | Effect |
|---|---|
| **Smoothing** | 0 = snap instantly; higher = slower, smoother glide |
| **Max speed (px/tick)** | hard cap on cursor movement per tick |
| **Gain** | overall strength multiplier |
| **Deadzone (px)** | stop (and optionally click) once this close |
| **Target jitter filter** | smooths the *target point* to kill detection jitter |
| **Tick rate (Hz)** | how often the loop runs |

---

## 7. Notes

- **`ort` version sensitivity:** the ONNX Runtime crate API still shifts between
  2.0 release candidates. This pins `ort = "=2.0.0-rc.10"`. If you change it and
  it stops compiling, the calls to check are `Session::builder`,
  `Tensor::from_array`, and `try_extract_raw_tensor` in `src/detector.rs`.
- **Nothing moves:** confirm *MOVER ENABLED* is on, you're holding the trigger
  key, and the preview shows a detection (lower *Confidence* if not).
- Multi-monitor capture uses the **primary** monitor.
- **`.onyx` vs `.onnx`:** the format is `.onnx`; rename if needed.
