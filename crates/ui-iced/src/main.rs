use std::time::Duration;
use std::{fmt, sync::Arc};

use app_core::AppCore;
use device::{DeviceController, DeviceEvent, DeviceService, DiscoveredDevice, HidDeviceService};
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, horizontal_space, pick_list, row,
    scrollable, slider, text, text_input,
};
use iced::{
    alignment::Horizontal, Alignment, Application, Background, Border, Color, Command, Element,
    Length, Settings, Shadow, Theme,
};
use tokio::sync::mpsc::Receiver;

use app_core::ids::ProfileId;
use storage::profiles::{ActionBinding, Profile, ProfileMeta};

use openaction::manifest::{ActionDefinition, SettingField, SettingType};
use openaction::marketplace::MarketplacePlugin;
use openaction::registry::InstalledPlugin;
use plugin_runtime::ActionRuntime;

fn main() -> iced::Result {
    init_tracing();

    App::run(Settings {
        window: iced::window::Settings {
            size: iced::Size::new(1240.0, 760.0),
            ..Default::default()
        },
        ..Default::default()
    })
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
}

struct App {
    core: AppCore,
    devices: Vec<DiscoveredDevice>,
    device_choices: Vec<DeviceChoice>,
    selected_device: Option<app_core::ids::DeviceId>,
    connecting: bool,
    connected: Option<ConnectedUi>,
    profiles: Vec<ProfileMeta>,
    profile_choices: Vec<ProfileChoice>,
    selected_profile: Option<ProfileId>,
    profile: Option<Profile>,
    selected_key: Option<usize>,
    edit_label: String,
    plugins: Vec<InstalledPlugin>,
    actions: Vec<ActionChoice>,
    action_search: String,
    install_plugin_path: String,
    active_view: ActiveView,
    marketplace: MarketplaceState,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    Main,
    Marketplace,
}

#[derive(Debug, Clone)]
struct MarketplaceState {
    sources: Vec<MarketplaceSource>,
    selected_source_idx: Option<usize>,
    loading: bool,
    plugins: Vec<MarketplacePlugin>,
    query: String,
    error: Option<String>,
    new_source_name: String,
    new_source_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MarketplaceSource {
    name: String,
    index_url: String,
}

impl fmt::Display for MarketplaceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl Application for App {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let sources = default_marketplace_sources();
        let selected_source_idx = if sources.is_empty() { None } else { Some(0) };

        let app = Self {
            core: AppCore::new(),
            devices: vec![],
            device_choices: vec![],
            selected_device: None,
            connecting: false,
            connected: None,
            profiles: vec![],
            profile_choices: vec![],
            selected_profile: None,
            profile: None,
            selected_key: None,
            edit_label: String::new(),
            plugins: vec![],
            actions: vec![],
            action_search: String::new(),
            install_plugin_path: String::new(),
            active_view: ActiveView::Main,
            marketplace: MarketplaceState {
                sources,
                selected_source_idx,
                loading: false,
                plugins: vec![],
                query: String::new(),
                error: None,
                new_source_name: String::new(),
                new_source_url: String::new(),
            },
            error: None,
        };

        let cmd = Command::batch([
            Command::perform(list_devices_async(), Message::DevicesLoaded),
            Command::perform(list_profiles_async(), Message::ProfilesLoaded),
            Command::perform(list_plugins_async(), Message::PluginsLoaded),
        ]);
        (app, cmd)
    }

    fn title(&self) -> String {
        "RiverDeck-Redux".to_string()
    }

    fn theme(&self) -> Self::Theme {
        // A more modern baseline look (affects all default widget styling).
        Theme::TokyoNightStorm
    }

    fn update(&mut self, _message: Self::Message) -> Command<Self::Message> {
        match _message {
            Message::RefreshDevices => {
                Command::perform(list_devices_async(), Message::DevicesLoaded)
            }
            Message::DevicesLoaded(res) => {
                match res {
                    Ok(devs) => {
                        self.devices = devs;
                        self.device_choices = self
                            .devices
                            .iter()
                            .map(|d| DeviceChoice {
                                id: d.id,
                                label: d.display_name.clone(),
                            })
                            .collect();
                        if self.selected_device.is_none() && !self.devices.is_empty() {
                            self.selected_device = Some(self.devices[0].id);
                        }
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                // Auto-connect if possible (startup + after refresh).
                if self.connected.is_none() && !self.connecting {
                    if let Some(id) = self.selected_device {
                        self.connecting = true;
                        self.error = None;
                        let events_slot: Arc<std::sync::Mutex<Option<Receiver<DeviceEvent>>>> =
                            Arc::new(std::sync::Mutex::new(None));
                        return Command::perform(
                            connect_device_async(id, events_slot),
                            Message::Connected,
                        );
                    }
                }

                Command::none()
            }
            Message::DevicePicked(choice) => {
                let id = choice.id;
                self.selected_device = Some(id);
                self.selected_key = None;
                self.edit_label.clear();
                self.profile = None;
                self.selected_profile = None;
                self.profiles.clear();
                self.profile_choices.clear();
                self.error = None;

                // Drop old connection and connect to the selected device.
                self.connected = None;
                self.connecting = true;
                let events_slot: Arc<std::sync::Mutex<Option<Receiver<DeviceEvent>>>> =
                    Arc::new(std::sync::Mutex::new(None));
                Command::perform(connect_device_async(id, events_slot), Message::Connected)
            }
            Message::Connected(res) => {
                self.connecting = false;
                match res {
                    Ok(info) => {
                        let mut guard = info.events_slot.lock().expect("events mutex poisoned");
                        let Some(events) = guard.take() else {
                            self.error = Some(
                                "Connect completed but event receiver was missing".to_string(),
                            );
                            self.connected = None;
                            return Command::none();
                        };

                        self.core.selected_device = Some(info.id);
                        let pressed = vec![false; info.key_count as usize];
                        self.connected = Some(ConnectedUi {
                            id: info.id,
                            name: info.name.clone(),
                            key_count: info.key_count,
                            pressed,
                            brightness: 30,
                            controller: info.controller,
                            events,
                        });
                        self.error = None;
                    }
                    Err(e) => {
                        self.connected = None;
                        self.error = Some(e);
                    }
                }
                Command::perform(list_profiles_async(), Message::ProfilesLoaded)
            }
            Message::RefreshProfiles => {
                Command::perform(list_profiles_async(), Message::ProfilesLoaded)
            }
            Message::ProfilesLoaded(res) => {
                match res {
                    Ok(mut metas) => {
                        if let Some(c) = &self.connected {
                            metas.retain(|m| m.key_count == c.key_count);
                        }
                        self.profiles = metas;
                        self.profile_choices = self
                            .profiles
                            .iter()
                            .map(|p| ProfileChoice {
                                id: p.id,
                                label: p.name.clone(),
                            })
                            .collect();

                        // If nothing exists for this device, auto-create a default profile.
                        if self.profiles.is_empty() {
                            if let Some(c) = &self.connected {
                                return Command::perform(
                                    create_profile_async("Default", c.key_count),
                                    Message::ProfileCreated,
                                );
                            }
                        }

                        // Keep/choose selection.
                        if let Some(sel) = self.selected_profile {
                            if !self.profiles.iter().any(|p| p.id == sel) {
                                self.selected_profile = None;
                                self.profile = None;
                            }
                        }

                        if self.selected_profile.is_none() {
                            self.selected_profile = self.profiles.first().map(|p| p.id);
                        }

                        if let Some(id) = self.selected_profile {
                            return Command::perform(
                                load_profile_async(id),
                                Message::ProfileLoaded,
                            );
                        }

                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                Command::none()
            }
            Message::ProfilePicked(choice) => {
                self.selected_profile = Some(choice.id);
                self.selected_key = None;
                self.edit_label.clear();
                Command::perform(load_profile_async(choice.id), Message::ProfileLoaded)
            }
            Message::CreateProfile => {
                let key_count = self.connected.as_ref().map(|c| c.key_count).unwrap_or(15);
                Command::perform(
                    create_profile_async("New Profile", key_count),
                    Message::ProfileCreated,
                )
            }
            Message::ProfileCreated(res) => match res {
                Ok(p) => {
                    self.selected_profile = Some(p.id);
                    self.profile = Some(p);
                    self.selected_key = None;
                    self.edit_label.clear();
                    Command::perform(list_profiles_async(), Message::ProfilesLoaded)
                }
                Err(e) => {
                    self.error = Some(e);
                    Command::none()
                }
            },
            Message::ProfileLoaded(res) => {
                match res {
                    Ok(p) => {
                        self.core.selected_profile = Some(p.id);
                        self.profile = Some(p);
                        self.error = None;
                    }
                    Err(e) => {
                        self.profile = None;
                        self.error = Some(e);
                    }
                }
                Command::none()
            }
            Message::SelectKey(idx) => {
                self.selected_key = Some(idx);
                if let Some(p) = &self.profile {
                    if let Some(k) = p.keys.get(idx) {
                        self.edit_label = k.label.clone();
                    }
                }
                Command::none()
            }
            Message::LabelChanged(val) => {
                self.edit_label = val;
                if let (Some(idx), Some(p)) = (self.selected_key, &mut self.profile) {
                    if let Some(k) = p.keys.get_mut(idx) {
                        k.label = self.edit_label.clone();
                    }
                }
                Command::none()
            }
            Message::SaveProfile => {
                let Some(p) = self.profile.clone() else {
                    return Command::none();
                };
                Command::perform(save_profile_async(p), Message::ProfileSaved)
            }
            Message::ProfileSaved(res) => {
                if let Err(e) = res {
                    self.error = Some(e);
                }
                Command::none()
            }
            Message::RefreshPlugins => {
                Command::perform(list_plugins_async(), Message::PluginsLoaded)
            }
            Message::PluginsLoaded(res) => {
                match res {
                    Ok(plugins) => {
                        self.plugins = plugins;
                        self.actions = build_action_choices(&self.plugins);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
                Command::none()
            }
            Message::InstallPluginPathChanged(p) => {
                self.install_plugin_path = p;
                Command::none()
            }
            Message::InstallPluginFromPath => {
                let path = self.install_plugin_path.trim().to_string();
                if path.is_empty() {
                    return Command::none();
                }
                Command::perform(install_plugin_async(path), Message::PluginInstalled)
            }
            Message::PluginInstalled(res) => match res {
                Ok(()) => {
                    self.install_plugin_path.clear();
                    Command::perform(list_plugins_async(), Message::PluginsLoaded)
                }
                Err(e) => {
                    self.error = Some(e);
                    Command::none()
                }
            },
            Message::OpenMarketplace => {
                self.active_view = ActiveView::Marketplace;
                let Some(idx) = self.marketplace.selected_source_idx else {
                    self.marketplace.loading = false;
                    self.marketplace.error = Some("No marketplace selected.".to_string());
                    return Command::none();
                };
                let url = self.marketplace.sources.get(idx).map(|s| s.index_url.clone());
                let Some(url) = url else {
                    self.marketplace.loading = false;
                    self.marketplace.error = Some("Invalid marketplace selection.".to_string());
                    return Command::none();
                };
                if url.trim().is_empty() {
                    self.marketplace.loading = false;
                    self.marketplace.error =
                        Some("Marketplace URL is empty. Add/edit a marketplace first.".to_string());
                    return Command::none();
                }
                self.marketplace.loading = true;
                self.marketplace.error = None;
                Command::perform(fetch_marketplace_async(url), Message::MarketplaceLoaded)
            }
            Message::CloseMarketplace => {
                self.active_view = ActiveView::Main;
                Command::none()
            }
            Message::MarketplaceRefresh => {
                let Some(idx) = self.marketplace.selected_source_idx else {
                    self.marketplace.loading = false;
                    self.marketplace.error = Some("No marketplace selected.".to_string());
                    return Command::none();
                };
                let url = self.marketplace.sources.get(idx).map(|s| s.index_url.clone());
                let Some(url) = url else {
                    self.marketplace.loading = false;
                    self.marketplace.error = Some("Invalid marketplace selection.".to_string());
                    return Command::none();
                };
                if url.trim().is_empty() {
                    self.marketplace.loading = false;
                    self.marketplace.error =
                        Some("Marketplace URL is empty. Add/edit a marketplace first.".to_string());
                    return Command::none();
                }
                self.marketplace.loading = true;
                self.marketplace.error = None;
                Command::perform(fetch_marketplace_async(url), Message::MarketplaceLoaded)
            }
            Message::MarketplaceSourcePicked(src) => {
                let idx = self.marketplace.sources.iter().position(|s| s == &src);
                self.marketplace.selected_source_idx = idx;
                Command::perform(async { () }, |_| Message::MarketplaceRefresh)
            }
            Message::MarketplaceIndexUrlChanged(url) => {
                // Allow editing the current source URL in-place.
                if let Some(idx) = self.marketplace.selected_source_idx {
                    if let Some(sel) = self.marketplace.sources.get_mut(idx) {
                        sel.index_url = url;
                    }
                }
                Command::none()
            }
            Message::MarketplaceSearchChanged(q) => {
                self.marketplace.query = q;
                Command::none()
            }
            Message::MarketplaceLoaded(res) => {
                self.marketplace.loading = false;
                match res {
                    Ok(list) => {
                        self.marketplace.plugins = list;
                        self.marketplace.error = None;
                    }
                    Err(e) => {
                        self.marketplace.plugins.clear();
                        self.marketplace.error = Some(e);
                    }
                }
                Command::none()
            }
            Message::MarketplaceNewSourceNameChanged(v) => {
                self.marketplace.new_source_name = v;
                Command::none()
            }
            Message::MarketplaceNewSourceUrlChanged(v) => {
                self.marketplace.new_source_url = v;
                Command::none()
            }
            Message::MarketplaceAddSource => {
                let name = self.marketplace.new_source_name.trim().to_string();
                let url = self.marketplace.new_source_url.trim().to_string();
                if name.is_empty() || url.is_empty() {
                    return Command::none();
                }

                let src = MarketplaceSource {
                    name,
                    index_url: url,
                };

                if !self.marketplace.sources.contains(&src) {
                    self.marketplace.sources.push(src.clone());
                }
                self.marketplace.selected_source_idx = self
                    .marketplace
                    .sources
                    .iter()
                    .position(|s| s == &src);
                self.marketplace.new_source_name.clear();
                self.marketplace.new_source_url.clear();

                Command::perform(async { () }, |_| Message::MarketplaceRefresh)
            }
            Message::ActionSelected(choice) => {
                let Some(idx) = self.selected_key else {
                    self.error =
                        Some("Select a key in the preview before assigning an action.".to_string());
                    return Command::none();
                };
                let Some(p) = &mut self.profile else {
                    self.error = Some("No profile loaded.".to_string());
                    return Command::none();
                };

                if let Some(k) = p.keys.get_mut(idx) {
                    let settings = default_settings_for_action(&self.plugins, &choice);
                    k.action = Some(ActionBinding {
                        plugin_id: choice.plugin_id.clone(),
                        action_id: choice.action_id.clone(),
                        settings,
                    });
                }
                Command::none()
            }
            Message::ActionSearchChanged(s) => {
                self.action_search = s;
                Command::none()
            }
            Message::SettingStringChanged { key, value } => {
                set_current_key_setting(
                    self.profile.as_mut(),
                    self.selected_key,
                    key,
                    serde_json::Value::String(value),
                );
                Command::none()
            }
            Message::SettingBoolChanged { key, value } => {
                set_current_key_setting(
                    self.profile.as_mut(),
                    self.selected_key,
                    key,
                    serde_json::Value::Bool(value),
                );
                Command::none()
            }
            Message::SettingNumberChanged { key, value } => {
                let num = value
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64);
                let v = num
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null);
                set_current_key_setting(self.profile.as_mut(), self.selected_key, key, v);
                Command::none()
            }
            Message::ActionInvoked(res) => {
                if let Err(e) = res {
                    self.error = Some(e);
                }
                Command::none()
            }
            Message::Tick => {
                let mut cmds: Vec<Command<Message>> = vec![];
                if let Some(c) = &mut self.connected {
                    while let Ok(ev) = c.events.try_recv() {
                        match ev {
                            DeviceEvent::KeyDown { key } => {
                                if let Some(slot) = c.pressed.get_mut(key as usize) {
                                    *slot = true;
                                }

                                // Dispatch bound action on key-down.
                                if let Some(p) = &self.profile {
                                    if let Some(kcfg) = p.keys.get(key as usize) {
                                        if let Some(binding) = &kcfg.action {
                                            if let Some(plugin) = self
                                                .plugins
                                                .iter()
                                                .find(|pl| pl.manifest.id == binding.plugin_id)
                                                .cloned()
                                            {
                                                let action_id = binding.action_id.clone();
                                                let settings = binding.settings.clone();
                                                cmds.push(Command::perform(
                                                    invoke_action_async(
                                                        plugin, action_id, key, settings,
                                                    ),
                                                    Message::ActionInvoked,
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                            DeviceEvent::KeyUp { key } => {
                                if let Some(slot) = c.pressed.get_mut(key as usize) {
                                    *slot = false;
                                }
                            }
                            DeviceEvent::Disconnected => {
                                self.error = Some("Device disconnected".to_string());
                                self.connected = None;
                                break;
                            }
                        }
                    }
                }
                Command::batch(cmds)
            }
            Message::BrightnessChanged(v) => {
                let Some(c) = &mut self.connected else {
                    return Command::none();
                };
                let v = v.clamp(0, 100) as u8;
                c.brightness = v;
                let controller = c.controller.clone();
                Command::perform(
                    set_brightness_async(controller, v),
                    Message::BrightnessApplied,
                )
            }
            Message::BrightnessApplied(res) => {
                if let Err(e) = res {
                    self.error = Some(e);
                }
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let topbar = self.view_topbar();
        let content: Element<Message> = match self.active_view {
            ActiveView::Main => {
                let sidebar = self.view_sidebar();
                let preview = self.view_preview_panel();
                let inspector = self.view_inspector_panel();
                let actions = self.view_actions_panel();

                let main = column![preview, h_divider(), inspector]
                    .spacing(0)
                    .width(Length::Fill)
                    .height(Length::Fill);

                row![sidebar, v_divider(), main, v_divider(), actions]
                    .spacing(0)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            ActiveView::Marketplace => self.view_marketplace(),
        };

        let mut root = column![topbar, content]
            .spacing(10)
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill);

        if let Some(err) = &self.error {
            root = root.push(self.view_error_banner(err));
        }

        container(root)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(app_background())
            .into()
    }

    fn subscription(&self) -> iced::Subscription<Self::Message> {
        iced::time::every(Duration::from_millis(33)).map(|_| Message::Tick)
    }
}

#[derive(Debug, Clone)]
enum Message {
    RefreshDevices,
    DevicesLoaded(Result<Vec<DiscoveredDevice>, String>),
    DevicePicked(DeviceChoice),
    Connected(Result<ConnectedInfo, String>),
    RefreshProfiles,
    ProfilesLoaded(Result<Vec<ProfileMeta>, String>),
    CreateProfile,
    ProfileCreated(Result<Profile, String>),
    ProfilePicked(ProfileChoice),
    ProfileLoaded(Result<Profile, String>),
    SelectKey(usize),
    LabelChanged(String),
    SaveProfile,
    ProfileSaved(Result<(), String>),
    RefreshPlugins,
    PluginsLoaded(Result<Vec<InstalledPlugin>, String>),
    InstallPluginPathChanged(String),
    InstallPluginFromPath,
    PluginInstalled(Result<(), String>),
    OpenMarketplace,
    CloseMarketplace,
    MarketplaceRefresh,
    MarketplaceSourcePicked(MarketplaceSource),
    MarketplaceIndexUrlChanged(String),
    MarketplaceSearchChanged(String),
    MarketplaceLoaded(Result<Vec<MarketplacePlugin>, String>),
    MarketplaceNewSourceNameChanged(String),
    MarketplaceNewSourceUrlChanged(String),
    MarketplaceAddSource,
    ActionSelected(ActionChoice),
    ActionSearchChanged(String),
    SettingStringChanged { key: String, value: String },
    SettingBoolChanged { key: String, value: bool },
    SettingNumberChanged { key: String, value: String },
    ActionInvoked(Result<(), String>),
    Tick,
    BrightnessChanged(i32),
    BrightnessApplied(Result<(), String>),
}

#[derive(Clone)]
struct ConnectedInfo {
    id: app_core::ids::DeviceId,
    name: String,
    key_count: u8,
    controller: DeviceController,
    events_slot: Arc<std::sync::Mutex<Option<Receiver<DeviceEvent>>>>,
}

impl fmt::Debug for ConnectedInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectedInfo")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("key_count", &self.key_count)
            .finish_non_exhaustive()
    }
}

struct ConnectedUi {
    id: app_core::ids::DeviceId,
    name: String,
    key_count: u8,
    pressed: Vec<bool>,
    brightness: u8,
    controller: DeviceController,
    events: tokio::sync::mpsc::Receiver<DeviceEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DeviceChoice {
    id: app_core::ids::DeviceId,
    label: String,
}

impl fmt::Display for DeviceChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProfileChoice {
    id: ProfileId,
    label: String,
}

impl fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActionChoice {
    plugin_id: String,
    action_id: String,
    label: String,
}

impl fmt::Display for ActionChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

impl App {
    fn view_topbar(&self) -> Element<'_, Message> {
        let device_selected = self
            .selected_device
            .and_then(|id| self.device_choices.iter().find(|d| d.id == id).cloned());
        let profile_selected = self
            .selected_profile
            .and_then(|id| self.profile_choices.iter().find(|p| p.id == id).cloned());

        let status = match (&self.connected, self.connecting) {
            (None, true) => "Connecting…".to_string(),
            (None, false) => "Not connected".to_string(),
            (Some(c), _) => format!(
                "Connected to {} • {} keys • id {}",
                c.name, c.key_count, c.id.0
            ),
        };

        let bar = row![
            column![
                text("RiverDeck-Redux").size(22),
                text(status).size(12).style(color_text_muted()),
            ]
            .spacing(2),
            horizontal_space(),
            column![
                text("Device").size(12).style(color_text_muted()),
                pick_list(
                    self.device_choices.clone(),
                    device_selected,
                    Message::DevicePicked
                )
            ]
            .spacing(4),
            column![
                text("Profile").size(12).style(color_text_muted()),
                row![
                    pick_list(
                        self.profile_choices.clone(),
                        profile_selected,
                        Message::ProfilePicked
                    ),
                    button(text("⟳"))
                        .style(iced::theme::Button::Secondary)
                        .on_press(Message::RefreshProfiles),
                    button(text("+"))
                        .style(iced::theme::Button::Secondary)
                        .on_press(Message::CreateProfile),
                ]
                .spacing(8)
                .align_items(Alignment::Center)
            ]
            .spacing(4),
            button(text("Refresh"))
                .style(iced::theme::Button::Secondary)
                .on_press(Message::RefreshDevices),
        ]
        .align_items(Alignment::Center)
        .spacing(14);

        container(bar)
            .padding(12)
            .style(panel())
            .width(Length::Fill)
            .into()
    }

    fn view_error_banner(&self, err: &str) -> Element<'_, Message> {
        container(text(err).style(Color::from_rgb8(255, 160, 160)))
            .padding(10)
            .style(error_banner())
            .width(Length::Fill)
            .into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let plugins_section = self.view_sidebar_plugins();

        let content = column![plugins_section].spacing(12);

        container(scrollable(content).height(Length::Fill))
            .padding(12)
            .width(Length::Fixed(300.0))
            .height(Length::Fill)
            .style(panel())
            .into()
    }

    fn view_sidebar_plugins(&self) -> Element<'_, Message> {
        let mut col = column![text("Plugins").size(16)].spacing(8);

        col = col.push(
            text_input(
                "Local plugin dir (contains manifest.json)",
                &self.install_plugin_path,
            )
            .on_input(Message::InstallPluginPathChanged),
        );
        col = col.push(
            row![
                button(text("Install"))
                    .style(iced::theme::Button::Primary)
                    .on_press(Message::InstallPluginFromPath),
                button(text("Refresh"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::RefreshPlugins),
                button(text("Marketplace"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::OpenMarketplace),
            ]
            .spacing(8),
        );

        if self.plugins.is_empty() {
            col = col.push(text("No plugins installed."));
        } else {
            for p in &self.plugins {
                col = col.push(text(format!("• {}", p.manifest.name)).size(13));
            }
        }

        col.into()
    }

    fn view_actions_panel(&self) -> Element<'_, Message> {
        let header = text("Actions").size(16);

        let search = text_input("Search actions…", &self.action_search)
            .on_input(Message::ActionSearchChanged);

        let q = self.action_search.trim().to_ascii_lowercase();
        let actions_iter = self.actions.iter().filter(|a| {
            if q.is_empty() {
                true
            } else {
                a.label.to_ascii_lowercase().contains(&q)
            }
        });

        let mut list = column![].spacing(8);
        let mut any = false;
        for a in actions_iter {
            any = true;
            list = list.push(
                button(text(&a.label).size(13))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::ActionSelected(a.clone())),
            );
        }

        if !any {
            list = list.push(
                text(if self.actions.is_empty() {
                    "No actions available. Install a plugin first."
                } else {
                    "No actions match your search."
                })
                .size(13)
                .style(color_text_muted()),
            );
        }

        let content = column![
            header,
            horizontal_rule(1),
            search,
            scrollable(list).height(Length::Fill),
        ]
        .spacing(10);

        container(content)
            .padding(12)
            .width(Length::Fixed(360.0))
            .height(Length::Fill)
            .style(panel())
            .into()
    }

    fn view_marketplace(&self) -> Element<'_, Message> {
        let header = row![
            text("Plugin Marketplace").size(18),
            horizontal_space(),
            button(text("Back"))
                .style(iced::theme::Button::Secondary)
                .on_press(Message::CloseMarketplace),
        ]
        .align_items(Alignment::Center)
        .spacing(10);

        let selected = self
            .marketplace
            .selected_source_idx
            .and_then(|i| self.marketplace.sources.get(i))
            .cloned();

        let source_picker = pick_list(
            self.marketplace.sources.clone(),
            selected.clone(),
            Message::MarketplaceSourcePicked,
        );

        let current_url = selected
            .as_ref()
            .map(|s| s.index_url.as_str())
            .unwrap_or("");

        let url_row = row![
            text("Marketplace").size(12).style(color_text_muted()),
            source_picker,
            horizontal_space(),
            text("Index URL").size(12).style(color_text_muted()),
            text_input("https://…/plugins.json", current_url)
                .on_input(Message::MarketplaceIndexUrlChanged),
            button(text("Refresh"))
                .style(iced::theme::Button::Secondary)
                .on_press(Message::MarketplaceRefresh),
        ]
        .spacing(10)
        .align_items(Alignment::Center);

        let add_row = row![
            text_input("New marketplace name", &self.marketplace.new_source_name)
                .on_input(Message::MarketplaceNewSourceNameChanged),
            text_input("New marketplace index URL", &self.marketplace.new_source_url)
                .on_input(Message::MarketplaceNewSourceUrlChanged),
            button(text("Add"))
                .style(iced::theme::Button::Secondary)
                .on_press(Message::MarketplaceAddSource),
        ]
        .spacing(10)
        .align_items(Alignment::Center);

        let search = text_input("Search plugins…", &self.marketplace.query)
            .on_input(Message::MarketplaceSearchChanged);

        let q = self.marketplace.query.trim().to_ascii_lowercase();
        let list_iter = self.marketplace.plugins.iter().filter(|p| {
            if q.is_empty() {
                true
            } else {
                p.name.to_ascii_lowercase().contains(&q)
                    || p.id.to_ascii_lowercase().contains(&q)
                    || p.description.to_ascii_lowercase().contains(&q)
            }
        });

        let mut list = column![].spacing(10);
        for p in list_iter {
            let mut body = column![text(&p.name).size(14)]
                .spacing(4)
                .push(text(p.id.clone()).size(12).style(color_text_muted()));

            if !p.version.is_empty() {
                body = body.push(text(format!("v{}", p.version)).size(12).style(color_text_muted()));
            }
            if !p.description.is_empty() {
                body = body.push(text(p.description.clone()).size(12).style(color_text_muted()));
            }

            let install_btn = if p.download_url.is_some() {
                button(text("Install")).style(iced::theme::Button::Secondary)
            } else {
                button(text("Install")).style(iced::theme::Button::Secondary)
            };

            list = list.push(container(row![container(body).width(Length::Fill), install_btn].spacing(12))
                .padding(10)
                .style(panel()));
        }

        let status: Element<Message> = if self.marketplace.loading {
            text("Loading…").style(color_text_muted()).into()
        } else if let Some(err) = &self.marketplace.error {
            text(format!("Error: {err}")).into()
        } else if self.marketplace.plugins.is_empty() {
            text("No plugins found.").style(color_text_muted()).into()
        } else {
            text("").into()
        };

        let content = column![
            header,
            h_divider(),
            url_row,
            add_row,
            search,
            status,
            scrollable(list).height(Length::Fill)
        ]
        .spacing(10)
        .height(Length::Fill);

        container(content)
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(panel())
            .into()
    }

    fn view_preview_panel(&self) -> Element<'_, Message> {
        let selected = self
            .selected_key
            .map(|k| format!("Selected: Key {k}"))
            .unwrap_or_else(|| "Selected: —".to_string());

        let title = row![
            column![
                text("Preview").size(16),
                text(selected)
                    .size(12)
                    .style(color_text_muted()),
            ]
            .spacing(2),
            horizontal_space(),
            self.view_brightness_control_compact(),
        ]
        .align_items(Alignment::Center)
        .spacing(12);

        let body: Element<Message> = match &self.connected {
            None => container(
                column![
                    text("Connect a device to see a preview.").size(16),
                    text("Tip: use the left sidebar to select a device and click Connect.")
                        .size(13)
                        .style(color_text_muted()),
                ]
                .spacing(6),
            )
            .center_x()
            .center_y()
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            Some(c) => container(self.view_deck_preview(c.key_count, &c.pressed))
                .center_x()
                .center_y()
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };

        container(column![title, horizontal_rule(1), body].spacing(10))
            .padding(12)
            .width(Length::Fill)
            .height(Length::FillPortion(2))
            .style(panel())
            .into()
    }

    fn view_brightness_control_compact(&self) -> Element<'_, Message> {
        let Some(c) = &self.connected else {
            return text("").into();
        };

        row![
            text(format!("Brightness {}%", c.brightness)).size(12),
            slider(0..=100, c.brightness as i32, Message::BrightnessChanged)
                .width(Length::Fixed(160.0))
        ]
        .spacing(10)
        .align_items(Alignment::Center)
        .into()
    }

    fn view_inspector_panel(&self) -> Element<'_, Message> {
        let header = text("Inspector").size(16);

        let body = match (self.connected.as_ref(), self.selected_key) {
            (None, _) => text("Connect a device to inspect keys.").into(),
            (Some(_), None) => text("Click a key in the preview to edit it.").into(),
            (Some(c), Some(idx)) => self.view_key_inspector(c, idx),
        };

        container(
            column![
                header,
                horizontal_rule(1),
                scrollable(body).height(Length::Fill)
            ]
            .spacing(10),
        )
        .padding(12)
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .style(panel())
        .into()
    }

    fn view_key_inspector(&self, c: &ConnectedUi, idx: usize) -> Element<'_, Message> {
        let is_down = c.pressed.get(idx).copied().unwrap_or(false);

        let mut col = column![
            text(format!("Key {idx}")).size(20),
            text(if is_down {
                "State: Pressed"
            } else {
                "State: Released"
            })
            .size(13)
            .style(color_text_muted()),
        ]
        .spacing(6);

        col = col.push(horizontal_rule(1));

        col = col.push(text("Label").size(14));
        col = col.push(text_input("Label", &self.edit_label).on_input(Message::LabelChanged));

        col = col.push(horizontal_rule(1));

        col = col.push(text("Action").size(14));
        let current = self.current_action_choice(idx);
        col = col.push(pick_list(
            self.actions.clone(),
            current,
            Message::ActionSelected,
        ));

        col = col.push(self.view_action_settings(idx));

        col = col.push(horizontal_rule(1));
        col = col.push(
            row![button(text("Save"))
                .on_press(Message::SaveProfile)
                .style(iced::theme::Button::Primary),]
            .spacing(8),
        );

        col.into()
    }

    fn view_deck_preview(&self, key_count: u8, pressed: &[bool]) -> Element<'_, Message> {
        let (cols, rows) = deck_grid_dims(key_count);
        let (key, gap, pad, radius) = deck_metrics(key_count);

        let mut grid = column![].spacing(gap as u16);
        for r in 0..rows {
            let mut line = row![].spacing(gap as u16);
            for c in 0..cols {
                let idx = r * cols + c;
                if idx >= key_count as usize {
                    break;
                }
                line = line.push(self.view_deck_key(idx, pressed));
            }
            grid = grid.push(line);
        }

        // Stream Deck+ preview: keys + touch strip + 4 dials.
        let content = if key_count == 8 {
            let strip = container(
                text("Touch strip")
                    .size(12)
                    .style(color_text_muted()),
            )
            .width(Length::Fill)
            .height(Length::Fixed(26.0))
            .center_x()
            .center_y()
            .style(iced::theme::Container::Custom(Box::new(|theme: &Theme| {
                let p = theme.extended_palette();
                iced::widget::container::Appearance {
                    background: Some(Background::Color(p.background.base.color)),
                    text_color: Some(p.background.base.text),
                    border: Border {
                        radius: 10.0.into(),
                        width: 1.0,
                        color: Color::from_rgba8(255, 255, 255, 0.08),
                    },
                    shadow: Shadow::default(),
                }
            })));

            let dial = |idx: usize| {
                let label = format!("Dial {}", idx + 1);
                container(text(label).size(11).style(color_text_muted()))
                    .width(Length::Fixed(64.0))
                    .height(Length::Fixed(44.0))
                    .center_x()
                    .center_y()
                    .style(iced::theme::Container::Custom(Box::new(|theme: &Theme| {
                        let p = theme.extended_palette();
                        iced::widget::container::Appearance {
                            background: Some(Background::Color(p.background.base.color)),
                            text_color: Some(p.background.base.text),
                            border: Border {
                                // Less "toy"/pill-like; feels more modern and consistent.
                                radius: 14.0.into(),
                                width: 1.0,
                                color: Color::from_rgba8(255, 255, 255, 0.08),
                            },
                            shadow: Shadow::default(),
                        }
                    })))
            };

            let dials = row![dial(0), dial(1), dial(2), dial(3)]
                .spacing(12)
                .align_items(Alignment::Center);

            column![grid, strip, dials].spacing(gap as u16)
        } else {
            grid
        };

        let width = (cols as f32 * key) + ((cols - 1) as f32 * gap) + 2.0 * pad;
        let base_height = (rows as f32 * key) + ((rows - 1) as f32 * gap) + 2.0 * pad;
        let extra = if key_count == 8 {
            26.0 + 44.0 + (gap * 2.0)
        } else {
            0.0
        };

        let deck = container(content)
            .padding(pad as u16)
            .style(deck_body_style(radius))
            .width(Length::Fixed(width))
            .height(Length::Fixed(base_height + extra));

        deck.into()
    }

    fn view_deck_key(&self, idx: usize, pressed: &[bool]) -> Element<'_, Message> {
        let is_pressed = pressed.get(idx).copied().unwrap_or(false);
        let is_selected = self.selected_key == Some(idx);
        let (key, _gap, _pad, _radius) =
            deck_metrics(self.connected.as_ref().map(|c| c.key_count).unwrap_or(15));

        let label = self
            .profile
            .as_ref()
            .and_then(|p| p.keys.get(idx))
            .map(|k| k.label.as_str())
            .unwrap_or("");

        let action_hint = self
            .profile
            .as_ref()
            .and_then(|p| p.keys.get(idx))
            .and_then(|k| k.action.as_ref())
            .and_then(|a| self.action_label(&a.plugin_id, &a.action_id));

        let max_title = if key <= 64.0 { 10 } else { 14 };
        let max_sub = if key <= 64.0 { 12 } else { 18 };

        let title = if label.is_empty() {
            action_hint.clone().unwrap_or_else(|| format!("Key {idx}"))
        } else {
            label.to_string()
        };

        let subtitle = if label.is_empty() { None } else { action_hint };
        let title = truncate(&title, max_title);
        let subtitle = subtitle.map(|s| truncate(&s, max_sub));

        let content = column![
            text(title)
                .size(12)
                .width(Length::Fill)
                .horizontal_alignment(Horizontal::Center),
            subtitle
                .map(|s| {
                    text(s)
                        .size(10)
                        .style(color_text_muted())
                        .width(Length::Fill)
                        .horizontal_alignment(Horizontal::Center)
                })
                .unwrap_or_else(|| text("").size(10))
        ]
        .spacing(2)
        .align_items(Alignment::Center);

        button(container(content).center_x().center_y())
            .width(Length::Fixed(key))
            .height(Length::Fixed(key))
            .padding(0)
            .on_press(Message::SelectKey(idx))
            .style(iced::theme::Button::custom(DeckKeyStyle {
                pressed: is_pressed,
                selected: is_selected,
            }))
            .into()
    }

    fn action_label(&self, plugin_id: &str, action_id: &str) -> Option<String> {
        let (_plugin, action) = find_action_def_by_ids(&self.plugins, plugin_id, action_id)?;
        Some(action.name.clone())
    }

    fn current_action_choice(&self, idx: usize) -> Option<ActionChoice> {
        let p = self.profile.as_ref()?;
        let k = p.keys.get(idx)?;
        let b = k.action.as_ref()?;
        self.actions
            .iter()
            .find(|a| a.plugin_id == b.plugin_id && a.action_id == b.action_id)
            .cloned()
    }

    fn view_action_settings(&self, idx: usize) -> Element<'_, Message> {
        let Some(p) = &self.profile else {
            return text("No profile loaded.").into();
        };
        let Some(k) = p.keys.get(idx) else {
            return text("Invalid key index.").into();
        };
        let Some(binding) = &k.action else {
            return text("No action bound.").into();
        };

        let Some((_plugin, action_def)) =
            find_action_def_by_ids(&self.plugins, &binding.plugin_id, &binding.action_id)
        else {
            return text("Action not found (plugin missing or manifest changed).").into();
        };

        if action_def.settings.is_empty() {
            return text("This action has no settings.").into();
        }

        let mut col = column![text("Settings").size(16)].spacing(8);

        for field in &action_def.settings {
            col = col.push(self.view_setting_field(field, &binding.settings));
        }

        col.into()
    }

    fn view_setting_field(
        &self,
        field: &SettingField,
        settings: &serde_json::Value,
    ) -> Element<'_, Message> {
        let key = field.key.clone();
        let label = field.label.clone();

        match field.ty {
            SettingType::String => {
                let current = settings
                    .get(&field.key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input =
                    text_input("", &current).on_input(move |v| Message::SettingStringChanged {
                        key: key.clone(),
                        value: v,
                    });
                column![text(label), input].spacing(4).into()
            }
            SettingType::Boolean => {
                let current = settings
                    .get(&field.key)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                checkbox(label, current)
                    .on_toggle(move |v| Message::SettingBoolChanged {
                        key: key.clone(),
                        value: v,
                    })
                    .into()
            }
            SettingType::Number => {
                let current = settings
                    .get(&field.key)
                    .and_then(|v| v.as_f64())
                    .map(|n| n.to_string())
                    .or_else(|| {
                        settings
                            .get(&field.key)
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default();
                let input =
                    text_input("", &current).on_input(move |v| Message::SettingNumberChanged {
                        key: key.clone(),
                        value: v,
                    });
                column![text(label), input].spacing(4).into()
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DeckKeyStyle {
    pressed: bool,
    selected: bool,
}

impl iced::widget::button::StyleSheet for DeckKeyStyle {
    type Style = Theme;

    fn active(&self, theme: &Self::Style) -> iced::widget::button::Appearance {
        let palette = theme.extended_palette();

        let bg = if self.pressed {
            palette.success.weak.color
        } else {
            palette.background.strong.color
        };

        let border_color = if self.selected {
            palette.primary.base.color
        } else if self.pressed {
            palette.success.base.color
        } else {
            palette.background.strong.color
        };

        iced::widget::button::Appearance {
            background: Some(Background::Color(bg)),
            text_color: palette.background.base.text,
            border: Border {
                color: border_color,
                width: if self.selected { 2.0 } else { 1.0 },
                radius: 8.0.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.28),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        }
    }

    fn hovered(&self, theme: &Self::Style) -> iced::widget::button::Appearance {
        let palette = theme.extended_palette();
        let mut a = self.active(theme);
        if !self.pressed {
            a.background = Some(Background::Color(palette.background.base.color));
        }
        a
    }

    fn pressed(&self, theme: &Self::Style) -> iced::widget::button::Appearance {
        let mut a = self.active(theme);
        a.shadow = Shadow::default();
        a
    }
}

fn app_background() -> iced::theme::Container {
    iced::theme::Container::Custom(Box::new(|theme: &Theme| {
        let p = theme.extended_palette();
        iced::widget::container::Appearance {
            background: Some(Background::Color(p.background.base.color)),
            text_color: Some(p.background.base.text),
            border: Border::default(),
            shadow: Shadow::default(),
        }
    }))
}

fn panel() -> iced::theme::Container {
    iced::theme::Container::Custom(Box::new(|theme: &Theme| {
        let p = theme.extended_palette();
        let mut shade = p.background.weak.color;
        shade.a = 0.92;
        iced::widget::container::Appearance {
            background: Some(Background::Color(shade)),
            text_color: Some(p.background.base.text),
            border: Border::default(),
            shadow: Shadow {
                // Subtle shading (keep it sleek; dividers do most of the separation).
                color: Color::from_rgba8(0, 0, 0, 0.12),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 12.0,
            },
        }
    }))
}

fn error_banner() -> iced::theme::Container {
    iced::theme::Container::Custom(Box::new(|_theme: &Theme| {
        iced::widget::container::Appearance {
            background: Some(Background::Color(Color::from_rgb8(70, 10, 10))),
            text_color: Some(Color::from_rgb8(255, 190, 190)),
            border: Border {
                radius: 0.0.into(),
                width: 1.0,
                color: Color::from_rgb8(120, 30, 30),
            },
            shadow: Shadow::default(),
        }
    }))
}

fn divider_style() -> iced::theme::Container {
    iced::theme::Container::Custom(Box::new(|theme: &Theme| {
        let p = theme.extended_palette();
        let mut c = p.background.strong.color;
        c.a = 0.65;
        iced::widget::container::Appearance {
            background: Some(Background::Color(c)),
            text_color: None,
            border: Border::default(),
            shadow: Shadow::default(),
        }
    }))
}

fn v_divider() -> Element<'static, Message> {
    container(text(""))
        .width(Length::Fixed(1.0))
        .height(Length::Fill)
        .style(divider_style())
        .into()
}

fn h_divider() -> Element<'static, Message> {
    container(text(""))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(divider_style())
        .into()
}

fn default_marketplace_sources() -> Vec<MarketplaceSource> {
    // Built-in default: Rivul/OpenAction marketplace catalogue
    // (as used by https://marketplace.rivul.us/).
    let mut out = vec![MarketplaceSource {
        name: "Rivul (OpenAction)".to_string(),
        index_url: "https://openactionapi.github.io/plugins/catalogue.json".to_string(),
    }];

    // Optional user-provided list:
    // OPENACTION_MARKETPLACES="Name|https://...;Other|https://..."
    if let Ok(raw) = std::env::var("OPENACTION_MARKETPLACES") {
        for part in raw.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let mut it = part.splitn(2, '|');
            let name = it.next().unwrap_or("").trim();
            let url = it.next().unwrap_or("").trim();
            if name.is_empty() || url.is_empty() {
                continue;
            }
            let src = MarketplaceSource {
                name: name.to_string(),
                index_url: url.to_string(),
            };
            if !out.contains(&src) {
                out.push(src);
            }
        }
    }

    // Placeholder "your own" marketplace, editable in UI.
    out.push(MarketplaceSource {
        name: "My Marketplace".to_string(),
        index_url: String::new(),
    });

    out
}

fn deck_grid_dims(key_count: u8) -> (usize, usize) {
    match key_count {
        8 => (4, 2),
        6 => (3, 2),
        32 => (8, 4),
        _ => (5, 3),
    }
}

/// Returns (key_size_px, gap_px, padding_px, deck_radius_px)
fn deck_metrics(key_count: u8) -> (f32, f32, f32, f32) {
    match key_count {
        // Slightly tighter + less rounded for a modern, cleaner feel.
        8 => (72.0, 10.0, 18.0, 18.0),
        32 => (62.0, 9.0, 16.0, 16.0),
        6 => (80.0, 12.0, 18.0, 18.0),
        _ => (74.0, 10.0, 18.0, 18.0),
    }
}

fn deck_body_style(radius: f32) -> iced::theme::Container {
    iced::theme::Container::Custom(Box::new(move |theme: &Theme| {
        let p = theme.extended_palette();
        iced::widget::container::Appearance {
            background: Some(Background::Color(p.background.strong.color)),
            text_color: Some(p.background.base.text),
            border: Border {
                radius: radius.into(),
                width: 1.0,
                color: Color::from_rgba8(255, 255, 255, 0.06),
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.32),
                offset: iced::Vector::new(0.0, 16.0),
                blur_radius: 42.0,
            },
        }
    }))
}

fn color_text_muted() -> Color {
    // Keep muted text aligned with the chosen theme, without hard-coding a random gray.
    let p = Theme::TokyoNightStorm.extended_palette();
    let base = p.background.base.text;
    Color { a: 0.72, ..base }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(ch);
    }
    if s.chars().count() > max_chars && max_chars >= 2 {
        out.pop();
        out.push('…');
    }
    out
}

async fn list_devices_async() -> Result<Vec<DiscoveredDevice>, String> {
    let svc = HidDeviceService::new().map_err(|e| e.to_string())?;
    svc.list_devices().await.map_err(|e| e.to_string())
}

async fn list_profiles_async() -> Result<Vec<ProfileMeta>, String> {
    storage::profiles::list_profiles().map_err(|e| e.to_string())
}

async fn load_profile_async(id: ProfileId) -> Result<Profile, String> {
    let path = storage::profiles::profile_path(id).map_err(|e| e.to_string())?;
    storage::profiles::load_profile(&path).map_err(|e| e.to_string())
}

async fn create_profile_async(name: &str, key_count: u8) -> Result<Profile, String> {
    let p = storage::profiles::create_profile(name, key_count).map_err(|e| e.to_string())?;
    storage::profiles::save_profile(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

async fn save_profile_async(profile: Profile) -> Result<(), String> {
    storage::profiles::save_profile(&profile).map_err(|e| e.to_string())
}

async fn list_plugins_async() -> Result<Vec<InstalledPlugin>, String> {
    openaction::registry::list_installed().map_err(|e| e.to_string())
}

async fn install_plugin_async(path: String) -> Result<(), String> {
    use std::path::Path;
    openaction::registry::install_local_dir(Path::new(&path)).map_err(|e| e.to_string())
}

async fn fetch_marketplace_async(url: String) -> Result<Vec<MarketplacePlugin>, String> {
    openaction::marketplace::fetch_plugins(&url)
        .await
        .map_err(|e| e.to_string())
}

async fn invoke_action_async(
    plugin: InstalledPlugin,
    action_id: String,
    key: u8,
    settings: serde_json::Value,
) -> Result<(), String> {
    let rt = ActionRuntime::new();
    rt.invoke(&plugin, &action_id, key, "key_down", settings)
        .await
        .map_err(|e| e.to_string())
}

fn build_action_choices(plugins: &[InstalledPlugin]) -> Vec<ActionChoice> {
    let mut out = vec![];
    for p in plugins {
        for a in &p.manifest.actions {
            out.push(ActionChoice {
                plugin_id: p.manifest.id.clone(),
                action_id: a.id.clone(),
                label: format!("{}: {}", p.manifest.name, a.name),
            });
        }
    }
    out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    out
}

fn find_action_def_by_ids<'a>(
    plugins: &'a [InstalledPlugin],
    plugin_id: &str,
    action_id: &str,
) -> Option<(&'a InstalledPlugin, &'a ActionDefinition)> {
    let plugin = plugins.iter().find(|p| p.manifest.id == plugin_id)?;
    let action = plugin.manifest.actions.iter().find(|a| a.id == action_id)?;
    Some((plugin, action))
}

fn default_settings_for_action(
    plugins: &[InstalledPlugin],
    choice: &ActionChoice,
) -> serde_json::Value {
    let Some((_plugin, action)) =
        find_action_def_by_ids(plugins, &choice.plugin_id, &choice.action_id)
    else {
        return serde_json::Value::Object(serde_json::Map::new());
    };

    let mut map = serde_json::Map::new();
    for f in &action.settings {
        let v = if let Some(d) = &f.default {
            d.clone()
        } else {
            match f.ty {
                SettingType::String => serde_json::Value::String(String::new()),
                SettingType::Boolean => serde_json::Value::Bool(false),
                SettingType::Number => serde_json::Value::Null,
            }
        };
        map.insert(f.key.clone(), v);
    }
    serde_json::Value::Object(map)
}

fn set_current_key_setting(
    profile: Option<&mut Profile>,
    idx: Option<usize>,
    key: String,
    value: serde_json::Value,
) {
    let (Some(profile), Some(idx)) = (profile, idx) else {
        return;
    };
    let Some(k) = profile.keys.get_mut(idx) else {
        return;
    };
    let Some(binding) = &mut k.action else {
        return;
    };
    let obj = binding.settings.as_object_mut();
    if let Some(obj) = obj {
        obj.insert(key, value);
    } else {
        let mut map = serde_json::Map::new();
        map.insert(key, value);
        binding.settings = serde_json::Value::Object(map);
    }
}

async fn connect_device_async(
    id: app_core::ids::DeviceId,
    events_slot: Arc<std::sync::Mutex<Option<Receiver<DeviceEvent>>>>,
) -> Result<ConnectedInfo, String> {
    let svc = HidDeviceService::new().map_err(|e| e.to_string())?;
    let dev = svc.connect(id).await.map_err(|e| e.to_string())?;
    let controller = dev.controller();

    {
        let mut guard = events_slot
            .lock()
            .map_err(|_| "events mutex poisoned".to_string())?;
        *guard = Some(dev.events);
    }

    Ok(ConnectedInfo {
        id: dev.id,
        name: dev.name,
        key_count: dev.key_count,
        controller,
        events_slot,
    })
}

async fn set_brightness_async(controller: DeviceController, percent: u8) -> Result<(), String> {
    controller
        .set_brightness(percent)
        .await
        .map_err(|e| e.to_string())
}
