
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use app_core::ids::DeviceId;
use elgato_streamdeck::{
    DeviceStateUpdate, list_devices, new_hidapi, AsyncStreamDeck, StreamDeckError,
    images::{convert_image_with_format, ImageRect},
    info::Kind,
};
use image::DynamicImage;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::{ConnectedDevice, ControlEvent, ControlEventKind, ControlId, DeviceEvent, DiscoveredDevice};

fn stable_device_id(kind: Kind, serial: &str) -> DeviceId {
    let mut h = DefaultHasher::new();
    format!("{:?}:{}", kind, serial).hash(&mut h);
    DeviceId(h.finish())
}

pub struct StreamDeckService {
    hid: Arc<hidapi::HidApi>,
}

impl StreamDeckService {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            hid: Arc::new(new_hidapi()?),
        })
    }

    pub async fn list_devices(&self) -> anyhow::Result<Vec<DiscoveredDevice>> {
        let mut out = vec![];
        for (kind, serial) in list_devices(&self.hid) {
            let id = stable_device_id(kind, &serial);
            let name = format!("Stream Deck {:?}", kind);
            out.push(DiscoveredDevice { id, display_name: name });
        }
        Ok(out)
    }

    pub async fn connect(&self, id: DeviceId) -> anyhow::Result<ConnectedDevice> {
        let (event_tx, event_rx) = mpsc::channel(128);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(32);

        // Find the device with matching stable id
        let mut found = None;
        for (kind, serial) in list_devices(&self.hid) {
            if stable_device_id(kind, &serial) == id {
                found = Some((kind, serial));
                break;
            }
        }
        let Some((kind, serial)) = found else {
            anyhow::bail!("device not found");
        };

        let device = AsyncStreamDeck::connect(&self.hid, kind, &serial)?;
        let product_name = device.product().await?;
        let key_count = (kind.row_count() * kind.column_count()) as u8;
        let is_plus = kind == Kind::Plus;

        // Spawn device event handler
        let event_tx_clone = event_tx.clone();
        let reader = device.get_reader();
        tokio::spawn(async move {
            loop {
                match reader.read(100.0).await {
                    Ok(updates) => {
                        for update in updates {
                            match update {
                                DeviceStateUpdate::ButtonDown(key) => {
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::Key(key),
                                        kind: ControlEventKind::Down,
                                    }));
                                }
                                DeviceStateUpdate::ButtonUp(key) => {
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::Key(key),
                                        kind: ControlEventKind::Up,
                                    }));
                                }
                                DeviceStateUpdate::EncoderTwist(dial, ticks) => {
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::Dial(dial),
                                        kind: ControlEventKind::Rotate { delta: ticks as i32 },
                                    }));
                                }
                                DeviceStateUpdate::EncoderDown(dial) => {
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::Dial(dial),
                                        kind: ControlEventKind::Down,
                                    }));
                                }
                                DeviceStateUpdate::EncoderUp(dial) => {
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::Dial(dial),
                                        kind: ControlEventKind::Up,
                                    }));
                                }
                                DeviceStateUpdate::TouchPointDown(_) | DeviceStateUpdate::TouchPointUp(_) => {}
                                DeviceStateUpdate::TouchScreenPress(x, y) => {
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::TouchStrip,
                                        kind: ControlEventKind::Tap { x },
                                    }));
                                }
                                DeviceStateUpdate::TouchScreenLongPress(x, y) => {
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::TouchStrip,
                                        kind: ControlEventKind::Tap { x },
                                    }));
                                }
                                DeviceStateUpdate::TouchScreenSwipe(start, end) => {
                                    let dx = (end.0 as i16) - (start.0 as i16);
                                    let _ = event_tx_clone.send(DeviceEvent::Control(ControlEvent {
                                        control: ControlId::TouchStrip,
                                        kind: ControlEventKind::Drag { delta_x: dx },
                                    }));
                                }
                            }
                        }
                    }
                    Err(_) => {
                        let _ = event_tx_clone.send(DeviceEvent::Disconnected);
                        break;
                    }
                }
            }
        });

        // Spawn command handler
        let device_clone = device.clone();
        let is_plus_clone = is_plus;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    DeviceCommand::SetBrightness { percent, resp } => {
                        let r = device_clone.set_brightness(percent.clamp(0, 100)).await;
                        let _ = resp.send(r.map_err(|e| anyhow::anyhow!(e)));
                    }
                    DeviceCommand::SetKeyImageJpeg { key, jpeg, resp } => {
                        let r = Self::set_key_image(&device_clone, key, jpeg).await;
                        let _ = resp.send(r);
                    }
                    DeviceCommand::SetDialImageJpeg { dial, jpeg, resp } => {
                        let r = if is_plus_clone {
                            Self::set_dial_image(&device_clone, dial, jpeg).await
                        } else {
                            Err(anyhow::anyhow!("dial images are only supported on Stream Deck+"))
                        };
                        let _ = resp.send(r);
                    }
                    DeviceCommand::SetTouchStripImageJpeg { jpeg, resp } => {
                        let r = if is_plus_clone {
                            Self::set_touch_strip_image(&device_clone, jpeg).await
                        } else {
                            Err(anyhow::anyhow!("touch strip images are only supported on Stream Deck+"))
                        };
                        let _ = resp.send(r);
                    }
                }
            }
        });

        Ok(ConnectedDevice {
            id,
            name: product_name,
            key_count,
            events: event_rx,
            handle: StreamDeckHandle { cmd_tx },
        })
    }

    async fn set_key_image(device: &AsyncStreamDeck, key: u8, jpeg: Vec<u8>) -> anyhow::Result<()> {
        let dyn_img = image::load_from_memory(&jpeg)?;
        device.set_button_image(key, dyn_img).await?;
        device.flush().await?;
        Ok(())
    }

    async fn set_dial_image(device: &AsyncStreamDeck, dial: u8, jpeg: Vec<u8>) -> anyhow::Result<()> {
        let dyn_img = image::load_from_memory(&jpeg)?;
        let overlay = render::plus_strip::make_segment_overlay(Some(dyn_img), None);
        let rect = ImageRect::from_image(overlay)?;
        device.write_lcd(dial as u16 * 200, 0, &rect).await?;
        device.flush().await?;
        Ok(())
    }

    async fn set_touch_strip_image(device: &AsyncStreamDeck, jpeg: Vec<u8>) -> anyhow::Result<()> {
        let dyn_img = image::load_from_memory(&jpeg)?;
        let resized = dyn_img.resize_exact(800, 100, image::imageops::FilterType::Nearest);
        let rect = ImageRect::from_image(resized)?;
        device.write_lcd_fill(&rect.data).await?;
        device.flush().await?;
        Ok(())
    }
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
