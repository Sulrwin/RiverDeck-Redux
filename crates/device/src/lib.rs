mod stream_deck;

use app_core::ids::DeviceId;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub id: DeviceId,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub enum DeviceEvent {
    KeyDown { key: u8 },
    KeyUp { key: u8 },
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
}
