//! HID transport layer (Linux + Windows) built on `hidapi`.

use std::ffi::CStr;

use hidapi::{DeviceInfo, HidApi, HidDevice};

#[derive(Debug, Clone)]
pub struct HidDiscoveredDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product_string: Option<String>,
    pub manufacturer_string: Option<String>,
    pub serial_number: Option<String>,
    pub interface_number: Option<i32>,
    pub path: Vec<u8>,
}

impl HidDiscoveredDevice {
    fn from_info(info: &DeviceInfo) -> Self {
        let if_num = info.interface_number();
        Self {
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            product_string: info.product_string().map(|s| s.to_string()),
            manufacturer_string: info.manufacturer_string().map(|s| s.to_string()),
            serial_number: info.serial_number().map(|s| s.to_string()),
            interface_number: (if_num >= 0).then_some(if_num),
            path: info.path().to_bytes_with_nul().to_vec(),
        }
    }
}

pub struct HidContext {
    api: HidApi,
}

impl HidContext {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            api: HidApi::new()?,
        })
    }

    pub fn list_all(&self) -> Vec<HidDiscoveredDevice> {
        self.api
            .device_list()
            .map(HidDiscoveredDevice::from_info)
            .collect()
    }

    pub fn open_path(&self, path: &[u8]) -> anyhow::Result<HidDeviceHandle> {
        let cstr = CStr::from_bytes_with_nul(path)?;
        Ok(HidDeviceHandle {
            inner: self.api.open_path(cstr)?,
        })
    }
}

/// Thin wrapper so downstream crates don't need to depend on `hidapi` directly.
pub struct HidDeviceHandle {
    inner: HidDevice,
}

impl HidDeviceHandle {
    pub fn set_blocking_mode(&mut self, blocking: bool) -> anyhow::Result<()> {
        self.inner.set_blocking_mode(blocking)?;
        Ok(())
    }

    pub fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> anyhow::Result<usize> {
        Ok(self.inner.read_timeout(buf, timeout_ms)?)
    }

    pub fn write(&mut self, buf: &[u8]) -> anyhow::Result<usize> {
        Ok(self.inner.write(buf)?)
    }

    pub fn send_feature_report(&mut self, report: &[u8]) -> anyhow::Result<()> {
        Ok(self.inner.send_feature_report(report)?)
    }
}

// (no additional HidContext impls)
