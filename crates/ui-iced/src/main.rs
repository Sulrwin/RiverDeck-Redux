use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::Duration;
use std::time::Instant;
use std::{fmt, sync::Arc};

use actions::{ActionBinding, ActionStep, BuiltinAction, PluginActionBinding};
use app_core::AppCore;
use device::{
    ControlEventKind, ControlId, DeviceController, DeviceEvent, DeviceService, DiscoveredDevice,
    HidDeviceService,
};
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, horizontal_space, image, mouse_area,
    pick_list, row, scrollable, slider, text, text_input,
};
use iced::{
    alignment::Horizontal, Alignment, Application, Background, Border, Color, Command, Element,
    Length, Settings, Shadow, Theme,
};
use tokio::sync::mpsc::Receiver;

use app_core::ids::ProfileId;
use storage::profiles::{Profile, ProfileMeta};

use openaction::manifest::{ActionDefinition, SettingField, SettingType};
use openaction::marketplace::MarketplacePlugin;
use openaction::registry::InstalledPlugin;
use plugin_runtime::{ActionRuntime, InvocationControl, InvocationEvent};

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
    selected_control: Option<SelectedControl>,
    selected_binding_target: BindingTarget,
    edit_label: String,
    edit_bg_rgb: String,
    edit_icon_path: String,
    edit_display_text: String,
    plugins: Vec<InstalledPlugin>,
    actions: Vec<ActionChoice>,
    action_search: String,
    install_plugin_path: String,
    active_view: ActiveView,
    marketplace: MarketplaceState,
    error: Option<String>,
    next_action_seq_id: u64,
    action_sequences: HashMap<u64, ActionSequence>,
    sys: sysinfo::System,
    sys_last_refresh: Instant,
    sys_snapshot: SystemSnapshot,
    drag: DragState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectedControl {
    Key(usize),
    Dial(usize),
    TouchStrip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingTarget {
    KeyPress,
    DialPress,
    DialRotate,
    TouchTap,
    TouchDrag,
}

impl fmt::Display for BindingTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingTarget::KeyPress => write!(f, "Key press"),
            BindingTarget::DialPress => write!(f, "Dial press"),
            BindingTarget::DialRotate => write!(f, "Dial rotate"),
            BindingTarget::TouchTap => write!(f, "Touch tap"),
            BindingTarget::TouchDrag => write!(f, "Touch drag"),
        }
    }
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
    icon_cache: HashMap<String, iced::widget::image::Handle>,
    image_cache: HashMap<String, iced::widget::image::Handle>,
    svg_cache: HashMap<String, iced::widget::svg::Handle>,
    details_cache: HashMap<String, MarketplaceDetails>,
    page: usize,
    installing: Option<String>,
    selected: Option<MarketplacePlugin>,
}

#[derive(Debug, Clone)]
struct ActionSequence {
    origin_control: InvocationControl,
    origin_event: InvocationEvent,
    steps: VecDeque<ActionStep>,
}

#[derive(Debug, Clone, Default)]
struct SystemSnapshot {
    cpu_percent: f32,
    mem_used: u64,
    mem_total: u64,
    load: (f64, f64, f64),
}

#[derive(Debug, Clone, Default)]
struct DragState {
    dragging: Option<DraggedAction>,
    over_key: Option<usize>,
}

#[derive(Debug, Clone, Default)]
struct MarketplaceDetails {
    /// Resolved direct download URL (ideally an archive asset URL).
    resolved_download_url: Option<String>,
    /// Total download count across releases (best-effort).
    total_downloads: Option<u64>,
    /// Repository URL (usually GitHub) if known.
    repository: Option<String>,
    /// README markdown (best-effort).
    readme_md: Option<String>,
    /// Discovered image URLs from README and/or marketplace metadata.
    image_urls: Vec<String>,
}

#[derive(Debug, Clone)]
enum DraggedAction {
    Plugin(ActionChoice),
    Builtin(BuiltinKindChoice),
}

impl DraggedAction {
    fn label(&self) -> String {
        match self {
            DraggedAction::Plugin(a) => a.label.clone(),
            DraggedAction::Builtin(k) => k.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MarketplaceSource {
    name: String,
    index_url: String,
    icon_base_url: Option<String>,
}

impl fmt::Display for MarketplaceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

const MARKETPLACE_PAGE_SIZE: usize = 50;

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
            selected_control: None,
            selected_binding_target: BindingTarget::KeyPress,
            edit_label: String::new(),
            edit_bg_rgb: String::new(),
            edit_icon_path: String::new(),
            edit_display_text: String::new(),
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
                icon_cache: HashMap::new(),
                image_cache: HashMap::new(),
                svg_cache: HashMap::new(),
                details_cache: HashMap::new(),
                page: 0,
                installing: None,
                selected: None,
            },
            error: None,
            next_action_seq_id: 1,
            action_sequences: HashMap::new(),
            sys: sysinfo::System::new(),
            sys_last_refresh: Instant::now(),
            sys_snapshot: SystemSnapshot::default(),
            drag: DragState::default(),
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
                self.selected_control = None;
                self.edit_label.clear();
                self.edit_bg_rgb.clear();
                self.edit_icon_path.clear();
                self.edit_display_text.clear();
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
                self.selected_control = None;
                self.edit_label.clear();
                self.edit_bg_rgb.clear();
                self.edit_icon_path.clear();
                self.edit_display_text.clear();
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
                    self.selected_control = None;
                    self.edit_label.clear();
                    self.edit_bg_rgb.clear();
                    self.edit_icon_path.clear();
                    self.edit_display_text.clear();
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
            Message::SelectControl(sel) => {
                self.selected_control = Some(sel);
                self.selected_binding_target = match sel {
                    SelectedControl::Key(_) => BindingTarget::KeyPress,
                    SelectedControl::Dial(_) => BindingTarget::DialPress,
                    SelectedControl::TouchStrip => BindingTarget::TouchTap,
                };

                self.edit_label.clear();
                self.edit_bg_rgb.clear();
                self.edit_icon_path.clear();
                self.edit_display_text.clear();

                if let Some(p) = &self.profile {
                    match sel {
                        SelectedControl::Key(idx) => {
                            if let Some(k) = p.keys.get(idx) {
                                self.edit_label = k.label.clone();
                                self.edit_icon_path = k.appearance.icon_path.clone().unwrap_or_default();
                                self.edit_display_text = k.appearance.text.clone().unwrap_or_default();
                                self.edit_bg_rgb = match k.appearance.background {
                                    storage::profiles::Background::Solid { rgb } => {
                                        format!("{},{},{}", rgb[0], rgb[1], rgb[2])
                                    }
                                    storage::profiles::Background::None => String::new(),
                                };
                            }
                        }
                        SelectedControl::Dial(idx) => {
                            if let Some(d) = p.dials.get(idx) {
                                self.edit_label = d.label.clone();
                                self.edit_icon_path = d.appearance.icon_path.clone().unwrap_or_default();
                                self.edit_display_text = d.appearance.text.clone().unwrap_or_default();
                                self.edit_bg_rgb = match d.appearance.background {
                                    storage::profiles::Background::Solid { rgb } => {
                                        format!("{},{},{}", rgb[0], rgb[1], rgb[2])
                                    }
                                    storage::profiles::Background::None => String::new(),
                                };
                            }
                        }
                        SelectedControl::TouchStrip => {
                            let a = &p.touch_strip.appearance;
                            self.edit_icon_path = a.icon_path.clone().unwrap_or_default();
                            self.edit_display_text = a.text.clone().unwrap_or_default();
                            self.edit_bg_rgb = match a.background {
                                storage::profiles::Background::Solid { rgb } => {
                                    format!("{},{},{}", rgb[0], rgb[1], rgb[2])
                                }
                                storage::profiles::Background::None => String::new(),
                            };
                        }
                    }
                }

                Command::none()
            }
            Message::LabelChanged(val) => {
                self.edit_label = val;
                if let (Some(sel), Some(p)) = (self.selected_control, &mut self.profile) {
                    match sel {
                        SelectedControl::Key(idx) => {
                            if let Some(k) = p.keys.get_mut(idx) {
                                k.label = self.edit_label.clone();
                                // Keep default display text in sync unless user customized it.
                                if k.appearance.text.as_deref().unwrap_or("").is_empty() {
                                    k.appearance.text = Some(k.label.clone());
                                    self.edit_display_text = k.label.clone();
                                }
                            }
                        }
                        SelectedControl::Dial(idx) => {
                            if let Some(d) = p.dials.get_mut(idx) {
                                d.label = self.edit_label.clone();
                            }
                        }
                        SelectedControl::TouchStrip => {}
                    }
                }
                Command::none()
            }
            Message::BindingTargetPicked(t) => {
                self.selected_binding_target = t;
                Command::none()
            }
            Message::BgRgbChanged(v) => {
                self.edit_bg_rgb = v;
                if let (Some(sel), Some(p)) = (self.selected_control, &mut self.profile) {
                    let bg = parse_bg_rgb(&self.edit_bg_rgb);
                    let background = bg
                        .map(|rgb| storage::profiles::Background::Solid { rgb })
                        .unwrap_or(storage::profiles::Background::None);

                    match sel {
                        SelectedControl::Key(idx) => {
                            if let Some(k) = p.keys.get_mut(idx) {
                                k.appearance.background = background;
                            }
                        }
                        SelectedControl::Dial(idx) => {
                            if let Some(d) = p.dials.get_mut(idx) {
                                d.appearance.background = background;
                            }
                        }
                        SelectedControl::TouchStrip => {
                            p.touch_strip.appearance.background = background;
                        }
                    }
                }
                Command::none()
            }
            Message::IconPathChanged(v) => {
                self.edit_icon_path = v;
                if let (Some(sel), Some(p)) = (self.selected_control, &mut self.profile) {
                    let val = self.edit_icon_path.trim();
                    let new = if val.is_empty() { None } else { Some(val.to_string()) };
                    match sel {
                        SelectedControl::Key(idx) => {
                            if let Some(k) = p.keys.get_mut(idx) {
                                k.appearance.icon_path = new;
                            }
                        }
                        SelectedControl::Dial(idx) => {
                            if let Some(d) = p.dials.get_mut(idx) {
                                d.appearance.icon_path = new;
                            }
                        }
                        SelectedControl::TouchStrip => {
                            p.touch_strip.appearance.icon_path = new;
                        }
                    }
                }
                Command::none()
            }
            Message::DisplayTextChanged(v) => {
                self.edit_display_text = v;
                if let (Some(sel), Some(p)) = (self.selected_control, &mut self.profile) {
                    let val = self.edit_display_text.trim();
                    let new = if val.is_empty() { None } else { Some(val.to_string()) };
                    match sel {
                        SelectedControl::Key(idx) => {
                            if let Some(k) = p.keys.get_mut(idx) {
                                k.appearance.text = new;
                            }
                        }
                        SelectedControl::Dial(idx) => {
                            if let Some(d) = p.dials.get_mut(idx) {
                                d.appearance.text = new;
                            }
                        }
                        SelectedControl::TouchStrip => {
                            p.touch_strip.appearance.text = new;
                        }
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
                match res {
                    Ok(()) => {
                        // Best-effort: push LCD displays after saving.
                        let Some(c) = self.connected.as_ref() else {
                            return Command::none();
                        };
                        let Some(p) = self.profile.clone() else {
                            return Command::none();
                        };
                        let controller = c.controller.clone();
                        Command::perform(apply_displays_async(controller, p), Message::DisplaysApplied)
                    }
                    Err(e) => {
                        self.error = Some(e);
                        Command::none()
                    }
                }
            }
            Message::DisplaysApplied(res) => {
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
                self.marketplace.page = 0;
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
                        Some("Marketplace URL is not configured.".to_string());
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
                        Some("Marketplace URL is not configured.".to_string());
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
            Message::MarketplaceSearchChanged(q) => {
                self.marketplace.query = q;
                self.marketplace.page = 0;
                Command::none()
            }
            Message::MarketplaceLoaded(res) => {
                self.marketplace.loading = false;
                match res {
                    Ok(list) => {
                        self.marketplace.plugins = list;
                        self.marketplace.error = None;
                        // Kick off icon fetches for the current marketplace source.
                        if let Some(src) = self.current_marketplace_source().cloned() {
                            let mut cmds = vec![];
                            let q = self.marketplace.query.trim().to_ascii_lowercase();
                            for p in self
                                .marketplace
                                .plugins
                                .iter()
                                .filter(|p| {
                                    if q.is_empty() {
                                        true
                                    } else {
                                        p.name.to_ascii_lowercase().contains(&q)
                                            || p.id.to_ascii_lowercase().contains(&q)
                                            || p.description.to_ascii_lowercase().contains(&q)
                                    }
                                })
                                .skip(self.marketplace.page.saturating_mul(MARKETPLACE_PAGE_SIZE))
                                .take(MARKETPLACE_PAGE_SIZE)
                            {
                                let Some(icon_url) = marketplace_icon_url(&src, p) else {
                                    continue;
                                };
                                let key = format!("{}|{}", src.index_url, p.id);
                                if self.marketplace.icon_cache.contains_key(&key) {
                                    continue;
                                }
                                cmds.push(Command::perform(
                                    fetch_icon_async(icon_url),
                                    {
                                        let key = key.clone();
                                        move |bytes| Message::MarketplaceIconLoaded { key, bytes }
                                    },
                                ));
                            }
                            return Command::batch(cmds);
                        }
                    }
                    Err(e) => {
                        self.marketplace.plugins.clear();
                        self.marketplace.error = Some(e);
                    }
                }
                Command::none()
            }
            Message::MarketplaceInstall(p) => {
                if self.marketplace.installing.is_some() {
                    return Command::none();
                }

                let Some(src) = self.current_marketplace_source().cloned() else {
                    self.marketplace.error = Some("No marketplace selected.".to_string());
                    return Command::none();
                };

                // Prefer a direct download URL from the marketplace feed.
                let url = resolve_marketplace_download_url(&src, &p);

                if self.plugins.iter().any(|ip| ip.manifest.id == p.id) {
                    return Command::none();
                }

                self.marketplace.installing = Some(p.id.clone());
                self.marketplace.error = None;
                if let Some(url) = url {
                    Command::perform(
                        install_marketplace_async(url, p.id),
                        Message::MarketplaceInstalled,
                    )
                } else if let Some(repo) = p.repository.clone() {
                    // Rivul marketplace derives downloads from the GitHub repository.
                    // We attempt to resolve a release asset URL and install it.
                    Command::perform(
                        install_marketplace_from_repo_async(repo, p.id),
                        Message::MarketplaceInstalled,
                    )
                        } else {
                    self.marketplace.installing = None;
                    self.marketplace.error =
                        Some("No installable download found for this plugin.".to_string());
                    Command::none()
                }
            }
            Message::MarketplaceInstalled(res) => {
                self.marketplace.installing = None;
                match res {
                    Ok(()) => {
                        self.marketplace.error = None;
                        Command::perform(list_plugins_async(), Message::PluginsLoaded)
                    }
                    Err(e) => {
                        self.marketplace.error = Some(e);
                        Command::none()
                    }
                }
            }
            Message::MarketplacePrevPage => {
                if self.marketplace.page == 0 {
                    return Command::none();
                }
                self.marketplace.page = self.marketplace.page.saturating_sub(1);
                self.marketplace_fetch_icons_for_current_page()
            }
            Message::MarketplaceNextPage => {
                self.marketplace.page = self.marketplace.page.saturating_add(1);
                self.marketplace_fetch_icons_for_current_page()
            }
            Message::MarketplaceIconLoaded { key, bytes } => {
                if let Ok(bytes) = bytes {
                    self.marketplace
                        .icon_cache
                        .insert(key, iced::widget::image::Handle::from_memory(bytes));
                }
                Command::none()
            }
            Message::MarketplaceSelect(p) => {
                self.marketplace.selected = Some(p.clone());
                self.marketplace.error = None;

                let Some(src) = self.current_marketplace_source().cloned() else {
                    return Command::none();
                };

                let mut cmds = vec![];

                // Ensure the selected icon is fetched (may not be in visible range).
                if let Some(icon_url) = marketplace_icon_url(&src, &p) {
                    let key = format!("{}|{}", src.index_url, p.id);
                    if !self.marketplace.icon_cache.contains_key(&key) {
                        cmds.push(Command::perform(
                            fetch_icon_async(icon_url),
                            {
                                let key = key.clone();
                                move |bytes| Message::MarketplaceIconLoaded { key, bytes }
                            },
                        ));
                    }
                }

                // Fetch screenshots / images lazily.
                for raw in p.images.iter().take(8) {
                    let Some(url) = resolve_marketplace_asset_url(&src, raw) else {
                        continue;
                    };
                    let key = format!("img|{}|{}|{}", src.index_url, p.id, url);
                    if self.marketplace.image_cache.contains_key(&key)
                        || self.marketplace.svg_cache.contains_key(&key)
                    {
                        continue;
                    }
                    if url.to_ascii_lowercase().contains(".svg") {
                        cmds.push(Command::perform(
                            fetch_icon_async(url),
                            {
                                let key = key.clone();
                                move |bytes| Message::MarketplaceSvgLoaded { key, bytes }
                            },
                        ));
                    } else {
                        cmds.push(Command::perform(
                            fetch_icon_async(url),
                            {
                                let key = key.clone();
                                move |bytes| Message::MarketplaceImageLoaded { key, bytes }
                            },
                        ));
                    }
                }

                // Fetch richer details (README, derived downloads, README images).
                if !self.marketplace.details_cache.contains_key(&p.id) {
                    cmds.push(Command::perform(
                        fetch_marketplace_details_async(p.clone()),
                        {
                            let plugin_id = p.id.clone();
                            move |details| Message::MarketplaceDetailsLoaded { plugin_id, details }
                        },
                    ));
                }

                if cmds.is_empty() {
                    Command::none()
                } else {
                Command::batch(cmds)
            }
            }
            Message::MarketplaceImageLoaded { key, bytes } => {
                if let Ok(bytes) = bytes {
                    self.marketplace
                        .image_cache
                        .insert(key, iced::widget::image::Handle::from_memory(bytes));
                }
                Command::none()
            }
            Message::MarketplaceSvgLoaded { key, bytes } => {
                if let Ok(bytes) = bytes {
                    self.marketplace
                        .svg_cache
                        .insert(key, iced::widget::svg::Handle::from_memory(bytes));
                }
                Command::none()
            }
            Message::MarketplaceDetailsLoaded { plugin_id, details } => {
                match details {
                    Ok(d) => {
                        // Merge cache.
                        let mut merged = d.clone();
                        if let Some(p) = self.marketplace.plugins.iter().find(|p| p.id == plugin_id) {
                            // include any marketplace-provided images too
                            for u in &p.images {
                                if !merged.image_urls.contains(u) {
                                    merged.image_urls.push(u.clone());
                                }
                            }
                        }
                        self.marketplace.details_cache.insert(plugin_id.clone(), merged.clone());

                        // If we discovered a direct download URL, update the marketplace plugin in-place
                        // so the list can show Install immediately.
                        if let Some(url) = merged.resolved_download_url.clone() {
                            if let Some(p) = self
                                .marketplace
                                .plugins
                                .iter_mut()
                                .find(|p| p.id == plugin_id)
                            {
                                if p.download_url.as_deref().map(|s| s.trim()).unwrap_or("").is_empty()
                                {
                                    p.download_url = Some(url.clone());
                                }
                            }
                            if let Some(sel) = self.marketplace.selected.as_mut() {
                                if sel.id == plugin_id
                                    && sel.download_url.as_deref().map(|s| s.trim()).unwrap_or("").is_empty()
                                {
                                    sel.download_url = Some(url);
                                }
                            }
                        }

                        // Kick off image fetches for discovered URLs (best-effort).
                        let Some(src) = self.current_marketplace_source().cloned() else {
                            return Command::none();
                        };
                        let mut cmds = vec![];
                        for raw in merged.image_urls.iter().take(12) {
                            if !is_renderable_image_url(raw) {
                                continue;
                            }
                            let Some(url) = resolve_marketplace_asset_url(&src, raw) else {
                                continue;
                            };
                            let key = format!("img|{}|{}|{}", src.index_url, plugin_id, url);
                            if self.marketplace.image_cache.contains_key(&key)
                                || self.marketplace.svg_cache.contains_key(&key)
                            {
                                continue;
                            }
                            if url.to_ascii_lowercase().contains(".svg") {
                                cmds.push(Command::perform(
                                    fetch_icon_async(url),
                                    {
                                        let key = key.clone();
                                        move |bytes| Message::MarketplaceSvgLoaded { key, bytes }
                                    },
                                ));
                            } else {
                                cmds.push(Command::perform(
                                    fetch_icon_async(url),
                                    {
                                        let key = key.clone();
                                        move |bytes| Message::MarketplaceImageLoaded { key, bytes }
                                    },
                                ));
                            }
                        }
                        if cmds.is_empty() {
                            Command::none()
                        } else {
                            Command::batch(cmds)
                        }
                    }
                    Err(e) => {
                        // Keep the UI usable even if GitHub is rate-limited.
                        tracing::debug!(plugin_id, error=%e, "marketplace details fetch failed");
                        Command::none()
                    }
                }
            }
            Message::OpenUrl(url) => {
                Command::perform(open_url_async(url), |_| Message::Tick)
            }
            Message::ActionSeqContinue(seq_id) => self.run_next_action_step(seq_id),
            Message::ActionSeqStepDone { seq_id, res } => {
                if let Err(e) = res {
                    tracing::error!(seq_id, error = %e, "action step failed");
                    self.error = Some(e);
                } else {
                    tracing::debug!(seq_id, "action step completed");
                }
                self.run_next_action_step(seq_id)
            }
            Message::ActionModePicked(mode) => {
                self.set_selected_action_mode(mode);
                Command::none()
            }
            Message::BuiltinKindPicked(kind) => {
                self.set_selected_builtin_kind(kind);
                Command::none()
            }
            Message::BuiltinIssueCommandChanged(v) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::IssueCommand { command, .. } = b {
                        *command = v;
                    }
                });
                Command::none()
            }
            Message::BuiltinIssueCwdChanged(v) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::IssueCommand { cwd, .. } = b {
                        let s = v.trim().to_string();
                        *cwd = if s.is_empty() { None } else { Some(s) };
                    }
                });
                Command::none()
            }
            Message::BuiltinIssueTimeoutChanged(v) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::IssueCommand { timeout_ms, .. } = b {
                        let s = v.trim();
                        *timeout_ms = if s.is_empty() {
                            None
                        } else {
                            s.parse::<u64>().ok()
                        };
                    }
                });
                Command::none()
            }
            Message::BuiltinKeyboardTextChanged(v) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::KeyboardInput { text, .. } = b {
                        let s = v;
                        *text = if s.trim().is_empty() { None } else { Some(s) };
                    }
                });
                Command::none()
            }
            Message::BuiltinKeyboardKeysChanged(v) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::KeyboardInput { keys, .. } = b {
                        *keys = v
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>();
                    }
                });
                Command::none()
            }
            Message::BuiltinPlaySoundPathChanged(v) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::PlaySound { path } = b {
                        *path = v;
                    }
                });
                Command::none()
            }
            Message::BuiltinSwitchProfilePicked(choice) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::SwitchProfile { mode } = b {
                        *mode = match choice {
                            SwitchProfileChoice::Next => actions::SwitchProfileMode::Next,
                            SwitchProfileChoice::Prev => actions::SwitchProfileMode::Prev,
                            SwitchProfileChoice::To(id) => {
                                actions::SwitchProfileMode::To { profile_id: id.0 }
                            }
                        };
                    }
                });
                Command::none()
            }
            Message::BuiltinBrightnessModePicked(m) => {
                // Keep existing value if possible.
                let current = self.selected_builtin_brightness_value().unwrap_or(30);
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::DeviceBrightness { mode } = b {
                        *mode = match m {
                            BrightnessModeChoice::Set => actions::BrightnessMode::Set {
                                percent: current as u8,
                            },
                            BrightnessModeChoice::Increase => actions::BrightnessMode::Increase {
                                delta: current as u8,
                            },
                            BrightnessModeChoice::Decrease => actions::BrightnessMode::Decrease {
                                delta: current as u8,
                            },
                        };
                    }
                });
                Command::none()
            }
            Message::BuiltinBrightnessValueChanged(v) => {
                let v = v.clamp(0, 100) as u8;
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::DeviceBrightness { mode } = b {
                        match mode {
                            actions::BrightnessMode::Set { percent } => *percent = v,
                            actions::BrightnessMode::Increase { delta } => *delta = v,
                            actions::BrightnessMode::Decrease { delta } => *delta = v,
                        }
                    }
                });
                Command::none()
            }
            Message::BuiltinMonitorKindPicked(k) => {
                self.update_selected_builtin(|b| {
                    if let BuiltinAction::SystemMonitoring { kind, .. } = b {
                        *kind = match k {
                            MonitorKindChoice::Cpu => actions::MonitorKind::Cpu,
                            MonitorKindChoice::Memory => actions::MonitorKind::Memory,
                            MonitorKindChoice::LoadAverage => actions::MonitorKind::LoadAverage,
                        }
                    }
                });
                Command::none()
            }
            Message::MacroAddStep => {
                self.macro_add_step();
                Command::none()
            }
            Message::MacroRemoveStep(i) => {
                self.macro_remove_step(i);
                Command::none()
            }
            Message::MacroMoveStepUp(i) => {
                self.macro_move_step(i, true);
                Command::none()
            }
            Message::MacroMoveStepDown(i) => {
                self.macro_move_step(i, false);
                Command::none()
            }
            Message::MacroStepKindPicked { idx, kind } => {
                self.macro_set_step_kind(idx, kind);
                Command::none()
            }
            Message::MacroStepDelayChanged { idx, value } => {
                self.macro_set_step_delay(idx, value);
                Command::none()
            }
            Message::MacroStepPluginPicked { idx, choice } => {
                self.macro_set_step_plugin(idx, choice);
                Command::none()
            }
            Message::MacroStepCommandChanged { idx, value } => {
                self.macro_set_step_command(idx, value);
                Command::none()
            }
            Message::StartDragAction(a) => {
                self.drag.dragging = Some(a);
                self.drag.over_key = None;
                Command::none()
            }
            Message::CancelDragAction => {
                self.drag.dragging = None;
                self.drag.over_key = None;
                Command::none()
            }
            Message::DragOverKey(idx) => {
                self.drag.over_key = idx;
                Command::none()
            }
            Message::DropOnKey(idx) => {
                let Some(dragged) = self.drag.dragging.clone() else {
                    return Command::none();
                };
                self.drag.dragging = None;
                self.drag.over_key = None;

                self.selected_control = Some(SelectedControl::Key(idx));
                self.selected_binding_target = BindingTarget::KeyPress;
                self.assign_dragged_action_to_key(idx, dragged);
                Command::none()
            }
            Message::ActionSelected(choice) => {
                if self.selected_control.is_none() {
                    self.error = Some(
                        "Select a key/dial/touch strip in the preview before assigning an action."
                            .to_string(),
                    );
                    return Command::none();
                };
                let Some(_p) = &mut self.profile else {
                    self.error = Some("No profile loaded.".to_string());
                    return Command::none();
                };

                let settings = default_settings_for_action(&self.plugins, &choice);
                let Some(slot) = self.selected_binding_mut() else {
                    self.error = Some("Invalid binding target for selected control.".to_string());
                    return Command::none();
                };
                *slot = Some(ActionBinding::Plugin(PluginActionBinding {
                    plugin_id: choice.plugin_id.clone(),
                    action_id: choice.action_id.clone(),
                    settings,
                }));
                Command::none()
            }
            Message::ActionSearchChanged(s) => {
                self.action_search = s;
                Command::none()
            }
            Message::SettingStringChanged { key, value } => {
                self.set_selected_plugin_setting(key, serde_json::Value::String(value));
                Command::none()
            }
            Message::SettingBoolChanged { key, value } => {
                self.set_selected_plugin_setting(key, serde_json::Value::Bool(value));
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
                self.set_selected_plugin_setting(key, v);
                Command::none()
            }
            Message::Tick => {
                let mut cmds: Vec<Command<Message>> = vec![];
                self.refresh_system_snapshot();
                let mut pending_actions: Vec<(InvocationControl, InvocationEvent, ActionBinding)> =
                    vec![];
                if let Some(c) = &mut self.connected {
                    while let Ok(ev) = c.events.try_recv() {
                        match ev {
                            DeviceEvent::Control(ev) => match (ev.control, ev.kind) {
                                (ControlId::Key(key), ControlEventKind::Down) => {
                                    if let Some(slot) = c.pressed.get_mut(key as usize) {
                                        *slot = true;
                                    }

                                    // Dispatch bound action on key-down (plugin or builtin).
                                    if let Some(p) = &self.profile {
                                        if let Some(kcfg) = p.keys.get(key as usize) {
                                            if let Some(binding) = &kcfg.action {
                                                pending_actions.push((
                                                    InvocationControl::Key { index: key },
                                                    InvocationEvent::KeyDown,
                                                    binding.clone(),
                                                ));
                                            }
                                        }
                                    }
                                }
                                (ControlId::Key(key), ControlEventKind::Up) => {
                                    if let Some(slot) = c.pressed.get_mut(key as usize) {
                                        *slot = false;
                                    }
                                }
                                (ControlId::Dial(dial), ControlEventKind::Down) => {
                                    if let Some(p) = &self.profile {
                                        if let Some(d) = p.dials.get(dial as usize) {
                                            if let Some(binding) = &d.press {
                                                pending_actions.push((
                                                    InvocationControl::Dial { index: dial },
                                                    InvocationEvent::DialDown,
                                                    binding.clone(),
                                                ));
                                            }
                                        }
                                    }
                                }
                                (ControlId::Dial(dial), ControlEventKind::Rotate { delta }) => {
                                    if let Some(p) = &self.profile {
                                        if let Some(d) = p.dials.get(dial as usize) {
                                            if let Some(binding) = &d.rotate {
                                                pending_actions.push((
                                                    InvocationControl::Dial { index: dial },
                                                    InvocationEvent::DialRotate { delta },
                                                    binding.clone(),
                                                ));
                                            }
                                        }
                                    }
                                }
                                (ControlId::TouchStrip, ControlEventKind::Tap { x }) => {
                                    if let Some(p) = &self.profile {
                                        if let Some(binding) = &p.touch_strip.tap {
                                            pending_actions.push((
                                                InvocationControl::TouchStrip,
                                                InvocationEvent::TouchTap { x },
                                                binding.clone(),
                                            ));
                                        }
                                    }
                                }
                                (ControlId::TouchStrip, ControlEventKind::Drag { delta_x }) => {
                                    if let Some(p) = &self.profile {
                                        if let Some(binding) = &p.touch_strip.drag {
                                            pending_actions.push((
                                                InvocationControl::TouchStrip,
                                                InvocationEvent::TouchDrag { delta_x },
                                                binding.clone(),
                                            ));
                                        }
                                    }
                                }
                                _ => {}
                            },
                            DeviceEvent::Disconnected => {
                                self.error = Some("Device disconnected".to_string());
                                self.connected = None;
                                break;
                            }
                        }
                    }
                }

                for (control, event, binding) in pending_actions {
                    cmds.push(self.start_action_sequence(control, event, &binding));
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
    SelectControl(SelectedControl),
    LabelChanged(String),
    BindingTargetPicked(BindingTarget),
    BgRgbChanged(String),
    IconPathChanged(String),
    DisplayTextChanged(String),
    SaveProfile,
    ProfileSaved(Result<(), String>),
    DisplaysApplied(Result<(), String>),
    RefreshPlugins,
    PluginsLoaded(Result<Vec<InstalledPlugin>, String>),
    InstallPluginPathChanged(String),
    InstallPluginFromPath,
    PluginInstalled(Result<(), String>),
    OpenMarketplace,
    CloseMarketplace,
    MarketplaceRefresh,
    MarketplaceSourcePicked(MarketplaceSource),
    MarketplaceSearchChanged(String),
    MarketplaceLoaded(Result<Vec<MarketplacePlugin>, String>),
    MarketplaceIconLoaded { key: String, bytes: Result<Vec<u8>, String> },
    MarketplacePrevPage,
    MarketplaceNextPage,
    MarketplaceSelect(MarketplacePlugin),
    MarketplaceImageLoaded { key: String, bytes: Result<Vec<u8>, String> },
    MarketplaceSvgLoaded { key: String, bytes: Result<Vec<u8>, String> },
    MarketplaceDetailsLoaded {
        plugin_id: String,
        details: Result<MarketplaceDetails, String>,
    },
    OpenUrl(String),
    MarketplaceInstall(MarketplacePlugin),
    MarketplaceInstalled(Result<(), String>),
    ActionSeqContinue(u64),
    ActionSeqStepDone { seq_id: u64, res: Result<(), String> },
    ActionModePicked(ActionModeChoice),
    BuiltinKindPicked(BuiltinKindChoice),
    BuiltinIssueCommandChanged(String),
    BuiltinIssueCwdChanged(String),
    BuiltinIssueTimeoutChanged(String),
    BuiltinKeyboardTextChanged(String),
    BuiltinKeyboardKeysChanged(String),
    BuiltinPlaySoundPathChanged(String),
    BuiltinSwitchProfilePicked(SwitchProfileChoice),
    BuiltinBrightnessModePicked(BrightnessModeChoice),
    BuiltinBrightnessValueChanged(i32),
    BuiltinMonitorKindPicked(MonitorKindChoice),
    MacroAddStep,
    MacroRemoveStep(usize),
    MacroMoveStepUp(usize),
    MacroMoveStepDown(usize),
    MacroStepKindPicked { idx: usize, kind: MacroStepKindChoice },
    MacroStepDelayChanged { idx: usize, value: String },
    MacroStepPluginPicked { idx: usize, choice: ActionChoice },
    MacroStepCommandChanged { idx: usize, value: String },
    StartDragAction(DraggedAction),
    CancelDragAction,
    DragOverKey(Option<usize>),
    DropOnKey(usize),
    ActionSelected(ActionChoice),
    ActionSearchChanged(String),
    SettingStringChanged { key: String, value: String },
    SettingBoolChanged { key: String, value: bool },
    SettingNumberChanged { key: String, value: String },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ActionModeChoice {
    None,
    Plugin,
    Builtin,
}

impl fmt::Display for ActionModeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionModeChoice::None => write!(f, "None"),
            ActionModeChoice::Plugin => write!(f, "Plugin"),
            ActionModeChoice::Builtin => write!(f, "Builtin"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BuiltinKindChoice {
    Macro,
    IssueCommand,
    KeyboardInput,
    PlaySound,
    SwitchProfile,
    DeviceBrightness,
    SystemMonitoring,
}

impl fmt::Display for BuiltinKindChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuiltinKindChoice::Macro => write!(f, "Macro"),
            BuiltinKindChoice::IssueCommand => write!(f, "Issue Command"),
            BuiltinKindChoice::KeyboardInput => write!(f, "Keyboard Input"),
            BuiltinKindChoice::PlaySound => write!(f, "Play Sound"),
            BuiltinKindChoice::SwitchProfile => write!(f, "Switch Profile"),
            BuiltinKindChoice::DeviceBrightness => write!(f, "Device Brightness"),
            BuiltinKindChoice::SystemMonitoring => write!(f, "System Monitoring"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SwitchProfileChoice {
    Next,
    Prev,
    // Specific profile chosen from list.
    To(ProfileId),
}

impl fmt::Display for SwitchProfileChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwitchProfileChoice::Next => write!(f, "Next profile"),
            SwitchProfileChoice::Prev => write!(f, "Previous profile"),
            SwitchProfileChoice::To(id) => write!(f, "Profile {}", id.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BrightnessModeChoice {
    Set,
    Increase,
    Decrease,
}

impl fmt::Display for BrightnessModeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrightnessModeChoice::Set => write!(f, "Set"),
            BrightnessModeChoice::Increase => write!(f, "Increase"),
            BrightnessModeChoice::Decrease => write!(f, "Decrease"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MonitorKindChoice {
    Cpu,
    Memory,
    LoadAverage,
}

impl fmt::Display for MonitorKindChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MonitorKindChoice::Cpu => write!(f, "CPU"),
            MonitorKindChoice::Memory => write!(f, "Memory"),
            MonitorKindChoice::LoadAverage => write!(f, "Load average"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MacroStepKindChoice {
    PluginAction,
    IssueCommand,
}

impl fmt::Display for MacroStepKindChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MacroStepKindChoice::PluginAction => write!(f, "Plugin action"),
            MacroStepKindChoice::IssueCommand => write!(f, "Issue command"),
        }
    }
}

impl App {
    fn current_marketplace_source(&self) -> Option<&MarketplaceSource> {
        self.marketplace
            .selected_source_idx
            .and_then(|i| self.marketplace.sources.get(i))
    }

    fn marketplace_fetch_icons_for_current_page(&self) -> Command<Message> {
        let Some(src) = self.current_marketplace_source().cloned() else {
            return Command::none();
        };

        let q = self.marketplace.query.trim().to_ascii_lowercase();
        let start = self.marketplace.page.saturating_mul(MARKETPLACE_PAGE_SIZE);

        let mut cmds = vec![];
        for p in self
            .marketplace
            .plugins
            .iter()
            .filter(|p| {
                if q.is_empty() {
                    true
                } else {
                    p.name.to_ascii_lowercase().contains(&q)
                        || p.id.to_ascii_lowercase().contains(&q)
                        || p.description.to_ascii_lowercase().contains(&q)
                }
            })
            .skip(start)
            .take(MARKETPLACE_PAGE_SIZE)
        {
            let Some(icon_url) = marketplace_icon_url(&src, p) else {
                continue;
            };
            let key = format!("{}|{}", src.index_url, p.id);
            if self.marketplace.icon_cache.contains_key(&key) {
                continue;
            }
            cmds.push(Command::perform(
                fetch_icon_async(icon_url),
                {
                    let key = key.clone();
                    move |bytes| Message::MarketplaceIconLoaded { key, bytes }
                },
            ));
        }

        if cmds.is_empty() {
            Command::none()
        } else {
            Command::batch(cmds)
        }
    }

    fn start_action_sequence(
        &mut self,
        control: InvocationControl,
        event: InvocationEvent,
        binding: &ActionBinding,
    ) -> Command<Message> {
        let steps = match actions::expand(binding) {
            Ok(steps) => steps,
            Err(e) => {
                tracing::error!(?control, ?event, error = %e, "failed to expand action binding");
                return Command::perform(async { () }, move |_| {
                    Message::ActionSeqStepDone {
                        seq_id: 0,
                        res: Err(e.to_string()),
                    }
                })
            }
        };

        let seq_id = self.next_action_seq_id;
        self.next_action_seq_id = self.next_action_seq_id.saturating_add(1);

        self.action_sequences.insert(
            seq_id,
            ActionSequence {
                origin_control: control.clone(),
                origin_event: event.clone(),
                steps: VecDeque::from(steps),
            },
        );

        tracing::info!(?control, ?event, seq_id, "starting action sequence");
        Command::perform(async { () }, move |_| Message::ActionSeqContinue(seq_id))
    }

    fn run_next_action_step(&mut self, seq_id: u64) -> Command<Message> {
        let Some((origin_control, origin_event)) = self
            .action_sequences
            .get(&seq_id)
            .map(|s| (s.origin_control.clone(), s.origin_event.clone()))
        else {
            return Command::none();
        };

        let step = {
            let Some(seq) = self.action_sequences.get_mut(&seq_id) else {
                return Command::none();
            };
            seq.steps.pop_front()
        };

        let Some(step) = step else {
            self.action_sequences.remove(&seq_id);
            tracing::debug!(seq_id, "action sequence finished");
            return Command::none();
        };

        match step {
            ActionStep::DelayMs(ms) => Command::perform(sleep_ms_async(ms), move |_| {
                tracing::debug!(seq_id, delay_ms = ms, "action sequence delay");
                Message::ActionSeqContinue(seq_id)
            }),
            ActionStep::Plugin(p) => {
                tracing::info!(
                    seq_id,
                    ?origin_control,
                    ?origin_event,
                    plugin_id = %p.plugin_id,
                    action_id = %p.action_id,
                    "executing plugin action"
                );
                let Some(plugin) = self.plugins.iter().find(|pl| pl.manifest.id == p.plugin_id).cloned() else {
                    return Command::perform(async { () }, move |_| Message::ActionSeqStepDone {
                        seq_id,
                        res: Err(format!("[Action] Plugin not installed: {}", p.plugin_id)),
                    });
                };
                let action_id = p.action_id.clone();
                let settings = p.settings.clone();
                Command::perform(
                    invoke_action_async(
                        plugin,
                        action_id,
                        origin_control.clone(),
                        origin_event.clone(),
                        settings,
                    ),
                    move |res| Message::ActionSeqStepDone { seq_id, res },
                )
            }
            ActionStep::Builtin(b) => {
                tracing::info!(seq_id, ?origin_control, builtin = ?b, "executing builtin action");
                self.execute_builtin_step(seq_id, origin_control.clone(), b)
            }
        }
    }

    fn execute_builtin_step(
        &mut self,
        seq_id: u64,
        origin_control: InvocationControl,
        b: actions::BuiltinAction,
    ) -> Command<Message> {
        match b {
            BuiltinAction::Macro { .. } => {
                // Macro should have been expanded away by `actions::expand`.
                Command::perform(async { () }, move |_| Message::ActionSeqStepDone {
                    seq_id,
                    res: Err("Internal: macro was not expanded".to_string()),
                })
            }
            BuiltinAction::IssueCommand { command, cwd, timeout_ms } => {
                tracing::info!(seq_id, ?origin_control, %command, "builtin: issue_command");
                Command::perform(issue_command_async(command, cwd, timeout_ms), move |res| {
                    Message::ActionSeqStepDone { seq_id, res }
                })
            }
            BuiltinAction::KeyboardInput { text, keys } => {
                tracing::info!(
                    seq_id,
                    ?origin_control,
                    has_text = text.is_some(),
                    keys_len = keys.len(),
                    "builtin: keyboard_input"
                );
                Command::perform(keyboard_input_async(text, keys), move |res| {
                    Message::ActionSeqStepDone { seq_id, res }
                })
            }
            BuiltinAction::PlaySound { path } => {
                tracing::info!(seq_id, ?origin_control, %path, "builtin: play_sound");
                Command::perform(play_sound_async(path), move |res| {
                Message::ActionSeqStepDone { seq_id, res }
                })
            }
            BuiltinAction::SwitchProfile { mode } => {
                tracing::info!(seq_id, ?origin_control, mode = ?mode, "builtin: switch_profile");
                let target = match mode {
                    actions::SwitchProfileMode::To { profile_id } => Some(ProfileId(profile_id)),
                    actions::SwitchProfileMode::Next => {
                        let current = self.selected_profile;
                        self.profiles
                            .iter()
                            .position(|p| Some(p.id) == current)
                            .map(|i| self.profiles[(i + 1) % self.profiles.len()].id)
                    }
                    actions::SwitchProfileMode::Prev => {
                        let current = self.selected_profile;
                        self.profiles.iter().position(|p| Some(p.id) == current).map(|i| {
                            let n = self.profiles.len();
                            self.profiles[(i + n - 1) % n].id
                        })
                    }
                };

                let Some(id) = target else {
                    return Command::perform(async { () }, move |_| Message::ActionSeqStepDone {
                        seq_id,
                        res: Err("[Action] No profiles available to switch.".to_string()),
                    });
                };

                self.selected_profile = Some(id);
                // Load profile and continue sequence.
                Command::batch([
                    Command::perform(load_profile_async(id), Message::ProfileLoaded),
                    Command::perform(async { () }, move |_| Message::ActionSeqContinue(seq_id)),
                ])
            }
            BuiltinAction::DeviceBrightness { mode } => {
                tracing::info!(seq_id, ?origin_control, mode = ?mode, "builtin: device_brightness");
                let Some(c) = &mut self.connected else {
                    return Command::perform(async { () }, move |_| Message::ActionSeqStepDone {
                        seq_id,
                        res: Err("[Action] Not connected to a device.".to_string()),
                    });
                };

                let new_val = match mode {
                    actions::BrightnessMode::Set { percent } => percent,
                    actions::BrightnessMode::Increase { delta } => c.brightness.saturating_add(delta),
                    actions::BrightnessMode::Decrease { delta } => c.brightness.saturating_sub(delta),
                }
                .clamp(0, 100);

                c.brightness = new_val;
                let controller = c.controller.clone();
                Command::perform(set_brightness_async(controller, new_val), move |res| {
                    Message::ActionSeqStepDone { seq_id, res }
                })
            }
            BuiltinAction::SystemMonitoring { .. } => {
                tracing::debug!(seq_id, ?origin_control, "builtin: system_monitoring (noop execute)");
                // Live display is handled via `binding_hint` + periodic sys refresh.
                // Executing it is a no-op for now.
                Command::perform(async { () }, move |_| Message::ActionSeqStepDone {
                    seq_id,
                    res: Ok(()),
                })
            }
        }
    }

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
        let builtin_actions = [
            BuiltinKindChoice::Macro,
            BuiltinKindChoice::IssueCommand,
            BuiltinKindChoice::KeyboardInput,
            BuiltinKindChoice::PlaySound,
            BuiltinKindChoice::SwitchProfile,
            BuiltinKindChoice::DeviceBrightness,
            BuiltinKindChoice::SystemMonitoring,
        ];

        let mut list = column![].spacing(8);
        let mut any = false;

        // Builtin actions (always available).
        for k in builtin_actions {
            let label = k.to_string();
            if !q.is_empty() && !label.to_ascii_lowercase().contains(&q) {
                continue;
            }
            any = true;
            list = list.push(
                mouse_area(
                    container(
                        row![
                            text(label).size(13),
                            horizontal_space(),
                            text("drag").size(12).style(color_text_muted()),
                        ]
                        .align_items(Alignment::Center),
                    )
                    .padding(8)
                    .style(panel()),
                )
                .on_press(Message::StartDragAction(DraggedAction::Builtin(k)))
                .interaction(iced::mouse::Interaction::Grab),
            );
        }

        // Plugin actions.
        for a in self.actions.iter() {
            if !q.is_empty() && !a.label.to_ascii_lowercase().contains(&q) {
                continue;
            }
            any = true;
            list = list.push(
                mouse_area(
                    container(
                        row![
                            text(&a.label).size(13),
                            horizontal_space(),
                            text("drag").size(12).style(color_text_muted()),
                        ]
                        .align_items(Alignment::Center),
                    )
                    .padding(8)
                    .style(panel()),
                )
                .on_press(Message::StartDragAction(DraggedAction::Plugin(a.clone())))
                .interaction(iced::mouse::Interaction::Grab),
            );
        }

        if !any {
            list = list.push(
                text(if self.actions.is_empty() {
                    "No actions match your search."
                } else {
                    "No actions match your search."
                })
                .size(13)
                .style(color_text_muted()),
            );
        }

        let drag_hint: Element<Message> = if let Some(d) = &self.drag.dragging {
            row![
                text(format!("Dragging: {}", d.label())).size(12),
                horizontal_space(),
                button(text("Cancel"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::CancelDragAction),
            ]
            .align_items(Alignment::Center)
            .into()
        } else {
            text("Drag an action onto a key to assign.").size(12).style(color_text_muted()).into()
        };

        let content = column![
            header,
            horizontal_rule(1),
            drag_hint,
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

        let selected = self.current_marketplace_source().cloned();

        let source_picker = pick_list(
            self.marketplace.sources.clone(),
            selected.clone(),
            Message::MarketplaceSourcePicked,
        );

        let url_row = row![
            text("Marketplace").size(12).style(color_text_muted()),
            source_picker,
            horizontal_space(),
            button(text("Refresh"))
                .style(iced::theme::Button::Secondary)
                .on_press(Message::MarketplaceRefresh),
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
        let matches = list_iter.collect::<Vec<_>>();
        let total_matches = matches.len();
        let page_count = (total_matches + MARKETPLACE_PAGE_SIZE - 1) / MARKETPLACE_PAGE_SIZE;
        let page = self.marketplace.page.min(page_count.saturating_sub(1));
        let start = page.saturating_mul(MARKETPLACE_PAGE_SIZE);
        let end = (start + MARKETPLACE_PAGE_SIZE).min(total_matches);
        let shown = if total_matches == 0 { 0 } else { end.saturating_sub(start) };

        let mut list = column![].spacing(10);
        for p in matches.into_iter().skip(start).take(MARKETPLACE_PAGE_SIZE) {
            let icon: Element<Message> = if let Some(src) = &selected {
                let key = format!("{}|{}", src.index_url, p.id);
                if let Some(handle) = self.marketplace.icon_cache.get(&key) {
                    image(handle.clone())
                        .width(Length::Fixed(32.0))
                        .height(Length::Fixed(32.0))
                        .into()
                } else {
                    container(text(""))
                        .width(Length::Fixed(32.0))
                        .height(Length::Fixed(32.0))
                        .style(panel())
                        .into()
                }
            } else {
                container(text(""))
                    .width(Length::Fixed(32.0))
                    .height(Length::Fixed(32.0))
                    .style(panel())
                    .into()
            };

            let mut body = column![text(&p.name).size(14)]
                .spacing(4)
                .push(text(p.id.clone()).size(12).style(color_text_muted()));

            if !p.version.is_empty() {
                body = body.push(text(format!("v{}", p.version)).size(12).style(color_text_muted()));
            }
            if !p.description.is_empty() {
                body = body.push(text(p.description.clone()).size(12).style(color_text_muted()));
            }

            let is_installed = self.plugins.iter().any(|ip| ip.manifest.id == p.id);
            let is_installing = self
                .marketplace
                .installing
                .as_deref()
                .is_some_and(|id| id == p.id);
            let is_selected = self
                .marketplace
                .selected
                .as_ref()
                .is_some_and(|sel| sel.id == p.id);
            let can_install_direct = selected
                .as_ref()
                .and_then(|src| resolve_marketplace_download_url(src, p))
                .is_some();
            let can_install_repo = p
                .repository
                .as_deref()
                .and_then(parse_github_owner_repo)
                .is_some();
            let can_install = can_install_direct || can_install_repo;

            let install_btn = if is_installed {
                button(text("Installed")).style(iced::theme::Button::Secondary)
            } else if is_installing {
                button(text("Installing…")).style(iced::theme::Button::Secondary)
            } else if can_install {
                button(text("Install"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::MarketplaceInstall(p.clone()))
            } else {
                button(text("No download")).style(iced::theme::Button::Secondary)
            };

            let select_area: Element<Message> = mouse_area(
                container(
                    row![icon, container(body).width(Length::Fill)]
                        .spacing(12)
                        .align_items(Alignment::Center),
                )
                .width(Length::Fill),
            )
            .on_press(Message::MarketplaceSelect(p.clone()))
            .into();

            list = list.push(
                container(
                    row![
                        container(select_area).width(Length::Fill),
                        install_btn
                    ]
                    .spacing(12)
                    .align_items(Alignment::Center),
                )
                .padding(10)
                .style(if is_selected { callout_card() } else { panel() }),
            );
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

        let footer: Element<Message> = if total_matches > 0 {
            row![
                text(format!(
                    "Page {} / {}  •  Showing {}–{} of {}",
                    page + 1,
                    page_count.max(1),
                    start + 1,
                    start + shown,
                    total_matches
                ))
                .style(color_text_muted()),
                horizontal_space(),
                button(text("Prev"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::MarketplacePrevPage),
                button(text("Next"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::MarketplaceNextPage),
            ]
            .align_items(Alignment::Center)
            .spacing(10)
            .into()
        } else {
            text("").into()
        };

        let content_left = column![
            header,
            h_divider(),
            url_row,
            search,
            status,
            scrollable(list).height(Length::Fill),
            footer
        ]
        .spacing(10)
        .height(Length::Fill);

        let details = self.view_marketplace_details_panel(selected.as_ref());

        row![
            container(content_left)
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
                .style(panel()),
            v_divider(),
            container(details)
                .padding(12)
                .width(Length::Fixed(420.0))
                .height(Length::Fill)
                .style(panel()),
        ]
        .spacing(12)
        .into()
    }

    fn view_marketplace_details_panel(
        &self,
        source: Option<&MarketplaceSource>,
    ) -> Element<'_, Message> {
        let header = row![
            text("Details").size(16),
            horizontal_space(),
        ]
        .align_items(Alignment::Center);

        let Some(p) = self.marketplace.selected.as_ref() else {
            return container(column![
                header,
                h_divider(),
                text("Click a plugin to view details.")
                    .size(13)
                    .style(color_text_muted()),
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        };

        let icon: Element<Message> = if let Some(src) = source {
            let key = format!("{}|{}", src.index_url, p.id);
            if let Some(handle) = self.marketplace.icon_cache.get(&key) {
                image(handle.clone())
                    .width(Length::Fixed(96.0))
                    .height(Length::Fixed(96.0))
                    .into()
            } else {
                container(text(""))
                    .width(Length::Fixed(96.0))
                    .height(Length::Fixed(96.0))
            .style(panel())
                    .into()
            }
        } else {
            container(text(""))
                .width(Length::Fixed(96.0))
                .height(Length::Fixed(96.0))
                .style(panel())
                .into()
        };

        let mut meta = column![
            row![icon, column![
                text(&p.name).size(18),
                text(p.id.clone()).size(12).style(color_text_muted()),
                if p.version.is_empty() {
                    text("").size(1)
                } else {
                    text(format!("v{}", p.version)).size(12).style(color_text_muted())
                },
            ]
            .spacing(4)]
            .spacing(12)
            .align_items(Alignment::Center),
        ]
        .spacing(10);

        if let Some(author) = &p.author {
            if !author.trim().is_empty() {
                meta = meta.push(text(format!("Author: {author}")).size(12).style(color_text_muted()));
            }
        }
        if let Some(home) = &p.homepage {
            if !home.trim().is_empty() {
                meta = meta.push(text(format!("Homepage: {home}")).size(12).style(color_text_muted()));
            }
        }
        if !p.description.trim().is_empty() {
            meta = meta.push(text(p.description.clone()).size(13));
        }

        // Extra details derived from repository (matches Rivul marketplace behavior).
        if let Some(d) = self.marketplace.details_cache.get(&p.id) {
            if let Some(repo) = d.repository.as_deref() {
                if !repo.trim().is_empty() {
                    meta = meta.push(text(format!("Repository: {repo}")).size(12).style(color_text_muted()));
                }
            }
            if let Some(dl) = d.total_downloads {
                meta = meta.push(
                    text(format!("Total downloads (GitHub releases): {dl}"))
                        .size(12)
                        .style(color_text_muted()),
                );
            }
            if let Some(url) = d.resolved_download_url.as_deref() {
                meta = meta.push(
                    text(format!("Resolved download: {url}"))
                        .size(12)
                        .style(color_text_muted()),
                );
            }
        }

        let readme_title = text("README").size(13).style(color_text_muted());
        let readme_body: Element<Message> = if let Some(d) = self.marketplace.details_cache.get(&p.id) {
            if let Some(md) = d.readme_md.as_deref() {
                container(self.render_markdown(md, source))
                    .padding(10)
                    .width(Length::Fill)
                    .style(panel())
                    .into()
            } else {
                text("Loading README…").size(12).style(color_text_muted()).into()
            }
        } else if p.repository.is_some() {
            text("Loading README…").size(12).style(color_text_muted()).into()
        } else {
            text("No repository/README available.").size(12).style(color_text_muted()).into()
        };

        let screenshots_title = text("Images").size(13).style(color_text_muted());
        let mut shots = column![].spacing(10);
        if let Some(src) = source {
            // Prefer images discovered from README; fall back to marketplace-provided screenshots.
            let mut urls: Vec<String> = vec![];
            if let Some(d) = self.marketplace.details_cache.get(&p.id) {
                urls.extend(d.image_urls.iter().cloned());
            }
            urls.extend(p.images.iter().cloned());

            let mut shown_any = false;
            for raw in urls.iter().take(12) {
                let Some(url) = resolve_marketplace_asset_url(src, raw) else {
                    continue;
                };
                shown_any = true;
                let key = format!("img|{}|{}|{}", src.index_url, p.id, url);
                if let Some(handle) = self.marketplace.image_cache.get(&key) {
                    shots = shots.push(image(handle.clone()).width(Length::Fill));
                } else if let Some(handle) = self.marketplace.svg_cache.get(&key) {
                    shots = shots.push(
                        iced::widget::svg(handle.clone())
                            .width(Length::Fill)
                            .height(Length::Shrink)
                        ,
                    );
                } else {
                    shots = shots.push(
                        container(text("Loading…").style(color_text_muted()))
                            .padding(10)
                            .width(Length::Fill)
                            .style(panel()),
                    );
                }
            }
            if !shown_any {
                shots = shots.push(text("No images found.").size(12).style(color_text_muted()));
            }
        }

        container(
            column![
                header,
                h_divider(),
                scrollable(
                    column![meta, h_divider(), readme_title, readme_body, h_divider(), screenshots_title, shots]
                        .spacing(12),
                )
                    .height(Length::Fill),
            ]
            .spacing(10),
        )
        .width(Length::Fill)
        .height(Length::Fill)
            .into()
    }

    fn view_preview_panel(&self) -> Element<'_, Message> {
        let selected = self
            .selected_control
            .map(|s| match s {
                SelectedControl::Key(k) => format!("Selected: Key {k}"),
                SelectedControl::Dial(d) => format!("Selected: Dial {d}"),
                SelectedControl::TouchStrip => "Selected: Touch strip".to_string(),
            })
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

        let body = match (self.connected.as_ref(), self.selected_control) {
            (None, _) => text("Connect a device to inspect controls.").into(),
            (Some(_), None) => text("Click a key/dial/touch strip in the preview to edit it.").into(),
            (Some(c), Some(sel)) => match sel {
                SelectedControl::Key(idx) => self.view_key_inspector(c, idx),
                SelectedControl::Dial(idx) => self.view_dial_inspector(idx),
                SelectedControl::TouchStrip => self.view_touch_strip_inspector(),
            },
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
        col = col.push(self.view_action_editor());

        col = col.push(horizontal_rule(1));
        col = col.push(text("Display (LCD)").size(14));
        col = col.push(self.view_display_editor());

        col = col.push(horizontal_rule(1));
        col = col.push(
            row![button(text("Save"))
                .on_press(Message::SaveProfile)
                .style(iced::theme::Button::Primary),]
            .spacing(8),
        );

        col.into()
    }

    fn view_dial_inspector(&self, idx: usize) -> Element<'_, Message> {
        let mut col = column![text(format!("Dial {idx}")).size(20)]
            .spacing(6);

        col = col.push(horizontal_rule(1));
        col = col.push(text("Label").size(14));
        col = col.push(text_input("Label", &self.edit_label).on_input(Message::LabelChanged));

        col = col.push(horizontal_rule(1));
        col = col.push(text("Binding target").size(14));
        col = col.push(pick_list(
            vec![BindingTarget::DialPress, BindingTarget::DialRotate],
            Some(self.selected_binding_target),
            Message::BindingTargetPicked,
        ));

        col = col.push(horizontal_rule(1));
        col = col.push(text("Action").size(14));
        col = col.push(self.view_action_editor());

        col = col.push(horizontal_rule(1));
        col = col.push(text("Display (LCD)").size(14));
        col = col.push(self.view_display_editor());

        col = col.push(horizontal_rule(1));
        col = col.push(
            row![button(text("Save"))
                .on_press(Message::SaveProfile)
                .style(iced::theme::Button::Primary),]
            .spacing(8),
        );

        col.into()
    }

    fn view_touch_strip_inspector(&self) -> Element<'_, Message> {
        let mut col = column![text("Touch strip").size(20)].spacing(6);

        col = col.push(horizontal_rule(1));
        col = col.push(text("Binding target").size(14));
        col = col.push(pick_list(
            vec![BindingTarget::TouchTap, BindingTarget::TouchDrag],
            Some(self.selected_binding_target),
            Message::BindingTargetPicked,
        ));

        col = col.push(horizontal_rule(1));
        col = col.push(text("Action").size(14));
        col = col.push(self.view_action_editor());

        col = col.push(horizontal_rule(1));
        col = col.push(text("Display (LCD)").size(14));
        col = col.push(self.view_display_editor());

        col = col.push(horizontal_rule(1));
        col = col.push(
            row![button(text("Save"))
                .on_press(Message::SaveProfile)
                .style(iced::theme::Button::Primary),]
            .spacing(8),
        );

        col.into()
    }

    fn selected_binding(&self) -> Option<&Option<ActionBinding>> {
        let p = self.profile.as_ref()?;
        let sel = self.selected_control?;
        match (sel, self.selected_binding_target) {
            (SelectedControl::Key(idx), BindingTarget::KeyPress) => Some(&p.keys.get(idx)?.action),
            (SelectedControl::Dial(idx), BindingTarget::DialPress) => Some(&p.dials.get(idx)?.press),
            (SelectedControl::Dial(idx), BindingTarget::DialRotate) => Some(&p.dials.get(idx)?.rotate),
            (SelectedControl::TouchStrip, BindingTarget::TouchTap) => Some(&p.touch_strip.tap),
            (SelectedControl::TouchStrip, BindingTarget::TouchDrag) => Some(&p.touch_strip.drag),
            _ => None,
        }
    }

    fn selected_binding_mut(&mut self) -> Option<&mut Option<ActionBinding>> {
        let p = self.profile.as_mut()?;
        let sel = self.selected_control?;
        match (sel, self.selected_binding_target) {
            (SelectedControl::Key(idx), BindingTarget::KeyPress) => Some(&mut p.keys.get_mut(idx)?.action),
            (SelectedControl::Dial(idx), BindingTarget::DialPress) => Some(&mut p.dials.get_mut(idx)?.press),
            (SelectedControl::Dial(idx), BindingTarget::DialRotate) => Some(&mut p.dials.get_mut(idx)?.rotate),
            (SelectedControl::TouchStrip, BindingTarget::TouchTap) => Some(&mut p.touch_strip.tap),
            (SelectedControl::TouchStrip, BindingTarget::TouchDrag) => Some(&mut p.touch_strip.drag),
            _ => None,
        }
    }

    fn set_selected_plugin_setting(&mut self, key: String, value: serde_json::Value) {
        let Some(slot) = self.selected_binding_mut() else {
            return;
        };
        let Some(ActionBinding::Plugin(binding)) = slot.as_mut() else {
            return;
        };

        if let Some(obj) = binding.settings.as_object_mut() {
            obj.insert(key, value);
            return;
        }

        let mut map = serde_json::Map::new();
        map.insert(key, value);
        binding.settings = serde_json::Value::Object(map);
    }

    fn view_display_editor(&self) -> Element<'_, Message> {
        column![
            text("Background RGB (r,g,b or empty)").size(12).style(color_text_muted()),
            text_input("", &self.edit_bg_rgb).on_input(Message::BgRgbChanged),
            text("Icon path (optional)").size(12).style(color_text_muted()),
            text_input("/path/to/icon.png", &self.edit_icon_path).on_input(Message::IconPathChanged),
            text("Text (optional)").size(12).style(color_text_muted()),
            text_input("", &self.edit_display_text).on_input(Message::DisplayTextChanged),
        ]
        .spacing(6)
        .into()
    }

    fn view_action_editor(&self) -> Element<'_, Message> {
        let Some(binding) = self.selected_binding() else {
            return text("Select a control to edit its action.").into();
        };

        let mode = match binding {
            None => ActionModeChoice::None,
            Some(ActionBinding::Plugin(_)) => ActionModeChoice::Plugin,
            Some(ActionBinding::Builtin(_)) => ActionModeChoice::Builtin,
        };

        let mode_picker = pick_list(
            vec![
                ActionModeChoice::None,
                ActionModeChoice::Plugin,
                ActionModeChoice::Builtin,
            ],
            Some(mode),
            Message::ActionModePicked,
        );

        let mut col = column![row![
            text("Mode").size(12).style(color_text_muted()),
            mode_picker
        ]
        .spacing(10)
        .align_items(Alignment::Center)]
        .spacing(8);

        match binding {
            None => {
                col = col.push(text("No action bound.").style(color_text_muted()));
            }
            Some(ActionBinding::Plugin(_)) => {
                let current = self.current_action_choice();
                col = col.push(pick_list(
                    self.actions.clone(),
                    current,
                    Message::ActionSelected,
                ));
                col = col.push(self.view_action_settings());
            }
            Some(ActionBinding::Builtin(b)) => {
                let current_kind = match b {
                    BuiltinAction::Macro { .. } => BuiltinKindChoice::Macro,
                    BuiltinAction::IssueCommand { .. } => BuiltinKindChoice::IssueCommand,
                    BuiltinAction::KeyboardInput { .. } => BuiltinKindChoice::KeyboardInput,
                    BuiltinAction::PlaySound { .. } => BuiltinKindChoice::PlaySound,
                    BuiltinAction::SwitchProfile { .. } => BuiltinKindChoice::SwitchProfile,
                    BuiltinAction::DeviceBrightness { .. } => BuiltinKindChoice::DeviceBrightness,
                    BuiltinAction::SystemMonitoring { .. } => BuiltinKindChoice::SystemMonitoring,
                };

                col = col.push(pick_list(
                    vec![
                        BuiltinKindChoice::Macro,
                        BuiltinKindChoice::IssueCommand,
                        BuiltinKindChoice::KeyboardInput,
                        BuiltinKindChoice::PlaySound,
                        BuiltinKindChoice::SwitchProfile,
                        BuiltinKindChoice::DeviceBrightness,
                        BuiltinKindChoice::SystemMonitoring,
                    ],
                    Some(current_kind),
                    Message::BuiltinKindPicked,
                ));

                col = col.push(self.view_builtin_settings(b));
            }
        }

        col.into()
    }

    fn view_builtin_settings(&self, b: &BuiltinAction) -> Element<'_, Message> {
        match b {
            BuiltinAction::Macro { steps } => self.view_macro_editor(steps),
            BuiltinAction::IssueCommand {
                command,
                cwd,
                timeout_ms,
            } => {
                let timeout = timeout_ms.map(|v| v.to_string()).unwrap_or_default();
                column![
                    text("Command").size(12).style(color_text_muted()),
                    text_input("bash command…", command).on_input(Message::BuiltinIssueCommandChanged),
                    text("Working dir (optional)").size(12).style(color_text_muted()),
                    text_input("", cwd.as_deref().unwrap_or("")).on_input(Message::BuiltinIssueCwdChanged),
                    text("Timeout ms (optional)").size(12).style(color_text_muted()),
                    text_input("", &timeout).on_input(Message::BuiltinIssueTimeoutChanged),
                    text("Runs via `bash -lc` (Linux MVP).").size(12).style(color_text_muted()),
                ]
                .spacing(6)
                .into()
            }
            BuiltinAction::KeyboardInput { text: input_text, keys } => {
                let keys_s = keys.join(" ");
                column![
                    text("Text (optional)").size(12).style(color_text_muted()),
                    text_input("", input_text.as_deref().unwrap_or(""))
                        .on_input(Message::BuiltinKeyboardTextChanged),
                    text("Keys (space-separated, optional)").size(12).style(color_text_muted()),
                    text_input("e.g. -k Return", &keys_s).on_input(Message::BuiltinKeyboardKeysChanged),
                    text("Linux MVP uses external tool: env `RIVERDECK_KEYBOARD_TOOL` (default: wtype).")
                        .size(12)
                        .style(color_text_muted()),
                ]
                .spacing(6)
                .into()
            }
            BuiltinAction::PlaySound { path } => column![
                text("Audio file path").size(12).style(color_text_muted()),
                text_input("/path/to/file.wav", path).on_input(Message::BuiltinPlaySoundPathChanged),
            ]
            .spacing(6)
            .into(),
            BuiltinAction::SwitchProfile { mode } => {
                let mut choices = vec![SwitchProfileChoice::Next, SwitchProfileChoice::Prev];
                for p in &self.profiles {
                    choices.push(SwitchProfileChoice::To(p.id));
                }

                let selected = match mode {
                    actions::SwitchProfileMode::Next => SwitchProfileChoice::Next,
                    actions::SwitchProfileMode::Prev => SwitchProfileChoice::Prev,
                    actions::SwitchProfileMode::To { profile_id } => {
                        SwitchProfileChoice::To(ProfileId(*profile_id))
                    }
                };

                column![
                    text("Mode").size(12).style(color_text_muted()),
                    pick_list(choices, Some(selected), Message::BuiltinSwitchProfilePicked),
                ]
                .spacing(6)
                .into()
            }
            BuiltinAction::DeviceBrightness { mode } => {
                let (m, v) = match mode {
                    actions::BrightnessMode::Set { percent } => (BrightnessModeChoice::Set, *percent as i32),
                    actions::BrightnessMode::Increase { delta } => {
                        (BrightnessModeChoice::Increase, *delta as i32)
                    }
                    actions::BrightnessMode::Decrease { delta } => {
                        (BrightnessModeChoice::Decrease, *delta as i32)
                    }
                };

                column![
                    text("Mode").size(12).style(color_text_muted()),
                    pick_list(
                        vec![
                            BrightnessModeChoice::Set,
                            BrightnessModeChoice::Increase,
                            BrightnessModeChoice::Decrease,
                        ],
                        Some(m),
                        Message::BuiltinBrightnessModePicked,
                    ),
                    slider(0..=100, v, Message::BuiltinBrightnessValueChanged),
                ]
                .spacing(6)
                .into()
            }
            BuiltinAction::SystemMonitoring { kind, .. } => {
                let k = match kind {
                    actions::MonitorKind::Cpu => MonitorKindChoice::Cpu,
                    actions::MonitorKind::Memory => MonitorKindChoice::Memory,
                    actions::MonitorKind::LoadAverage => MonitorKindChoice::LoadAverage,
                };

                column![
                    text("Metric").size(12).style(color_text_muted()),
                    pick_list(
                        vec![
                            MonitorKindChoice::Cpu,
                            MonitorKindChoice::Memory,
                            MonitorKindChoice::LoadAverage,
                        ],
                        Some(k),
                        Message::BuiltinMonitorKindPicked,
                    ),
                    text("Displayed live in preview (device rendering later).")
                        .size(12)
                        .style(color_text_muted()),
                ]
                .spacing(6)
                .into()
            }
        }
    }

    fn view_macro_editor(&self, steps: &[actions::MacroStep]) -> Element<'_, Message> {
        let mut col = column![
            row![
                text("Macro steps").size(12).style(color_text_muted()),
                horizontal_space(),
                button(text("+")).style(iced::theme::Button::Secondary).on_press(Message::MacroAddStep),
            ]
            .align_items(Alignment::Center)
            .spacing(8),
        ]
        .spacing(8);

        if steps.is_empty() {
            col = col.push(text("No steps yet.").style(color_text_muted()));
            return col.into();
        }

        for (i, s) in steps.iter().enumerate() {
            let kind = match s.action.as_ref() {
                ActionBinding::Plugin(_) => MacroStepKindChoice::PluginAction,
                ActionBinding::Builtin(BuiltinAction::IssueCommand { .. }) => {
                    MacroStepKindChoice::IssueCommand
                }
                _ => MacroStepKindChoice::PluginAction,
            };

            let delay = s.delay_ms.map(|d| d.to_string()).unwrap_or_default();
            let kind_picker = pick_list(
                vec![MacroStepKindChoice::PluginAction, MacroStepKindChoice::IssueCommand],
                Some(kind),
                move |k| Message::MacroStepKindPicked { idx: i, kind: k },
            );

            let controls = row![
                button(text("↑"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::MacroMoveStepUp(i)),
                button(text("↓"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::MacroMoveStepDown(i)),
                button(text("×"))
                    .style(iced::theme::Button::Secondary)
                    .on_press(Message::MacroRemoveStep(i)),
            ]
            .spacing(6);

            let delay_input = text_input("Delay ms", &delay).on_input(move |v| {
                Message::MacroStepDelayChanged {
                    idx: i,
                    value: v,
                }
            });

            let editor: Element<Message> = match s.action.as_ref() {
                ActionBinding::Plugin(p) => {
                    let current = self
                        .actions
                        .iter()
                        .find(|a| a.plugin_id == p.plugin_id && a.action_id == p.action_id)
                        .cloned();
                    pick_list(self.actions.clone(), current, move |c| {
                        Message::MacroStepPluginPicked { idx: i, choice: c }
                    })
                    .into()
                }
                ActionBinding::Builtin(BuiltinAction::IssueCommand { command, .. }) => {
                    text_input("bash command…", command).on_input(move |v| {
                        Message::MacroStepCommandChanged { idx: i, value: v }
                    })
                    .into()
                }
                _ => text("Unsupported step type (edit by changing kind).")
                    .style(color_text_muted())
                    .into(),
            };

            col = col.push(
                container(
                    column![
                        row![
                            controls,
                            horizontal_space(),
                            text(format!("Step {}", i + 1)).style(color_text_muted()),
                        ]
                        .align_items(Alignment::Center),
                        row![text("Kind").size(12).style(color_text_muted()), kind_picker]
                            .spacing(10)
                            .align_items(Alignment::Center),
                        row![text("Delay").size(12).style(color_text_muted()), delay_input]
                            .spacing(10)
                            .align_items(Alignment::Center),
                        editor,
                    ]
                    .spacing(6),
                )
                .padding(10)
                .style(panel()),
            );
        }

        col.into()
    }

    fn set_selected_action_mode(&mut self, mode: ActionModeChoice) {
        match mode {
            ActionModeChoice::None => {
                if let Some(slot) = self.selected_binding_mut() {
                    *slot = None;
                }
            }
            ActionModeChoice::Plugin => {
                // Keep existing plugin binding if present; otherwise choose first available action.
                if matches!(
                    self.selected_binding().and_then(|b| b.as_ref()),
                    Some(ActionBinding::Plugin(_))
                ) {
                    return;
                }
                let Some(first) = self.actions.first().cloned() else {
                    self.error = Some("No plugin actions available. Install a plugin first.".to_string());
                    if let Some(slot) = self.selected_binding_mut() {
                        *slot = None;
                    }
                    return;
                };
                let settings = default_settings_for_action(&self.plugins, &first);
                if let Some(slot) = self.selected_binding_mut() {
                    *slot = Some(ActionBinding::Plugin(PluginActionBinding {
                        plugin_id: first.plugin_id,
                        action_id: first.action_id,
                        settings,
                    }));
                }
            }
            ActionModeChoice::Builtin => {
                if matches!(
                    self.selected_binding().and_then(|b| b.as_ref()),
                    Some(ActionBinding::Builtin(_))
                ) {
                    return;
                }
                if let Some(slot) = self.selected_binding_mut() {
                    *slot = Some(ActionBinding::Builtin(BuiltinAction::IssueCommand {
                        command: String::new(),
                        cwd: None,
                        timeout_ms: None,
                    }));
                }
            }
        }
    }

    fn assign_dragged_action_to_key(&mut self, idx: usize, dragged: DraggedAction) {
        let Some(p) = &mut self.profile else {
            self.error = Some("[Action] No profile loaded.".to_string());
            return;
        };
        let Some(k) = p.keys.get_mut(idx) else {
            return;
        };

        match dragged {
            DraggedAction::Plugin(choice) => {
                let settings = default_settings_for_action(&self.plugins, &choice);
                k.action = Some(ActionBinding::Plugin(PluginActionBinding {
                    plugin_id: choice.plugin_id,
                    action_id: choice.action_id,
                    settings,
                }));
            }
            DraggedAction::Builtin(kind) => {
                k.action = Some(ActionBinding::Builtin(match kind {
                    BuiltinKindChoice::Macro => BuiltinAction::Macro { steps: vec![] },
                    BuiltinKindChoice::IssueCommand => BuiltinAction::IssueCommand {
                        command: String::new(),
                        cwd: None,
                        timeout_ms: None,
                    },
                    BuiltinKindChoice::KeyboardInput => BuiltinAction::KeyboardInput {
                        text: None,
                        keys: vec![],
                    },
                    BuiltinKindChoice::PlaySound => BuiltinAction::PlaySound {
                        path: String::new(),
                    },
                    BuiltinKindChoice::SwitchProfile => BuiltinAction::SwitchProfile {
                        mode: actions::SwitchProfileMode::Next,
                    },
                    BuiltinKindChoice::DeviceBrightness => BuiltinAction::DeviceBrightness {
                        mode: actions::BrightnessMode::Set { percent: 30 },
                    },
                    BuiltinKindChoice::SystemMonitoring => BuiltinAction::SystemMonitoring {
                        kind: actions::MonitorKind::Cpu,
                        refresh_ms: Some(500),
                    },
                }));
            }
        }
    }

    fn set_selected_builtin_kind(&mut self, kind: BuiltinKindChoice) {
        let Some(slot) = self.selected_binding_mut() else {
            return;
        };
        *slot = Some(ActionBinding::Builtin(match kind {
            BuiltinKindChoice::Macro => BuiltinAction::Macro { steps: vec![] },
            BuiltinKindChoice::IssueCommand => BuiltinAction::IssueCommand {
                command: String::new(),
                cwd: None,
                timeout_ms: None,
            },
            BuiltinKindChoice::KeyboardInput => BuiltinAction::KeyboardInput {
                text: None,
                keys: vec![],
            },
            BuiltinKindChoice::PlaySound => BuiltinAction::PlaySound {
                path: String::new(),
            },
            BuiltinKindChoice::SwitchProfile => BuiltinAction::SwitchProfile {
                mode: actions::SwitchProfileMode::Next,
            },
            BuiltinKindChoice::DeviceBrightness => BuiltinAction::DeviceBrightness {
                mode: actions::BrightnessMode::Set { percent: 30 },
            },
            BuiltinKindChoice::SystemMonitoring => BuiltinAction::SystemMonitoring {
                kind: actions::MonitorKind::Cpu,
                refresh_ms: Some(500),
            },
        }));
    }

    fn update_selected_builtin(&mut self, f: impl FnOnce(&mut BuiltinAction)) {
        let Some(slot) = self.selected_binding_mut() else {
            return;
        };
        let Some(ActionBinding::Builtin(b)) = slot.as_mut() else {
            return;
        };
        f(b)
    }

    fn selected_builtin_brightness_value(&self) -> Option<i32> {
        let b = self.selected_binding()?.as_ref()?;
        let ActionBinding::Builtin(BuiltinAction::DeviceBrightness { mode }) = b else {
            return None;
        };
        Some(match mode {
            actions::BrightnessMode::Set { percent } => *percent as i32,
            actions::BrightnessMode::Increase { delta } => *delta as i32,
            actions::BrightnessMode::Decrease { delta } => *delta as i32,
        })
    }

    fn macro_add_step(&mut self) {
        let default_plugin = self.actions.first().cloned();
        let default_settings = default_plugin
            .as_ref()
            .map(|c| default_settings_for_action(&self.plugins, c));
        self.update_selected_builtin(|b| {
            let BuiltinAction::Macro { steps } = b else {
                return;
            };

            let action = if let Some(first) = default_plugin {
                ActionBinding::Plugin(PluginActionBinding {
                    plugin_id: first.plugin_id,
                    action_id: first.action_id,
                    settings: default_settings.unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new())),
                })
            } else {
                ActionBinding::Builtin(BuiltinAction::IssueCommand {
                    command: String::new(),
                    cwd: None,
                    timeout_ms: None,
                })
            };

            steps.push(actions::MacroStep {
                action: Box::new(action),
                delay_ms: None,
            });
        });
    }

    fn macro_remove_step(&mut self, idx: usize) {
        self.update_selected_builtin(|b| {
            let BuiltinAction::Macro { steps } = b else {
                return;
            };
            if idx < steps.len() {
                steps.remove(idx);
            }
        });
    }

    fn macro_move_step(&mut self, idx: usize, up: bool) {
        self.update_selected_builtin(|b| {
            let BuiltinAction::Macro { steps } = b else {
                return;
            };
            if steps.is_empty() || idx >= steps.len() {
                return;
            }
            if up {
                if idx == 0 {
                    return;
                }
                steps.swap(idx, idx - 1);
            } else {
                if idx + 1 >= steps.len() {
                    return;
                }
                steps.swap(idx, idx + 1);
            }
        });
    }

    fn macro_set_step_kind(&mut self, idx: usize, kind: MacroStepKindChoice) {
        let default_plugin = self.actions.first().cloned();
        let default_settings = default_plugin
            .as_ref()
            .map(|c| default_settings_for_action(&self.plugins, c));
        self.update_selected_builtin(|b| {
            let BuiltinAction::Macro { steps } = b else {
                return;
            };
            let Some(step) = steps.get_mut(idx) else {
                return;
            };
            step.action = Box::new(match kind {
                MacroStepKindChoice::PluginAction => {
                    if let Some(first) = default_plugin {
                        ActionBinding::Plugin(PluginActionBinding {
                            plugin_id: first.plugin_id,
                            action_id: first.action_id,
                            settings: default_settings.unwrap_or_else(|| {
                                serde_json::Value::Object(serde_json::Map::new())
                            }),
                        })
                    } else {
                        ActionBinding::Builtin(BuiltinAction::IssueCommand {
                            command: String::new(),
                            cwd: None,
                            timeout_ms: None,
                        })
                    }
                }
                MacroStepKindChoice::IssueCommand => ActionBinding::Builtin(BuiltinAction::IssueCommand {
                    command: String::new(),
                    cwd: None,
                    timeout_ms: None,
                }),
            });
        });
    }

    fn macro_set_step_delay(&mut self, idx: usize, value: String) {
        self.update_selected_builtin(|b| {
            let BuiltinAction::Macro { steps } = b else {
                return;
            };
            let Some(step) = steps.get_mut(idx) else {
                return;
            };
            let s = value.trim();
            step.delay_ms = if s.is_empty() { None } else { s.parse::<u64>().ok() };
        });
    }

    fn macro_set_step_plugin(&mut self, idx: usize, choice: ActionChoice) {
        let settings = default_settings_for_action(&self.plugins, &choice);
        self.update_selected_builtin(|b| {
            let BuiltinAction::Macro { steps } = b else {
                return;
            };
            let Some(step) = steps.get_mut(idx) else {
                return;
            };
            step.action = Box::new(ActionBinding::Plugin(PluginActionBinding {
                plugin_id: choice.plugin_id,
                action_id: choice.action_id,
                settings,
            }));
        });
    }

    fn macro_set_step_command(&mut self, idx: usize, value: String) {
        self.update_selected_builtin(|b| {
            let BuiltinAction::Macro { steps } = b else {
                return;
            };
            let Some(step) = steps.get_mut(idx) else {
                return;
            };
            step.action = Box::new(ActionBinding::Builtin(BuiltinAction::IssueCommand {
                command: value,
                cwd: None,
                timeout_ms: None,
            }));
        });
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
            let strip_h = 52.0;
            let dial_size = 56.0;
            let strip_selected = self.selected_control == Some(SelectedControl::TouchStrip);

            let strip = container(text("Touch strip").size(12).style(color_text_muted()))
            .width(Length::Fill)
            .height(Length::Fixed(strip_h))
            .center_x()
            .center_y()
            .style(iced::theme::Container::Custom(Box::new(move |theme: &Theme| {
                let p = theme.extended_palette();
                iced::widget::container::Appearance {
                    background: Some(Background::Color(p.background.base.color)),
                    text_color: Some(p.background.base.text),
                    border: Border {
                        // Square strip (sleek / hardware-like)
                        radius: 0.0.into(),
                        width: 1.0,
                        color: if strip_selected {
                            Color::from_rgba8(255, 255, 255, 0.22)
                        } else {
                            Color::from_rgba8(255, 255, 255, 0.08)
                        },
                    },
                    shadow: Shadow::default(),
                }
            })));

            let dial = |idx: usize| {
                let dial_selected = self.selected_control == Some(SelectedControl::Dial(idx));
                let label = self
                    .profile
                    .as_ref()
                    .and_then(|p| p.dials.get(idx))
                    .map(|d| d.label.clone())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| format!("Dial {}", idx + 1));
                container(text(label).size(11).style(color_text_muted()))
                    .width(Length::Fixed(dial_size))
                    .height(Length::Fixed(dial_size))
                    .center_x()
                    .center_y()
                    .style(iced::theme::Container::Custom(Box::new(move |theme: &Theme| {
                        let p = theme.extended_palette();
                        iced::widget::container::Appearance {
                            background: Some(Background::Color(p.background.base.color)),
                            text_color: Some(p.background.base.text),
                            border: Border {
                                // Fully round knobs.
                                radius: 999.0.into(),
                                width: 1.0,
                                color: if dial_selected {
                                    Color::from_rgba8(255, 255, 255, 0.22)
                                } else {
                                    Color::from_rgba8(255, 255, 255, 0.08)
                                },
                            },
                            shadow: Shadow::default(),
                        }
                    })))
            };

            let strip = mouse_area(strip).on_press(Message::SelectControl(SelectedControl::TouchStrip));
            let dials = container(
                row![
                    mouse_area(dial(0)).on_press(Message::SelectControl(SelectedControl::Dial(0))),
                    mouse_area(dial(1)).on_press(Message::SelectControl(SelectedControl::Dial(1))),
                    mouse_area(dial(2)).on_press(Message::SelectControl(SelectedControl::Dial(2))),
                    mouse_area(dial(3)).on_press(Message::SelectControl(SelectedControl::Dial(3))),
                ]
                .spacing(12)
                .align_items(Alignment::Center),
            )
            .width(Length::Fill)
            .center_x();

            column![grid, strip, dials].spacing(gap as u16)
        } else {
            grid
        };

        let width = (cols as f32 * key) + ((cols - 1) as f32 * gap) + 2.0 * pad;
        let base_height = (rows as f32 * key) + ((rows - 1) as f32 * gap) + 2.0 * pad;
        let extra = if key_count == 8 {
            // Touch strip + dials + gaps between sections
            52.0 + 56.0 + (gap * 2.0)
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
        let is_selected = self.selected_control == Some(SelectedControl::Key(idx));
        let is_drop_hover = self.drag.dragging.is_some() && self.drag.over_key == Some(idx);
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
            .and_then(|a| self.binding_hint(a));

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

        let key_btn = button(container(content).center_x().center_y())
            .width(Length::Fixed(key))
            .height(Length::Fixed(key))
            .padding(0)
            .on_press(Message::SelectControl(SelectedControl::Key(idx)))
            .style(iced::theme::Button::custom(DeckKeyStyle {
                pressed: is_pressed,
                selected: is_selected,
                drop_hover: is_drop_hover,
            }))
            ;

        // Drop target surface: release mouse over a key to assign the currently dragged action.
        mouse_area(key_btn)
            .on_enter(Message::DragOverKey(Some(idx)))
            .on_exit(Message::DragOverKey(None))
            .on_release(Message::DropOnKey(idx))
            .into()
    }

    fn action_label(&self, plugin_id: &str, action_id: &str) -> Option<String> {
        let (_plugin, action) = find_action_def_by_ids(&self.plugins, plugin_id, action_id)?;
        Some(action.name.clone())
    }

    fn binding_hint(&self, binding: &ActionBinding) -> Option<String> {
        match binding {
            ActionBinding::Plugin(p) => self.action_label(&p.plugin_id, &p.action_id),
            ActionBinding::Builtin(b) => Some(match b {
                actions::BuiltinAction::Macro { .. } => "Macro".to_string(),
                actions::BuiltinAction::IssueCommand { .. } => "Issue Command".to_string(),
                actions::BuiltinAction::KeyboardInput { .. } => "Keyboard Input".to_string(),
                actions::BuiltinAction::PlaySound { .. } => "Play Sound".to_string(),
                actions::BuiltinAction::SwitchProfile { .. } => "Switch Profile".to_string(),
                actions::BuiltinAction::DeviceBrightness { .. } => "Device Brightness".to_string(),
                actions::BuiltinAction::SystemMonitoring { kind, .. } => match kind {
                    actions::MonitorKind::Cpu => format!("CPU {:.0}%", self.sys_snapshot.cpu_percent),
                    actions::MonitorKind::Memory => {
                        let used = self.sys_snapshot.mem_used as f32 / (1024.0 * 1024.0);
                        let total = self.sys_snapshot.mem_total as f32 / (1024.0 * 1024.0);
                        format!("Mem {:.0}/{:.0}MB", used, total)
                    }
                    actions::MonitorKind::LoadAverage => {
                        let (a, b, c) = self.sys_snapshot.load;
                        format!("Load {:.2} {:.2} {:.2}", a, b, c)
                    }
                },
            }),
        }
    }

    fn refresh_system_snapshot(&mut self) {
        // Lightweight periodic refresh for UI display (no device rendering yet).
        // Keep it conservative to avoid adding overhead.
        if self.sys_last_refresh.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.sys_last_refresh = Instant::now();

        use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind};
        let refresh = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything());
        self.sys.refresh_specifics(refresh);

        // CPU usage: average across all CPUs.
        let cpus = self.sys.cpus();
        let cpu_percent = if cpus.is_empty() {
            0.0
        } else {
            cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / (cpus.len() as f32)
        };

        let mem_total = self.sys.total_memory();
        let mem_used = self.sys.used_memory();

        let load = sysinfo::System::load_average();
        self.sys_snapshot = SystemSnapshot {
            cpu_percent,
            mem_used,
            mem_total,
            load: (load.one, load.five, load.fifteen),
        };
    }

    fn current_action_choice(&self) -> Option<ActionChoice> {
        let b = match self.selected_binding()?.as_ref()? {
            ActionBinding::Plugin(p) => p,
            ActionBinding::Builtin(_) => return None,
        };
        self.actions
            .iter()
            .find(|a| a.plugin_id == b.plugin_id && a.action_id == b.action_id)
            .cloned()
    }

    fn view_action_settings(&self) -> Element<'_, Message> {
        let Some(binding) = self.selected_binding().and_then(|b| b.as_ref()) else {
            return text("No action bound.").into();
        };

        let ActionBinding::Plugin(binding) = binding else {
            // Builtin actions have their own editor (added in `ui-config` todo).
            return text("Builtin action settings are not shown here yet.").into();
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

    fn render_markdown(&self, md: &str, source: Option<&MarketplaceSource>) -> Element<'_, Message> {
        use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_FOOTNOTES);
        opts.insert(Options::ENABLE_TASKLISTS);

        let parser = Parser::new_ext(md, opts);

        let mut elems: Vec<Element<Message>> = vec![];

        let mut cur = String::new();
        let mut cur_link: Option<String> = None;
        let mut heading_level: Option<u8> = None;
        let mut in_code_block = false;
        let mut code_buf = String::new();

        fn flush_paragraph(
            elems: &mut Vec<Element<Message>>,
            text_buf: &mut String,
            link: &mut Option<String>,
            heading_level: &mut Option<u8>,
        ) {
            let s = text_buf.trim();
            if s.is_empty() {
                *text_buf = String::new();
                *link = None;
                *heading_level = None;
                return;
            }

            // Heading
            if let Some(level) = *heading_level {
                let size = match level {
                    1 => 18,
                    2 => 16,
                    3 => 15,
                    _ => 14,
                };
                elems.push(text(s.to_string()).size(size).into());
                *text_buf = String::new();
                *link = None;
                *heading_level = None;
                return;
            }

            // Link-only line (basic)
            if let Some(url) = link.take() {
                elems.push(
                    button(text(s.to_string()))
                        .style(iced::theme::Button::Secondary)
                        .on_press(Message::OpenUrl(url))
                        .into(),
                );
                *text_buf = String::new();
                *heading_level = None;
                return;
            }

            elems.push(text(s.to_string()).size(12).into());
            *text_buf = String::new();
            *heading_level = None;
        }

        for ev in parser {
            match ev {
                Event::Start(Tag::Heading { level, .. }) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                    heading_level = Some(level as u8);
                }
                Event::End(TagEnd::Heading { .. }) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                }
                Event::Start(Tag::Paragraph) => {}
                Event::End(TagEnd::Paragraph) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                }
                Event::Start(Tag::Link { dest_url, .. }) => {
                    cur_link = Some(dest_url.to_string());
                }
                Event::End(TagEnd::Link) => {
                    // keep link; flush on paragraph end
                }
                Event::Start(Tag::CodeBlock(_)) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                    in_code_block = true;
                    code_buf.clear();
                }
                Event::End(TagEnd::CodeBlock) => {
                    in_code_block = false;
                    let s = code_buf.trim_end().to_string();
                    if !s.is_empty() {
                        elems.push(
                            container(text(s).size(12).style(color_text_muted()))
                                .padding(10)
                                .style(panel())
                                .into(),
                        );
                    }
                    code_buf.clear();
                }
                Event::Text(t) => {
                    if in_code_block {
                        code_buf.push_str(&t);
                    } else {
                        cur.push_str(&t);
                    }
                }
                Event::Code(t) => {
                    if in_code_block {
                        code_buf.push_str(&t);
                    } else {
                        cur.push('`');
                        cur.push_str(&t);
                        cur.push('`');
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if in_code_block {
                        code_buf.push('\n');
                    } else {
                        cur.push('\n');
                    }
                }
                Event::Start(Tag::List(_)) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                }
                Event::End(TagEnd::List(_)) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                }
                Event::Start(Tag::Item) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                    cur.push_str("• ");
                }
                Event::End(TagEnd::Item) => {
                    flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);
                }
                Event::Start(Tag::Image { dest_url, .. }) => {
                    // Try to show image if it's a renderable format; otherwise provide a link button.
                    let raw = dest_url.to_string();
                    if is_renderable_image_url(&raw) {
                        if let Some(src) = source {
                            if let Some(url) = resolve_marketplace_asset_url(src, &raw) {
                                elems.push(
                                    button(text(format!("Open image: {}", truncate(&url, 60))))
                                        .style(iced::theme::Button::Secondary)
                                        .on_press(Message::OpenUrl(url))
                                        .into(),
                                );
                            }
                        } else {
                            elems.push(
                                button(text(format!("Open image: {}", truncate(&raw, 60))))
                                    .style(iced::theme::Button::Secondary)
                                    .on_press(Message::OpenUrl(raw))
                                    .into(),
                            );
                        }
                    } else {
                        elems.push(
                            button(text(format!("Open image (unsupported): {}", truncate(&raw, 60))))
                                .style(iced::theme::Button::Secondary)
                                .on_press(Message::OpenUrl(raw))
                                .into(),
                        );
                    }
                }
                _ => {}
            }
        }

        flush_paragraph(&mut elems, &mut cur, &mut cur_link, &mut heading_level);

        let out = elems
            .into_iter()
            .fold(column![].spacing(6), |col, el| col.push(el));

        // Important: do NOT wrap markdown in its own scrollable, because the details panel already
        // has a scroll container. Nested scrollables prevent mouse-wheel scrolling from working.
        out.into()
    }
}

#[derive(Debug, Clone, Copy)]
struct DeckKeyStyle {
    pressed: bool,
    selected: bool,
    drop_hover: bool,
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

        let border_color = if self.drop_hover {
            palette.primary.base.color
        } else if self.selected {
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

fn callout_card() -> iced::theme::Container {
    iced::theme::Container::Custom(Box::new(|theme: &Theme| {
        let p = theme.extended_palette();
        let mut shade = p.background.strong.color;
        shade.a = 0.96;
        iced::widget::container::Appearance {
            background: Some(Background::Color(shade)),
            text_color: Some(p.background.base.text),
            border: Border {
                radius: 10.0.into(),
                width: 1.0,
                color: Color::from_rgba8(255, 255, 255, 0.12),
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.18),
                offset: iced::Vector::new(0.0, 6.0),
                blur_radius: 14.0,
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
        icon_base_url: Some("https://openactionapi.github.io/plugins/icons/".to_string()),
    }];

    // Optional user-provided list:
    // OPENACTION_MARKETPLACES="Name|https://...;Other|https://..."
    // Optional third field:
    // OPENACTION_MARKETPLACES="Name|https://...|https://icons/;Other|https://..."
    if let Ok(raw) = std::env::var("OPENACTION_MARKETPLACES") {
        for part in raw.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let mut it = part.split('|');
            let name = it.next().unwrap_or("").trim();
            let url = it.next().unwrap_or("").trim();
            let icon_base_url = it.next().map(|s| s.trim()).filter(|s| !s.is_empty());
            if name.is_empty() || url.is_empty() {
                continue;
            }
            let src = MarketplaceSource {
                name: name.to_string(),
                index_url: url.to_string(),
                icon_base_url: icon_base_url.map(|s| s.to_string()),
            };
            if !out.contains(&src) {
                out.push(src);
            }
        }
    }

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

async fn install_marketplace_async(url: String, expected_id: String) -> Result<(), String> {
    openaction::installer::install_from_url(&url, Some(&expected_id))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn install_marketplace_from_repo_async(
    repo_url: String,
    expected_id: String,
) -> Result<(), String> {
    let url = resolve_github_release_asset_url_async(&repo_url).await?;
    install_marketplace_async(url, expected_id).await
}

async fn fetch_marketplace_details_async(plugin: MarketplacePlugin) -> Result<MarketplaceDetails, String> {
    let mut out = MarketplaceDetails {
        repository: plugin.repository.clone(),
        ..MarketplaceDetails::default()
    };

    let Some(repo_url) = plugin.repository.as_deref() else {
        return Ok(out);
    };
    let Some((owner, repo)) = parse_github_owner_repo(repo_url) else {
        return Ok(out);
    };

    // README + images
    if let Ok((branch, md)) = fetch_github_readme_md_async(&owner, &repo).await {
        out.readme_md = Some(md.clone());
        for raw in extract_readme_image_urls(&md).into_iter().take(24) {
            if !is_renderable_image_url(&raw) {
                continue;
            }
            if let Some(abs) = resolve_github_readme_asset_url(&owner, &repo, &branch, &raw) {
                if !out.image_urls.contains(&abs) {
                    out.image_urls.push(abs);
                }
            }
        }
    }

    // Releases: total download counts + best asset URL for install.
    if let Ok((count, asset)) = fetch_github_releases_info_async(&owner, &repo).await {
        out.total_downloads = Some(count);
        out.resolved_download_url = asset;
    }

    Ok(out)
}

async fn fetch_marketplace_async(url: String) -> Result<Vec<MarketplacePlugin>, String> {
    openaction::marketplace::fetch_plugins(&url)
        .await
        .map_err(|e| e.to_string())
}

async fn fetch_icon_async(url: String) -> Result<Vec<u8>, String> {
    openaction::marketplace::fetch_bytes(&url)
        .await
        .map_err(|e| e.to_string())
}

async fn sleep_ms_async(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await
}

async fn issue_command_async(
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<(), String> {
    use tokio::process::Command;

    let mut cmd = Command::new("bash");
    cmd.arg("-lc").arg(command);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    let fut = async move {
        let status = cmd.status().await.map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("Command exited with status: {status}"))
        }
    };

    if let Some(ms) = timeout_ms {
        tokio::time::timeout(Duration::from_millis(ms), fut)
            .await
            .map_err(|_| "Command timed out".to_string())?
    } else {
        fut.await
    }
}

async fn keyboard_input_async(text: Option<String>, keys: Vec<String>) -> Result<(), String> {
    // Linux MVP: delegate to an external tool.
    // Configure with RIVERDECK_KEYBOARD_TOOL, default: wtype
    let tool = std::env::var("RIVERDECK_KEYBOARD_TOOL").unwrap_or_else(|_| "wtype".to_string());

    if let Some(text) = text {
        let cmd = format!("{tool} {}", shell_escape(&text));
        return issue_command_async(cmd, None, Some(5_000)).await;
    }

    if keys.is_empty() {
        return Ok(());
    }

    // Treat `keys` as tool arguments (e.g. for wtype: `-k Return`).
    let args = keys
        .into_iter()
        .map(|k| shell_escape(&k))
        .collect::<Vec<_>>()
        .join(" ");
    let cmd = format!("{tool} {args}");
    issue_command_async(cmd, None, Some(5_000)).await
}

async fn open_url_async(url: String) -> Result<(), String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    let cmd = format!("start {}", shell_escape(&url));
    #[cfg(target_os = "macos")]
    let cmd = format!("open {}", shell_escape(&url));
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = format!("xdg-open {}", shell_escape(&url));

    issue_command_async(cmd, None, Some(5_000)).await
}

fn shell_escape(s: &str) -> String {
    // Minimal, safe shell escaping for bash -lc.
    // Wrap in single quotes and escape internal single quotes.
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

async fn play_sound_async(path: String) -> Result<(), String> {
    // Use a blocking thread because rodio decoding + playback is blocking.
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        use std::fs::File;
        use std::io::BufReader;

        let (_stream, handle) = rodio::OutputStream::try_default().map_err(|e| e.to_string())?;
        let sink = rodio::Sink::try_new(&handle).map_err(|e| e.to_string())?;

        let f = File::open(&path).map_err(|e| e.to_string())?;
        let src = rodio::Decoder::new(BufReader::new(f)).map_err(|e| e.to_string())?;
        sink.append(src);
        sink.sleep_until_end();
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn marketplace_icon_url(source: &MarketplaceSource, plugin: &MarketplacePlugin) -> Option<String> {
    if let Some(url) = &plugin.icon_url {
        if !url.trim().is_empty() {
            return Some(url.clone());
        }
    }
    let base = source.icon_base_url.as_ref()?.trim();
    if base.is_empty() {
        return None;
    }
    let mut out = base.to_string();
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(&plugin.id);
    out.push_str(".png");
    Some(out)
}

fn resolve_marketplace_download_url(
    source: &MarketplaceSource,
    plugin: &MarketplacePlugin,
) -> Option<String> {
    let raw = plugin.download_url.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }

    // Absolute.
    if reqwest::Url::parse(raw).is_ok() {
        return Some(raw.to_string());
    }

    // Relative: resolve against the marketplace index URL.
    let base = reqwest::Url::parse(source.index_url.trim()).ok()?;
    base.join(raw).ok().map(|u| u.to_string())
}

fn resolve_marketplace_asset_url(source: &MarketplaceSource, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // Absolute.
    if reqwest::Url::parse(raw).is_ok() {
        return Some(raw.to_string());
    }

    // Relative: resolve against the marketplace index URL.
    let base = reqwest::Url::parse(source.index_url.trim()).ok()?;
    base.join(raw).ok().map(|u| u.to_string())
}

fn parse_github_owner_repo(repo_url: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(repo_url.trim()).ok()?;
    if url.host_str()? != "github.com" {
        return None;
    }
    let mut segs = url.path_segments()?;
    let owner = segs.next()?.trim().to_string();
    let repo = segs.next()?.trim().trim_end_matches(".git").to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

async fn fetch_github_readme_md_async(owner: &str, repo: &str) -> Result<(String, String), String> {
    let candidates = [
        ("main", "README.md"),
        ("main", "readme.md"),
        ("master", "README.md"),
        ("master", "readme.md"),
    ];
    for (branch, file) in candidates {
        let url = format!(
            "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{file}"
        );
        match openaction::marketplace::fetch_bytes(&url).await {
            Ok(bytes) => {
                let md = String::from_utf8(bytes).map_err(|_| "README is not utf-8".to_string())?;
                return Ok((branch.to_string(), md));
            }
            Err(_) => continue,
        }
    }
    Err("README not found (tried main/master README.md/readme.md)".to_string())
}

fn extract_readme_image_urls(md: &str) -> Vec<String> {
    let mut out = Vec::new();

    // Markdown: ![alt](url)
    let bytes = md.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b'!' && bytes[i + 1] == b'[' {
            // find "]("
            if let Some(close_bracket) = md[i + 2..].find("](") {
                let start = i + 2 + close_bracket + 2; // points at '('
                // actual url begins after '('
                let url_start = start + 1;
                if url_start >= md.len() {
                    break;
                }
                if let Some(end_rel) = md[url_start..].find(')') {
                    let url_raw = md[url_start..url_start + end_rel].trim();
                    // strip optional title: "url \"title\""
                    let url_only = url_raw
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .trim_matches('<')
                        .trim_matches('>');
                    if !url_only.is_empty() {
                        out.push(url_only.to_string());
                    }
                    i = url_start + end_rel + 1;
                    continue;
                }
            }
        }
        i += 1;
    }

    // HTML: <img src="...">
    let mut rest = md;
    while let Some(pos) = rest.find("src=\"") {
        rest = &rest[pos + 5..];
        if let Some(end) = rest.find('"') {
            let url = rest[..end].trim();
            if !url.is_empty() {
                out.push(url.to_string());
            }
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }

    // De-dupe, preserve order
    let mut uniq = Vec::new();
    for u in out {
        if !uniq.contains(&u) {
            uniq.push(u);
        }
    }
    uniq
}

fn is_renderable_image_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    if u.is_empty() {
        return false;
    }
    // SVG is now supported via iced's `svg` widget; raster formats supported via `image` features.
    u.ends_with(".svg")
        || u.contains(".svg?")
        || u.ends_with(".png")
        || u.contains(".png?")
        || u.ends_with(".jpg")
        || u.contains(".jpg?")
        || u.ends_with(".jpeg")
        || u.contains(".jpeg?")
}

fn resolve_github_readme_asset_url(owner: &str, repo: &str, branch: &str, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // Already absolute.
    if reqwest::Url::parse(raw).is_ok() {
        // Convert GitHub blob URLs to raw URLs.
        if raw.starts_with("https://github.com/") && raw.contains("/blob/") {
            // https://github.com/{owner}/{repo}/blob/{branch}/path -> raw.githubusercontent.com/{owner}/{repo}/{branch}/path
            let prefix = format!("https://github.com/{owner}/{repo}/blob/");
            if let Some(rest) = raw.strip_prefix(&prefix) {
                let mut it = rest.splitn(2, '/');
                let b = it.next().unwrap_or(branch);
                let path = it.next().unwrap_or("");
                return Some(format!(
                    "https://raw.githubusercontent.com/{owner}/{repo}/{b}/{path}"
                ));
            }
        }
        return Some(raw.to_string());
    }

    // Root-relative within repo.
    if raw.starts_with('/') {
        return Some(format!(
            "https://raw.githubusercontent.com/{owner}/{repo}/{branch}{}",
            raw
        ));
    }

    // Relative path.
    let rel = raw.trim_start_matches("./");
    Some(format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{rel}"
    ))
}

async fn fetch_github_releases_info_async(owner: &str, repo: &str) -> Result<(u64, Option<String>), String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases");
    let client = reqwest::Client::builder()
        .user_agent("RiverDeck-Redux/0.1 (Marketplace)")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let releases = v.as_array().ok_or_else(|| "unexpected GitHub API response".to_string())?;

    let mut total: u64 = 0;
    for rel in releases {
        if let Some(assets) = rel.get("assets").and_then(|a| a.as_array()) {
            for a in assets {
                if let Some(c) = a.get("download_count").and_then(|c| c.as_u64()) {
                    total += c;
                }
            }
        }
    }

    // Pick best installable asset from the latest release (first in list).
    let asset_url = releases
        .first()
        .and_then(|rel| rel.get("assets").and_then(|a| a.as_array()))
        .and_then(|assets| pick_best_github_asset_url(assets));

    Ok((total, asset_url))
}

fn pick_best_github_asset_url(assets: &[serde_json::Value]) -> Option<String> {
    // Gather candidates (archive/plugin-bundle assets only).
    let mut cands: Vec<(i32, String)> = vec![];
    for a in assets {
        let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("").to_ascii_lowercase();
        let url = a
            .get("browser_download_url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            continue;
        }
        // Common archive formats + known plugin bundle extensions.
        let is_supported = name.ends_with(".zip")
            || name.ends_with(".tar.gz")
            || name.ends_with(".tgz")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".opendeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin")
            || name.ends_with(".streamdeckplugin");
        if !is_supported {
            continue;
        }
        let mut score = 0;
        // Prefer platform matches.
        if cfg!(windows) {
            if name.contains("windows") || name.contains("win") {
                score += 50;
            }
        } else if cfg!(target_os = "macos") {
            if name.contains("mac") || name.contains("darwin") || name.contains("osx") {
                score += 50;
            }
        } else {
            if name.contains("linux") {
                score += 50;
            }
        }
        // Prefer common packaging formats slightly.
        if name.ends_with(".zip") {
            score += 10;
        }
        if name.ends_with(".streamdeckplugin") {
            score += 12;
        }
        cands.push((score, url));
    }
    cands.sort_by(|a, b| b.0.cmp(&a.0));
    cands.first().map(|(_, u)| u.clone())
}

async fn resolve_github_release_asset_url_async(repo_url: &str) -> Result<String, String> {
    let (owner, repo) =
        parse_github_owner_repo(repo_url).ok_or_else(|| "unsupported repository url".to_string())?;
    let (_count, asset) = fetch_github_releases_info_async(&owner, &repo).await?;
    asset.ok_or_else(|| "no downloadable release archive found on GitHub".to_string())
}

fn parse_bg_rgb(raw: &str) -> Option<[u8; 3]> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].parse::<u8>().ok()?;
    let g = parts[1].parse::<u8>().ok()?;
    let b = parts[2].parse::<u8>().ok()?;
    Some([r, g, b])
}

async fn invoke_action_async(
    plugin: InstalledPlugin,
    action_id: String,
    control: InvocationControl,
    event: InvocationEvent,
    settings: serde_json::Value,
) -> Result<(), String> {
    let rt = ActionRuntime::new();
    rt.invoke(
        &plugin,
        &action_id,
        control,
        event,
        settings,
    )
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

async fn apply_displays_async(controller: DeviceController, profile: Profile) -> Result<(), String> {
    use std::path::PathBuf;

    let (key_w, key_h) = match profile.key_count {
        6 => (80, 80),
        32 => (96, 96),
        8 => (120, 120), // Stream Deck+ (best-effort default)
        _ => (72, 72),
    };

    // Keys
    for (idx, k) in profile.keys.iter().enumerate() {
        let bg = match k.appearance.background {
            storage::profiles::Background::Solid { rgb } => Some(rgb),
            storage::profiles::Background::None => None,
        };
        let icon_path = k
            .appearance
            .icon_path
            .as_ref()
            .map(|s| PathBuf::from(s));
        let icon_ref = icon_path.as_deref();
        let text = k.appearance.text.as_deref();

        let jpeg = render::lcd::render_lcd_jpeg(
            key_w,
            key_h,
            bg,
            icon_ref,
            text,
        )
        .map_err(|e| e.to_string())?;

        controller
            .set_key_image_jpeg(idx as u8, jpeg)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Stream Deck+ extras (best-effort sizes; device protocol may differ by firmware).
    if profile.key_count == 8 {
        // Dials
        for (idx, d) in profile.dials.iter().enumerate().take(4) {
            let bg = match d.appearance.background {
                storage::profiles::Background::Solid { rgb } => Some(rgb),
                storage::profiles::Background::None => None,
            };
            let icon_path = d
                .appearance
                .icon_path
                .as_ref()
                .map(|s| PathBuf::from(s));
            let icon_ref = icon_path.as_deref();
            let text = d.appearance.text.as_deref();

            let jpeg = render::lcd::render_lcd_jpeg(100, 100, bg, icon_ref, text)
                .map_err(|e| e.to_string())?;

            controller
                .set_dial_image_jpeg(idx as u8, jpeg)
                .await
                .map_err(|e| e.to_string())?;
        }

        // Touch strip
        let bg = match profile.touch_strip.appearance.background {
            storage::profiles::Background::Solid { rgb } => Some(rgb),
            storage::profiles::Background::None => None,
        };
        let icon_path = profile
            .touch_strip
            .appearance
            .icon_path
            .as_ref()
            .map(|s| PathBuf::from(s));
        let icon_ref = icon_path.as_deref();
        let text = profile.touch_strip.appearance.text.as_deref();

        let jpeg = render::lcd::render_lcd_jpeg(800, 100, bg, icon_ref, text)
            .map_err(|e| e.to_string())?;

        controller
            .set_touch_strip_image_jpeg(jpeg)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

async fn set_brightness_async(controller: DeviceController, percent: u8) -> Result<(), String> {
    controller
        .set_brightness(percent)
        .await
        .map_err(|e| e.to_string())
}
