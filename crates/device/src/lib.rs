mod stream_deck;

use app_core::ids::DeviceId;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub id: DeviceId,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    /// Physical LCD key (Stream Deck+ has 8 keys, indexed 0..7).
    Key(u8),
    /// Dial control (Stream Deck+ has 4 dials, indexed 0..3).
    Dial(u8),
    /// Touch strip (single surface).
    TouchStrip,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlEventKind {
    Down,
    Up,
    /// Dial rotation delta. Negative = left/CCW, positive = right/CW.
    Rotate { delta: i32 },
    /// Touch strip tap position in device coordinates (0..width-1).
    Tap { x: u16 },
    /// Touch strip drag delta in device coordinates. Negative = left, positive = right.
    Drag { delta_x: i16 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlEvent {
    pub control: ControlId,
    pub kind: ControlEventKind,
}

#[derive(Debug, Clone)]
pub enum DeviceEvent {
    Control(ControlEvent),
    Disconnected,
}

#[async_trait]
pub trait DeviceService: Send + Sync {
    async fn list_devices(&self) -> anyhow::Result<Vec<DiscoveredDevice>>;
}

pub struct HidDeviceService {
    inner: stream_deck::StreamDeckService,
}

impl HidDeviceService {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            inner: stream_deck::StreamDeckService::new()?,
        })
    }

    /// Connect to a device and start streaming events.
    pub async fn connect(&self, id: DeviceId) -> anyhow::Result<ConnectedDevice> {
        self.inner.connect(id).await
    }
}

#[async_trait]
impl DeviceService for HidDeviceService {
    async fn list_devices(&self) -> anyhow::Result<Vec<DiscoveredDevice>> {
        self.inner.list_devices().await
    }
}

pub struct ConnectedDevice {
    pub id: DeviceId,
    pub name: String,
    pub key_count: u8,
    pub events: tokio::sync::mpsc::Receiver<DeviceEvent>,
    handle: stream_deck::StreamDeckHandle,
}

impl ConnectedDevice {
    pub fn controller(&self) -> DeviceController {
        DeviceController {
            handle: self.handle.clone(),
        }
    }

    pub async fn set_brightness(&self, percent: u8) -> anyhow::Result<()> {
        self.handle.set_brightness(percent).await
    }

    pub async fn set_key_image_jpeg(&self, key: u8, jpeg_bytes: Vec<u8>) -> anyhow::Result<()> {
        self.handle.set_key_image_jpeg(key, jpeg_bytes).await
    }

    /// Stream Deck+ only: set the LCD image for a dial (0..3).
    pub async fn set_dial_image_jpeg(&self, dial: u8, jpeg_bytes: Vec<u8>) -> anyhow::Result<()> {
        self.handle.set_dial_image_jpeg(dial, jpeg_bytes).await
    }

    /// Stream Deck+ only: set the LCD image for the touch strip.
    pub async fn set_touch_strip_image_jpeg(&self, jpeg_bytes: Vec<u8>) -> anyhow::Result<()> {
        self.handle.set_touch_strip_image_jpeg(jpeg_bytes).await
    }
}

#[derive(Clone)]
pub struct DeviceController {
    handle: stream_deck::StreamDeckHandle,
}

impl DeviceController {
    pub async fn set_brightness(&self, percent: u8) -> anyhow::Result<()> {
        self.handle.set_brightness(percent).await
    }

    pub async fn set_key_image_jpeg(&self, key: u8, jpeg_bytes: Vec<u8>) -> anyhow::Result<()> {
        self.handle.set_key_image_jpeg(key, jpeg_bytes).await
    }

    pub async fn set_dial_image_jpeg(&self, dial: u8, jpeg_bytes: Vec<u8>) -> anyhow::Result<()> {
        self.handle.set_dial_image_jpeg(dial, jpeg_bytes).await
    }

    pub async fn set_touch_strip_image_jpeg(&self, jpeg_bytes: Vec<u8>) -> anyhow::Result<()> {
        self.handle.set_touch_strip_image_jpeg(jpeg_bytes).await
    }
}
