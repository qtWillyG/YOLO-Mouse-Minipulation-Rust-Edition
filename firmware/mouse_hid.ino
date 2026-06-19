/*
 * mouse_hid.ino  -  USB HID mouse bridge for RP2040 / RP2350
 * ------------------------------------------------------------
 * Receives movement / click commands over USB-serial (CDC) and
 * replays them as a real USB HID mouse, so to the host PC the
 * cursor moves exactly like a physical mouse.
 *
 * Works on both the RP2040 and the RP2350 using the
 * "Raspberry Pi Pico/RP2040/RP2350" core by Earle Philhower.
 *
 * REQUIRED Arduino IDE settings:
 *   Tools -> USB Stack -> "Adafruit TinyUSB"
 *   (this lets us expose Serial (CDC) AND a HID mouse at once)
 *
 * Install the library:  Sketch -> Include Library -> Manage Libraries
 *   -> search "Adafruit TinyUSB" and install.
 *
 * ------------------------------------------------------------
 * Wire protocol (fixed 7-byte packets, little-endian):
 *
 *   byte0 : 0xAA                      sync
 *   byte1 : cmd  ('M' move, 'B' button, 'C' click, 'P' ping)
 *   byte2 : d0
 *   byte3 : d1
 *   byte4 : d2
 *   byte5 : d3
 *   byte6 : checksum = byte1 ^ byte2 ^ byte3 ^ byte4 ^ byte5
 *
 *   'M' move   : d0,d1 = int16 dx ; d2,d3 = int16 dy
 *   'B' button : d0 = button(0=L,1=R,2=M) ; d1 = state(1 down,0 up)
 *   'C' click  : d0 = button ; (press + release)
 *   'P' ping   : firmware replies with a single byte 'K' (0x4B)
 * ------------------------------------------------------------
 */

#include <Adafruit_TinyUSB.h>

// ---- HID mouse descriptor (no report id -> use report_id 0) ----
static const uint8_t kDescHidReport[] = { TUD_HID_REPORT_DESC_MOUSE() };
Adafruit_USBD_HID usb_hid;

// ---- button bit masks (match TinyUSB MOUSE_BUTTON_*) ----
static const uint8_t kBtnMask[3] = {
  MOUSE_BUTTON_LEFT,
  MOUSE_BUTTON_RIGHT,
  MOUSE_BUTTON_MIDDLE
};

static uint8_t  gButtons = 0;          // current held-button bitmap

// ---- packet receive state machine ----
static uint8_t  gBuf[7];
static uint8_t  gIdx = 0;

// Send a single HID report carrying a clamped move (-127..127).
static void sendReport(int8_t dx, int8_t dy) {
  // Wait briefly for the endpoint to be ready so we never drop a report.
  for (uint8_t tries = 0; tries < 50 && !usb_hid.ready(); ++tries) delay(1);
  if (usb_hid.ready()) {
    usb_hid.mouseReport(0, gButtons, dx, dy, 0, 0);
  }
}

// Move by an arbitrary int16 delta, split into HID-legal -127..127 chunks.
static void moveMouse(int16_t dx, int16_t dy) {
  while (dx != 0 || dy != 0) {
    int8_t cx = (dx >  127) ?  127 : (dx < -127) ? -127 : (int8_t)dx;
    int8_t cy = (dy >  127) ?  127 : (dy < -127) ? -127 : (int8_t)dy;
    sendReport(cx, cy);
    dx -= cx;
    dy -= cy;
  }
}

static void setButton(uint8_t id, bool down) {
  if (id > 2) return;
  if (down) gButtons |=  kBtnMask[id];
  else      gButtons &= ~kBtnMask[id];
  sendReport(0, 0);            // report the new button state, no movement
}

static void clickButton(uint8_t id) {
  setButton(id, true);
  delay(15);
  setButton(id, false);
}

// Decode one validated 7-byte packet in gBuf.
static void handlePacket() {
  uint8_t chk = gBuf[1] ^ gBuf[2] ^ gBuf[3] ^ gBuf[4] ^ gBuf[5];
  if (chk != gBuf[6]) return;            // corrupt -> ignore

  switch (gBuf[1]) {
    case 'M': {                          // move
      int16_t dx = (int16_t)(gBuf[2] | (gBuf[3] << 8));
      int16_t dy = (int16_t)(gBuf[4] | (gBuf[5] << 8));
      moveMouse(dx, dy);
      break;
    }
    case 'B':                            // button down/up
      setButton(gBuf[2], gBuf[3] != 0);
      break;
    case 'C':                            // click
      clickButton(gBuf[2]);
      break;
    case 'P':                            // ping
      Serial.write('K');
      break;
  }
}

void setup() {
  // Some cores need the device re-attached after configuring TinyUSB.
  if (!TinyUSBDevice.isInitialized()) TinyUSBDevice.begin(0);

  usb_hid.setBootProtocol(HID_ITF_PROTOCOL_MOUSE);
  usb_hid.setPollInterval(2);
  usb_hid.setReportDescriptor(kDescHidReport, sizeof(kDescHidReport));
  usb_hid.begin();

  Serial.begin(115200);

  // Wait until the host has enumerated the device.
  while (!TinyUSBDevice.mounted()) delay(1);
}

void loop() {
  // Keep TinyUSB serviced (required on some cores).
  #ifdef TINYUSB_NEED_POLLING_TASK
  TinyUSBDevice.task();
  #endif

  while (Serial.available()) {
    uint8_t b = (uint8_t)Serial.read();

    if (gIdx == 0) {
      if (b == 0xAA) gBuf[gIdx++] = b;   // wait for sync byte
    } else {
      gBuf[gIdx++] = b;
      if (gIdx >= 7) {                   // full packet collected
        handlePacket();
        gIdx = 0;
      }
    }
  }
}
