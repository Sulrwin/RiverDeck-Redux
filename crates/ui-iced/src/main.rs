use std::time::Duration;
use std::{fmt, sync::Arc};

use app_core::AppCore;
use device::{DeviceController, DeviceEvent, DeviceService, DiscoveredDevice, HidDeviceService};
use iced::widget::{button, checkbox, column, container, pick_list, row, slider, text, text_input};
use iced::{Alignment, Application, Color, Command, Element, Length, Settings, Theme};
use tokio::sync::mpsc::Receiver;

use app_core::ids::ProfileId;
use storage::profiles::{ActionBinding, Profile, ProfileMeta};

use openaction::manifest::{ActionDefinition, SettingField, SettingType};
use openaction::registry::InstalledPlugin;
use plugin_runtime::ActionRuntime;

fn main() -> iced::Result {
    init_tracing();

    App::run(Settings {
        window: iced::window::Settings {
            size: iced::Size::new(900.0, 650.0),
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
    selected_device: Option<app_core::ids::DeviceId>,
    connecting: bool,
    connected: Option<ConnectedUi>,
    profiles: Vec<ProfileMeta>,
    selected_profile: Option<ProfileId>,
    profile: Option<Profile>,
    selected_key: Option<usize>,
    edit_label: String,
    plugins: Vec<InstalledPlugin>,
    actions: Vec<ActionChoice>,
    install_plugin_path: String,
    error: Option<String>,
}

impl Application for App {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let app = Self {
            core: AppCore::new(),
            devices: vec![],
            selected_device: None,
            connecting: false,
            connected: None,
            profiles: vec![],
            selected_profile: None,
            profile: None,
            selected_key: None,
            edit_label: String::new(),
            plugins: vec![],
            actions: vec![],
            install_plugin_path: String::new(),
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
        Theme::Dark
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
                        if self.selected_device.is_none() && !self.devices.is_empty() {
                            self.selected_device = Some(self.devices[0].id);
                        }
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                Command::none()
            }
            Message::SelectDevice(id) => {
                self.selected_device = Some(id);
                self.error = None;
                Command::none()
            }
            Message::ConnectSelected => {
                let Some(id) = self.selected_device else {
                    return Command::none();
                };
                self.connecting = true;
                self.error = None;
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
            Message::SelectProfile(id) => {
                self.selected_profile = Some(id);
                self.selected_key = None;
                self.edit_label.clear();
                Command::perform(load_profile_async(id), Message::ProfileLoaded)
            }
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
            Message::ActionSelected(choice) => {
                if let (Some(idx), Some(p)) = (self.selected_key, &mut self.profile) {
                    if let Some(k) = p.keys.get_mut(idx) {
                        let settings = default_settings_for_action(&self.plugins, &choice);
                        k.action = Some(ActionBinding {
                            plugin_id: choice.plugin_id.clone(),
                            action_id: choice.action_id.clone(),
                            settings,
                        });
                    }
                }
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
        let header = column![
            text("RiverDeck-Redux").size(32),
            text("Clean-room rewrite • Iced UI • OpenAction-only").size(16),
        ]
        .spacing(6);

        let left = self.view_device_panel();
        let right = self.view_connected_panel();

        let content = row![left, right]
            .spacing(16)
            .width(Length::Fill)
            .height(Length::Fill);

        let mut body = column![header, content]
            .spacing(16)
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_items(Alignment::Start);

        if let Some(err) = &self.error {
            body =
                body.push(container(text(err).style(Color::from_rgb8(255, 140, 140))).padding(8));
        }

        body.into()
    }

    fn subscription(&self) -> iced::Subscription<Self::Message> {
        iced::time::every(Duration::from_millis(33)).map(|_| Message::Tick)
    }
}

#[derive(Debug, Clone)]
enum Message {
    RefreshDevices,
    DevicesLoaded(Result<Vec<DiscoveredDevice>, String>),
    SelectDevice(app_core::ids::DeviceId),
    ConnectSelected,
    Connected(Result<ConnectedInfo, String>),
    RefreshProfiles,
    ProfilesLoaded(Result<Vec<ProfileMeta>, String>),
    CreateProfile,
    ProfileCreated(Result<Profile, String>),
    SelectProfile(ProfileId),
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
    ActionSelected(ActionChoice),
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
    fn view_device_panel(&self) -> Element<'_, Message> {
        let mut col = column![text("Devices").size(18)].spacing(8);

        let refresh = button("Refresh").on_press(Message::RefreshDevices);
        col = col.push(refresh);

        if self.devices.is_empty() {
            col = col.push(text("No devices found."));
        } else {
            for d in &self.devices {
                let is_selected = Some(d.id) == self.selected_device;
                let label = if is_selected {
                    format!("▶ {}", d.display_name)
                } else {
                    d.display_name.clone()
                };
                col = col.push(button(text(label)).on_press(Message::SelectDevice(d.id)));
            }

            let connect_btn = if self.connecting {
                button("Connecting…")
            } else {
                button("Connect").on_press(Message::ConnectSelected)
            };
            col = col.push(connect_btn);
        }

        col = col.push(text("Profiles").size(18));
        col = col.push(
            row![
                button("Refresh").on_press(Message::RefreshProfiles),
                button("New").on_press(Message::CreateProfile)
            ]
            .spacing(8),
        );

        if self.profiles.is_empty() {
            col = col.push(text("No profiles."));
        } else {
            for p in &self.profiles {
                let is_selected = Some(p.id) == self.selected_profile;
                let label = if is_selected {
                    format!("▶ {}", p.name)
                } else {
                    p.name.clone()
                };
                col = col.push(button(text(label)).on_press(Message::SelectProfile(p.id)));
            }
        }

        col = col.push(text("Plugins").size(18));
        col = col.push(
            row![
                text_input(
                    "Local plugin dir (contains manifest.json)",
                    &self.install_plugin_path
                )
                .on_input(Message::InstallPluginPathChanged),
                button("Install").on_press(Message::InstallPluginFromPath),
            ]
            .spacing(8),
        );
        col = col.push(button("Refresh plugins").on_press(Message::RefreshPlugins));
        if self.plugins.is_empty() {
            col = col.push(text("No plugins installed."));
        } else {
            for p in &self.plugins {
                col = col.push(text(format!("• {}", p.manifest.name)));
            }
        }

        container(col)
            .padding(12)
            .width(Length::FillPortion(1))
            .into()
    }

    fn view_connected_panel(&self) -> Element<'_, Message> {
        let mut col = column![text("Device").size(18)].spacing(10);

        match &self.connected {
            None => {
                col = col.push(text("Not connected."));
            }
            Some(c) => {
                col = col.push(text(format!("Connected: {}", c.name)));
                col = col.push(text(format!("ID: {}", c.id.0)));
                col = col.push(text(format!("Keys: {}", c.key_count)));

                col = col.push(text(format!("Brightness: {}%", c.brightness)));
                col = col.push(slider(
                    0..=100,
                    c.brightness as i32,
                    Message::BrightnessChanged,
                ));

                if let Some(p) = &self.profile {
                    col = col.push(text(format!("Profile: {}", p.name)).size(16));
                } else {
                    col = col.push(text("Profile: (none loaded)").size(16));
                }

                col = col.push(self.view_key_grid(c.key_count, &c.pressed));

                if let Some(idx) = self.selected_key {
                    col = col.push(text(format!("Edit key {idx}")).size(16));
                    col = col.push(text("Label"));
                    col = col.push(
                        text_input("Label", &self.edit_label).on_input(Message::LabelChanged),
                    );

                    col = col.push(text("Action"));
                    let current = self.current_action_choice(idx);
                    col = col.push(pick_list(
                        self.actions.clone(),
                        current,
                        Message::ActionSelected,
                    ));

                    col = col.push(self.view_action_settings(idx));
                    col = col.push(button("Save profile").on_press(Message::SaveProfile));
                } else {
                    col = col.push(text("Select a key to edit its label."));
                }
            }
        }

        container(col)
            .padding(12)
            .width(Length::FillPortion(2))
            .height(Length::Fill)
            .into()
    }

    fn view_key_grid(&self, key_count: u8, pressed: &[bool]) -> Element<'_, Message> {
        let (cols, rows) = match key_count {
            6 => (3, 2),
            32 => (8, 4),
            _ => (5, 3),
        };

        let mut outer = column![];
        for r in 0..rows {
            let mut line = row![].spacing(8);
            for c in 0..cols {
                let idx = r * cols + c;
                if idx >= key_count as usize {
                    break;
                }
                let is_down = pressed.get(idx).copied().unwrap_or(false);
                let label = self
                    .profile
                    .as_ref()
                    .and_then(|p| p.keys.get(idx))
                    .map(|k| k.label.as_str())
                    .unwrap_or("");
                let caption = if label.is_empty() {
                    format!("{idx}")
                } else {
                    format!("{idx}: {label}")
                };

                let is_selected = self.selected_key == Some(idx);
                let style = if is_down {
                    iced::theme::Button::Positive
                } else if is_selected {
                    iced::theme::Button::Primary
                } else {
                    iced::theme::Button::Secondary
                };

                let cell = button(text(caption).size(14))
                    .width(Length::FillPortion(1))
                    .padding(12)
                    .on_press(Message::SelectKey(idx))
                    .style(style);

                line = line.push(cell);
            }
            outer = outer.push(line);
        }
        outer.spacing(8).into()
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
