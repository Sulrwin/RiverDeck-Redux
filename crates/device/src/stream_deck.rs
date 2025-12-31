use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::thread;
use std::time::Duration;

use app_core::ids::DeviceId;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};
use transport_hid::{HidContext, HidDeviceHandle, HidDiscoveredDevice};

use crate::{ConnectedDevice, ControlEvent, ControlEventKind, ControlId, DeviceEvent, DiscoveredDevice};

// Elgato Systems GmbH
const ELGATO_VENDOR_ID: u16 = 0x0fd9;

/// Product IDs we know about. We also fall back to product string matching.
///
/// This list is intentionally conservative; adding unknown PIDs is safe as long
/// as we validate strings/behaviour on connect.
const KNOWN_STREAMDECK_PRODUCT_IDS: &[u16] = &[
    0x0060, // Stream Deck (original)
    0x0063, // Stream Deck Mini
    0x006c, // Stream Deck XL (commonly reported)
    0x0080, // Stream Deck MK.2 (commonly reported)
];

#[derive(Debug, Clone)]
struct Candidate {
    id: DeviceId,
    name: String,
    product_id: u16,
    interface_number: Option<i32>,
    path: Vec<u8>,
}

fn stable_device_id(vendor_id: u16, product_id: u16, serial: &Option<String>) -> DeviceId {
    let mut h = DefaultHasher::new();
    vendor_id.hash(&mut h);
    product_id.hash(&mut h);
    serial.hash(&mut h);
    DeviceId(h.finish())
}

fn looks_like_stream_deck(d: &HidDiscoveredDevice) -> bool {
    if d.vendor_id != ELGATO_VENDOR_ID {
        return false;
    }

    if KNOWN_STREAMDECK_PRODUCT_IDS.contains(&d.product_id) {
        return true;
    }

    let p = d
        .product_string
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    p.contains("stream deck")
}

pub struct StreamDeckService {
    ctx: HidContext,
}

impl StreamDeckService {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            ctx: HidContext::new()?,
        })
    }

    pub async fn list_devices(&self) -> anyhow::Result<Vec<DiscoveredDevice>> {
        let mut out = vec![];
        for d in self.ctx.list_all() {
            if !looks_like_stream_deck(&d) {
                continue;
            }

            let id = stable_device_id(d.vendor_id, d.product_id, &d.serial_number);
            let name = d
                .product_string
                .clone()
                .or_else(|| {
                    Some(format!(
                        "Stream Deck ({:04x}:{:04x})",
                        d.vendor_id, d.product_id
                    ))
                })
                .unwrap();

            // Only expose one row per physical device (multiple HID interfaces may exist).
            if out.iter().any(|x: &DiscoveredDevice| x.id == id) {
                continue;
            }

            out.push(DiscoveredDevice {
                id,
                display_name: name,
            });
        }

        Ok(out)
    }

    pub async fn connect(&self, id: DeviceId) -> anyhow::Result<ConnectedDevice> {
        // Collect all HID entries that map to the same DeviceId.
        let mut candidates: Vec<Candidate> = self
            .ctx
            .list_all()
            .into_iter()
            .filter(looks_like_stream_deck)
            .map(|d| Candidate {
                id: stable_device_id(d.vendor_id, d.product_id, &d.serial_number),
                name: d
                    .product_string
                    .clone()
                    .unwrap_or_else(|| "Stream Deck".to_string()),
                product_id: d.product_id,
                interface_number: d.interface_number,
                path: d.path,
            })
            .filter(|c| c.id == id)
            .collect();

        if candidates.is_empty() {
            anyhow::bail!("device not found");
        }

        // Heuristic selection:
        // - Stream Deck devices often expose multiple HID interfaces.
        // - A common pattern is interface 0 for write and interface 1 for read.
        candidates.sort_by_key(|c| c.interface_number.unwrap_or(999));

        let write_path = candidates
            .iter()
            .find(|c| c.interface_number == Some(0))
            .or_else(|| candidates.first())
            .map(|c| c.path.clone())
            .unwrap();
        let read_path = candidates
            .iter()
            .find(|c| c.interface_number == Some(1))
            .or_else(|| candidates.last())
            .map(|c| c.path.clone())
            .unwrap();

        let name = candidates[0].name.clone();
        let product_lower = name.to_ascii_lowercase();
        let is_plus = product_lower.contains("plus") || product_lower.contains("stream deck +");
        let key_count = match candidates[0].product_id {
            0x0063 => 6,  // Mini
            0x006c => 32, // XL (common)
            _ => {
                // Heuristic: Stream Deck+ is 8 keys (2x4) + touch strip + 4 dials.
                if is_plus {
                    8
                } else {
                    15 // Original/MK.2 default
                }
            }
        };

        let (event_tx, event_rx) = mpsc::channel(128);
        let (cmd_tx, cmd_rx) = mpsc::channel(32);

        let ctx = HidContext::new()?;

        thread::spawn(move || {
            if let Err(err) =
                run_stream_deck_thread(ctx, read_path, write_path, key_count, is_plus, event_tx, cmd_rx)
            {
                warn!(?err, "device thread terminated with error");
            }
        });

        Ok(ConnectedDevice {
            id,
            name,
            key_count,
            events: event_rx,
            handle: StreamDeckHandle { cmd_tx },
        })
    }
}

fn run_stream_deck_thread(
    ctx: HidContext,
    read_path: Vec<u8>,
    write_path: Vec<u8>,
    key_count: u8,
    is_plus: bool,
    event_tx: mpsc::Sender<DeviceEvent>,
    mut cmd_rx: mpsc::Receiver<DeviceCommand>,
) -> anyhow::Result<()> {
    let mut read_dev = ctx.open_path(&read_path)?;
    let mut write_dev = ctx.open_path(&write_path)?;

    // Avoid fully blocking in read so we can also process commands.
    read_dev.set_blocking_mode(false)?;

    debug!(key_count, "stream deck device thread started");

    let mut last_keys: Vec<bool> = vec![false; key_count as usize];
    let mut last_dials_down: [bool; 4] = [false; 4];
    let mut buf = vec![0u8; 64];

    loop {
        // If the UI dropped both the event receiver and the command sender, exit cleanly.
        if event_tx.is_closed() && cmd_rx.is_closed() {
            return Ok(());
        }

        // 1) Poll for commands.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                DeviceCommand::SetBrightness { percent, resp } => {
                    let r = set_brightness_v1(&mut write_dev, percent);
                    let _ = resp.send(r);
                }
                DeviceCommand::SetKeyImageJpeg { key, jpeg, resp } => {
                    let r = set_key_image_v1_jpeg(&mut write_dev, key, &jpeg);
                    let _ = resp.send(r);
                }
                DeviceCommand::SetDialImageJpeg { dial, jpeg, resp } => {
                    // MVP heuristic for Stream Deck+:
                    // treat dial LCDs as "virtual keys" after the 8 physical keys.
                    let key = 8u8.saturating_add(dial.min(3));
                    let r = set_key_image_v1_jpeg(&mut write_dev, key, &jpeg);
                    let _ = resp.send(r);
                }
                DeviceCommand::SetTouchStripImageJpeg { jpeg, resp } => {
                    // MVP heuristic for Stream Deck+:
                    // treat touch strip as the next "virtual key" after keys+dials.
                    let key = 12u8;
                    let r = set_key_image_v1_jpeg(&mut write_dev, key, &jpeg);
                    let _ = resp.send(r);
                }
            }
        }

        // 2) Read key state report (non-blocking).
        match read_dev.read_timeout(&mut buf, 20) {
            Ok(n) if n > 0 => {
                let report = &buf[..n];
                if let Some(states) = parse_key_states_v1(report, key_count) {
                    emit_key_events(&mut last_keys, &states, &event_tx);
                } else if is_plus {
                    emit_plus_events(report, &mut last_dials_down, &event_tx);
                }
            }
            Ok(_) => {}
            Err(_err) => {
                // Most platforms return timeout as Ok(0); treat other errors as disconnect.
                let _ = event_tx.blocking_send(DeviceEvent::Disconnected);
                return Ok(());
            }
        }

        thread::sleep(Duration::from_millis(5));
    }
}

fn emit_key_events(last: &mut [bool], current: &[bool], tx: &mpsc::Sender<DeviceEvent>) {
    for (i, (&prev, &now)) in last.iter().zip(current.iter()).enumerate() {
        if prev == now {
            continue;
        }
        let key = i as u8;
        let kind = if now {
            ControlEventKind::Down
        } else {
            ControlEventKind::Up
        };
        let ev = DeviceEvent::Control(ControlEvent {
            control: ControlId::Key(key),
            kind,
        });
        let _ = tx.blocking_send(ev);
    }
    last.copy_from_slice(current);
}

fn parse_key_states_v1(report: &[u8], key_count: u8) -> Option<Vec<bool>> {
    // Common key state report format for 15-key Stream Deck family:
    // report[0] = 0x01, report[1..] = 1 byte per key (0/1)
    if report.is_empty() {
        return None;
    }
    if report[0] != 0x01 {
        return None;
    }
    if report.len() < 1 + key_count as usize {
        return None;
    }
    Some(
        report[1..1 + key_count as usize]
            .iter()
            .map(|b| *b != 0)
            .collect(),
    )
}

fn emit_plus_events(report: &[u8], last_dials_down: &mut [bool; 4], tx: &mpsc::Sender<DeviceEvent>) {
    // Stream Deck+ support in this repo is intentionally MVP-level.
    //
    // The device exposes additional HID reports for dials and the touch strip.
    // We parse a few conservative patterns and ignore anything unknown.
    //
    // Note: The exact report formats vary by firmware/platform; this is a best-effort decoder.

    if report.is_empty() {
        return;
    }

    match report[0] {
        // Heuristic: some firmwares append dial-press bytes after key bytes in the same 0x01 report.
        0x01 => {
            // report[1..] starts with key_count bytes; if there are 4 more bytes, treat them as dial-down states.
            if report.len() < 1 + 8 + 4 {
                return;
            }
            let dial_start = 1 + 8;
            for i in 0..4usize {
                let now = report.get(dial_start + i).copied().unwrap_or(0) != 0;
                if last_dials_down[i] == now {
                    continue;
                }
                last_dials_down[i] = now;
                let ev = DeviceEvent::Control(ControlEvent {
                    control: ControlId::Dial(i as u8),
                    kind: if now { ControlEventKind::Down } else { ControlEventKind::Up },
                });
                let _ = tx.blocking_send(ev);
            }
        }
        // Heuristic: dial rotation report: [0x03, dial_index, delta_i8]
        0x03 if report.len() >= 3 => {
            let dial = report[1].min(3);
            let delta = report[2] as i8 as i32;
            if delta != 0 {
                let _ = tx.blocking_send(DeviceEvent::Control(ControlEvent {
                    control: ControlId::Dial(dial),
                    kind: ControlEventKind::Rotate { delta },
                }));
            }
        }
        // Heuristic: touch strip tap report: [0x04, x_lo, x_hi]
        0x04 if report.len() >= 3 => {
            let x = u16::from_le_bytes([report[1], report[2]]);
            let _ = tx.blocking_send(DeviceEvent::Control(ControlEvent {
                control: ControlId::TouchStrip,
                kind: ControlEventKind::Tap { x },
            }));
        }
        // Heuristic: touch strip drag report: [0x05, dx_lo, dx_hi]
        0x05 if report.len() >= 3 => {
            let dx = i16::from_le_bytes([report[1], report[2]]);
            if dx != 0 {
                let _ = tx.blocking_send(DeviceEvent::Control(ControlEvent {
                    control: ControlId::TouchStrip,
                    kind: ControlEventKind::Drag { delta_x: dx },
                }));
            }
        }
        _ => {}
    }
}

fn set_brightness_v1(dev: &mut HidDeviceHandle, percent: u8) -> anyhow::Result<()> {
    let p = percent.min(100);
    let mut report = [0u8; 17];
    report[0] = 0x05;
    report[1] = 0x55;
    report[2] = p;
    dev.send_feature_report(&report)?;
    Ok(())
}

fn set_key_image_v1_jpeg(dev: &mut HidDeviceHandle, key: u8, jpeg: &[u8]) -> anyhow::Result<()> {
    // Output report format used by many 15-key Stream Deck devices:
    // report_id = 0x02, payload chunked into fixed-size reports.
    //
    // This is intentionally minimal for MVP: it pushes raw JPEG bytes and relies on
    // device-side scaling/cropping expectations being met by the caller.
    const REPORT_LEN: usize = 1024;
    const HEADER_LEN: usize = 8;
    let max_payload = REPORT_LEN - HEADER_LEN;

    let mut page: u16 = 0;
    let mut offset = 0usize;
    while offset < jpeg.len() {
        let remaining = jpeg.len() - offset;
        let take = remaining.min(max_payload);
        let is_last = (offset + take) >= jpeg.len();

        let mut report = vec![0u8; REPORT_LEN];
        report[0] = 0x02;
        report[1] = 0x01;
        report[2] = key;
        report[3] = if is_last { 1 } else { 0 };
        report[4] = (page & 0xff) as u8;
        report[5] = (page >> 8) as u8;
        report[6] = (take & 0xff) as u8;
        report[7] = (take >> 8) as u8;
        report[HEADER_LEN..HEADER_LEN + take].copy_from_slice(&jpeg[offset..offset + take]);

        dev.write(&report)?;

        offset += take;
        page = page.wrapping_add(1);
    }

    Ok(())
}

enum DeviceCommand {
    SetBrightness {
        percent: u8,
        resp: oneshot::Sender<anyhow::Result<()>>,
    },
    SetKeyImageJpeg {
        key: u8,
        jpeg: Vec<u8>,
        resp: oneshot::Sender<anyhow::Result<()>>,
    },
    SetDialImageJpeg {
        dial: u8,
        jpeg: Vec<u8>,
        resp: oneshot::Sender<anyhow::Result<()>>,
    },
    SetTouchStripImageJpeg {
        jpeg: Vec<u8>,
        resp: oneshot::Sender<anyhow::Result<()>>,
    },
}

#[derive(Clone)]
pub struct StreamDeckHandle {
    cmd_tx: mpsc::Sender<DeviceCommand>,
}

impl StreamDeckHandle {
    pub async fn set_brightness(&self, percent: u8) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DeviceCommand::SetBrightness { percent, resp: tx })
            .await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?
    }

    pub async fn set_key_image_jpeg(&self, key: u8, jpeg: Vec<u8>) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DeviceCommand::SetKeyImageJpeg {
                key,
                jpeg,
                resp: tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?
    }

    pub async fn set_dial_image_jpeg(&self, dial: u8, jpeg: Vec<u8>) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DeviceCommand::SetDialImageJpeg {
                dial,
                jpeg,
                resp: tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?
    }

    pub async fn set_touch_strip_image_jpeg(&self, jpeg: Vec<u8>) -> anyhow::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(DeviceCommand::SetTouchStripImageJpeg { jpeg, resp: tx })
            .await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("device thread stopped"))?
    }
}
