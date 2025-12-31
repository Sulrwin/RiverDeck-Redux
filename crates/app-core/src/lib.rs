pub mod ids;

use ids::{ActionId, DeviceId, ProfileId};

#[derive(Debug, Clone, Default)]
pub struct AppCore {
    pub selected_device: Option<DeviceId>,
    pub selected_profile: Option<ProfileId>,
    pub selected_action: Option<ActionId>,
}

impl AppCore {
    pub fn new() -> Self {
        Self::default()
    }
}
