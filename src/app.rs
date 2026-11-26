//! Application state, start screen (projects, bus, catalogs, settings), dialogs and saving.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align, Align2, Color32, Layout, Margin, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::bus::Bus;
use crate::model::*;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Project(u64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OvTab {
    Projects,
    Bus,
    Catalogs,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum View {
    Buildings,
    Topology,
    GroupAddresses,
    Devices,
    Catalog,
    Monitors,
}

impl View {
    pub const ALL: [View; 6] = [View::Topology, View::Buildings, View::GroupAddresses, View::Devices, View::Catalog, View::Monitors];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Sel {
    None,
    Space(u32),
    Area(u8),
    Line(u8, u8),
    Device(u32),
    Main(u8),
    Middle(u8, u8),
    Ga(u32),
}

/// What used to be the bottom dock; now shown in a panel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    Catalog,
    GroupMonitor,
    BusMonitor,
    Jobs,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PropTab {
    Settings,
    Comments,
    Information,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum DevTab {
    GroupObjects,
    Parameters,
}

/// Which monitor a Monitors panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum MonView {
    GroupMonitor,
    BusMonitor,
    Jobs,
    DeviceInfo,
    ProgMode,
    AddressCheck,
    LineScan,
}

fn d_true() -> bool {
    true
}
fn d_one() -> String {
    "1".into()
}
fn d_size() -> f32 {
    320.0
}
fn d_tree() -> f32 {
    280.0
}
fn d_mon() -> MonView {
    MonView::GroupMonitor
}
fn d_dev_tab() -> DevTab {
    DevTab::Parameters
}
fn d_view() -> View {
    View::Topology
}
fn d_sel() -> Sel {
    Sel::None
}
fn d_last() -> (View, Sel) {
    (View::Topology, Sel::None)
}

/// One panel of a project view: its own content kind, selection and editor state.
#[derive(Clone, Serialize, Deserialize)]
pub struct Pane {
    #[serde(default = "crate::model::fresh_id")]
    pub id: u64,
    #[serde(default = "d_view")]
    pub view: View,
    #[serde(default = "d_sel")]
    pub sel: Sel,
    #[serde(default)]
    pub open_device: Option<u32>,
    #[serde(default = "d_dev_tab")]
    pub dev_tab: DevTab,
    #[serde(default)]
    pub param_page: String,
    #[serde(default)]
    pub sel_object: Option<usize>,
    #[serde(skip)]
    pub link_pick: Option<u32>,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub dev_folder: u8,
    #[serde(skip)]
    pub cat_filter: String,
    #[serde(default)]
    pub cat_manu: Option<String>,
    #[serde(skip)]
    pub cat_sel: Option<usize>,
    #[serde(skip)]
    pub send_ga: Option<u32>,
    #[serde(default = "d_one")]
    pub send_value: String,
    #[serde(skip)]
    pub mon_filter: String,
    #[serde(skip)]
    pub mon_sel: Option<u64>,
    #[serde(default = "d_mon")]
    pub mon_view: MonView,
    #[serde(default = "d_true")]
    pub show_ifaces: bool,
    /// Diagnostics: address and line typed in, repeated programming mode search.
    #[serde(default)]
    pub diag_addr: String,
    #[serde(default)]
    pub diag_line: String,
    #[serde(skip)]
    pub diag_auto: bool,
    #[serde(skip)]
    pub diag_next: f64,
    #[serde(skip)]
    pub back: Vec<(View, Sel)>,
    #[serde(skip)]
    pub fwd: Vec<(View, Sel)>,
    #[serde(skip, default = "d_last")]
    pub last: (View, Sel),
    /// Height in the stack (points)
    #[serde(default = "d_size")]
    pub size: f32,
    /// Width of the panel's tree
    #[serde(default = "d_tree")]
    pub tree_w: f32,
}

impl Default for Pane {
    fn default() -> Self {
        Pane::new(View::Topology)
    }
}

impl Pane {
    pub fn new(view: View) -> Self {
        Pane {
            id: fresh_id(),
            view,
            sel: Sel::None,
            open_device: None,
            dev_tab: DevTab::Parameters,
            param_page: String::new(),
            sel_object: None,
            link_pick: None,
            filter: String::new(),
            dev_folder: 0,
            cat_filter: String::new(),
            cat_manu: None,
            cat_sel: None,
            send_ga: None,
            send_value: "1".into(),
            mon_filter: String::new(),
            mon_sel: None,
            mon_view: MonView::GroupMonitor,
            show_ifaces: true,
            diag_addr: String::new(),
            diag_line: String::new(),
            diag_auto: false,
            diag_next: 0.0,
            back: vec![],
            fwd: vec![],
            last: (view, Sel::None),
            size: 320.0,
            tree_w: 280.0,
        }
    }
    pub fn monitors(m: MonView) -> Self {
        let mut p = Pane::new(View::Monitors);
        p.mon_view = m;
        p
    }
}

/// One project view (a tab): a stack of panels plus the sidebar state.
/// The focused panel's state lives in `p` while the program runs; `panes[focus]` is then a placeholder.
#[derive(Clone, Serialize, Deserialize)]
pub struct Ws {
    #[serde(skip)]
    pub p: Pane,
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub focus: usize,
    #[serde(default = "d_prop_tab")]
    pub prop_tab: PropTab,
    #[serde(default = "d_side")]
    pub side: crate::chrome::Side,
    #[serde(default = "d_true")]
    pub side_open: bool,
    #[serde(default)]
    pub run_tab: u8,
    #[serde(skip, default = "d_add")]
    pub add_count: u32,
    #[serde(skip)]
    pub knx_cache: Option<(u32, u64, std::sync::Arc<crate::knxapp::Visible>)>,
    /// A view to show in some panel at the end of the frame (from buttons like "Group Monitor").
    #[serde(skip)]
    pub pending_show: Option<(View, Option<MonView>)>,
    #[serde(skip)]
    pub new_focus: Option<usize>,
    #[serde(skip)]
    pub pane_ops: Vec<PaneOp>,
    /// Last used tree panel (view, selection): where catalog panels add devices.
    #[serde(skip, default = "d_last")]
    pub last_tree: (View, Sel),
    /// Link requested by drag & drop: (device, object index, group address).
    #[serde(skip)]
    pub pending_link: Option<(u32, usize, u32)>,
    /// Address or line to fill in on the diagnostics page shown by `pending_show`.
    #[serde(skip)]
    pub diag_prefill: Option<String>,
}

fn d_prop_tab() -> PropTab {
    PropTab::Settings
}
fn d_side() -> crate::chrome::Side {
    crate::chrome::Side::Properties
}
fn d_add() -> u32 {
    1
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaneOp {
    /// Add a panel after the given index
    Add(usize, View),
    Close(usize),
}

impl Default for Ws {
    fn default() -> Self {
        Ws::with_panes(vec![Pane::new(View::Topology), {
            let mut c = Pane::new(View::Catalog);
            c.size = 260.0;
            c
        }])
    }
}

impl Ws {
    pub fn with_panes(mut panes: Vec<Pane>) -> Self {
        let p = std::mem::take(&mut panes[0]);
        Ws {
            p,
            panes,
            focus: 0,
            prop_tab: PropTab::Settings,
            side: crate::chrome::Side::Properties,
            side_open: true,
            run_tab: 0,
            add_count: 1,
            knx_cache: None,
            pending_show: None,
            new_focus: None,
            pane_ops: vec![],
            last_tree: (View::Topology, Sel::None),
            pending_link: None,
            diag_prefill: None,
        }
    }

    /// A second view of a project: group addresses and the monitors.
    pub fn second() -> Self {
        let mut ga = Pane::new(View::GroupAddresses);
        ga.size = 360.0;
        Ws::with_panes(vec![ga, Pane::monitors(MonView::GroupMonitor)])
    }

    /// For saving: all panels in place.
    pub fn saved(&self) -> Ws {
        let mut w = self.clone();
        if w.focus < w.panes.len() {
            w.panes[w.focus] = self.p.clone();
        }
        w.knx_cache = None;
        w
    }

    /// After loading: move the focused panel into `p`.
    pub fn restored(mut self) -> Ws {
        if self.panes.is_empty() {
            return Ws::default();
        }
        self.focus = self.focus.min(self.panes.len() - 1);
        self.p = std::mem::take(&mut self.panes[self.focus]);
        self
    }

    /// Short name of what the view shows (for the tab).
    pub fn subtitle(&self) -> &'static str {
        crate::chrome::view_label(self.p.view).1
    }
}

#[derive(Serialize, Deserialize)]
pub struct Store {
    pub projects: Vec<Project>,
    #[serde(default)]
    pub dark: bool,
    #[serde(default = "one")]
    pub zoom: f32,
    #[serde(default = "default_style")]
    pub default_ga_style: GaStyle,
    /// Real interfaces added by hand: "ip:port" or "routing".
    #[serde(default)]
    pub saved_interfaces: Vec<String>,
    /// Language for texts of imported products.
    #[serde(default = "default_lang")]
    pub product_language: String,
    /// Saved project views (tabs with their panels) per project id.
    #[serde(default)]
    pub layouts: std::collections::HashMap<String, Vec<Ws>>,
    #[serde(default)]
    pub layout_active: std::collections::HashMap<String, usize>,
    /// Built-in MCP server for AI assistants (this computer only).
    #[serde(default = "yes")]
    pub mcp_enabled: bool,
    #[serde(default = "mcp_port")]
    pub mcp_port: u16,
}
fn yes() -> bool {
    true
}
fn mcp_port() -> u16 {
    crate::mcp::DEFAULT_PORT
}
fn one() -> f32 {
    1.0
}
fn default_lang() -> String {
    "en-US".into()
}
fn default_style() -> GaStyle {
    GaStyle::ThreeLevel
}

/// Change of project tabs requested this frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TabOp {
    Switch(usize),
    New,
    Close(usize),
    /// Close the whole project (all its views).
    CloseProject,
}

pub struct NewProject {
    pub name: String,
    pub backbone: Medium,
    pub line: Medium,
    pub style: GaStyle,
    pub sample: bool,
}

pub struct Shot {
    pub path: PathBuf,
    pub frames: u32,
    pub requested: bool,
    pub done: Arc<AtomicBool>,
}

pub struct App {
    pub store: Store,
    pub catalog: Vec<Product>,
    pub bus: Bus,
    pub screen: Screen,
    pub ov_tab: OvTab,
    pub ov_sel: Option<u64>,
    pub ov_filter: String,
    pub new_project: Option<NewProject>,
    pub confirm_delete: Option<u64>,
    pub confirm_delete_device: Option<u32>,
    pub about: bool,
    pub ws: Ws,
    /// All views (tabs) of the open project; the slot of the active one holds a placeholder.
    pub tabs: Vec<Ws>,
    pub active_tab: usize,
    /// Tab change requested this frame: Some(index) to switch, or a new tab to add.
    pub tab_op: Option<TabOp>,
    pub sel_labels: bool,
    pub msg: String,
    pub msg_time: f64,
    pub undo: Vec<Project>,
    pub redo: Vec<Project>,
    pub changed: bool,
    pub coalesce: bool,
    pub last_push: f64,
    pub dirty: bool,
    pub save_timer: f32,
    pub theme_applied: bool,
    pub shot: Option<Shot>,
    pub add_ip: String,
    pub add_port: String,
    pub library: crate::library::Library,
    pub import_job: Option<crate::import_ui::ImportJob>,
    pub pw_prompt: Option<crate::import_ui::PasswordPrompt>,
    pub import_report: Option<crate::import_ui::ImportReport>,
    pub dl_confirm: Option<crate::dl_ui::DlConfirm>,
    pub report_view: Option<(String, String)>,
    pub problem: Option<crate::dl_ui::Problem>,
    /// A download waiting for re-imported product data: (project id, device, kind).
    pub pending_download: Option<(u64, u32, crate::bus::JobKind)>,
    pub retry_download: bool,
    /// A backup file waiting for confirmation before it replaces the local data: (file, description).
    pub confirm_restore: Option<(PathBuf, String)>,
    /// Built-in MCP server; its tool calls are carried out at the start of each frame.
    pub mcp: crate::mcp::McpServer,
    /// Port typed in Settings (applied with the button).
    pub mcp_port_text: String,
}

/// The app's data folder (projects, product library, download logs).
/// A folder from the app's earlier name is taken over once, so nothing is lost.
/// Windows: %APPDATA%\BusCraft · Linux: ~/.local/share/BusCraft · macOS: ~/Library/Application Support/BusCraft
pub fn data_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = || env("HOME").unwrap_or_else(|| PathBuf::from("."));
    let base = if cfg!(windows) {
        env("APPDATA").unwrap_or_else(|| PathBuf::from("."))
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support")
    } else {
        env("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local").join("share"))
    };
    let dir = base.join("BusCraft");
    let old = base.join("BusPlanStudio");
    if !dir.exists() && old.exists() && std::fs::rename(&old, &dir).is_err() {
        return old;
    }
    dir
}

fn store_path() -> PathBuf {
    data_dir().join("projects.json")
}

impl App {
    pub fn new(shot: Option<PathBuf>) -> Self {
        let library = crate::library::Library::load(if shot.is_some() { None } else { Some(crate::library::Library::default_dir()) });
        let mut catalog = catalog();
        catalog.extend(library.products.iter().cloned());
        let store = if shot.is_some() {
            None
        } else {
            std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str::<Store>(&s).ok())
        };
        let store = store.unwrap_or_else(|| {
            let mut office = Project::new("Office Floor 3", Medium::IP, Medium::TP, GaStyle::ThreeLevel);
            office.status = ProjectStatus::Planning;
            office.number = "2026-014".into();
            office.modified -= 86_400 * 3;
            office.created -= 86_400 * 9;
            Store { projects: vec![sample_project(&catalog), office], dark: false, zoom: 1.0, default_ga_style: GaStyle::ThreeLevel, saved_interfaces: vec![], product_language: default_lang(), layouts: Default::default(), layout_active: Default::default(), mcp_enabled: true, mcp_port: mcp_port() }
        });
        let mut mcp = crate::mcp::McpServer::default();
        mcp.port = store.mcp_port;
        if store.mcp_enabled && shot.is_none() {
            mcp.start(store.mcp_port);
        } else if store.mcp_enabled {
            mcp.status = "Not started while taking a screenshot".into();
        }
        let ov_sel = store.projects.first().map(|p| p.id);
        let mut bus = Bus::new();
        for s in &store.saved_interfaces {
            if s == "routing" {
                bus.add_interface(Bus::routing_interface());
            } else if let Ok(a) = s.parse::<std::net::SocketAddrV4>() {
                bus.add_interface(Bus::manual_tunnel(*a.ip(), a.port()));
            }
        }
        App {
            store,
            catalog,
            bus,
            add_ip: String::new(),
            add_port: "3671".into(),
            library,
            import_job: None,
            pw_prompt: None,
            import_report: None,
            dl_confirm: None,
            report_view: None,
            problem: None,
            pending_download: None,
            retry_download: false,
            confirm_restore: None,
            mcp_port_text: String::new(),
            mcp,
            screen: Screen::Overview,
            ov_tab: OvTab::Projects,
            ov_sel,
            ov_filter: String::new(),
            new_project: None,
            confirm_delete: None,
            confirm_delete_device: None,
            about: false,
            ws: Ws::default(),
            tabs: vec![Ws::default()],
            active_tab: 0,
            tab_op: None,
            sel_labels: true,
            msg: "Ready".into(),
            msg_time: 0.0,
            undo: vec![],
            redo: vec![],
            changed: false,
            coalesce: false,
            last_push: -10.0,
            dirty: false,
            save_timer: 0.0,
            theme_applied: false,
            shot: shot.map(|path| Shot { path, frames: 0, requested: false, done: Arc::new(AtomicBool::new(false)) }),
        }
    }

    /// Open the first project in a given view (used for previews).
    pub fn preview(&mut self, what: &str) {
        let Some(p) = self.store.projects.first() else { return };
        let id = p.id;
        let dev = p.devices.iter().find(|d| d.product.starts_with("Universal dimming")).map(|d| d.id);
        let room = p.devices.iter().find(|d| d.product.starts_with("Push button")).and_then(|d| p.room_of(d.id)).map(|s| s.id);
        let ga = p.addresses.first().map(|g| g.id);
        self.open_project(id);
        match what {
            "knx" | "dl" => {
                // Import a product file given in BPS_IMPORT and open the new device's parameters.
                let Ok(file) = std::env::var("BPS_IMPORT") else { return };
                let lang = self.store.product_language.clone();
                let Ok(out) = crate::knxfile::import(std::path::Path::new(&file), None, true, &lang, &crate::knxfile::Progress::default()) else { return };
                self.library.add_masks(out.masks);
                let _ = self.library.add(out.products, out.apps);
                self.refresh_catalog();
                let prods = self.library.products.clone();
                let prod = prods
                    .iter()
                    .find(|p| p.app_id.as_ref().and_then(|a| self.library.app(a)).and_then(|a| a.load.clone()).map(|l| l.mask == "MV-07B0" && l.segments.iter().any(|s| !s.data.is_empty())).unwrap_or(false))
                    .or(prods.first())
                    .cloned();
                let Some(prod) = prod else { return };
                let Some(i) = self.store.projects.iter().position(|p| p.id == id) else { return };
                let did = self.store.projects[i].add_device_from(&prod, 1, 1);
                if let Some(app) = prod.app_id.as_ref().and_then(|a| self.library.app(a)) {
                    app.sync_objects(self.store.projects[i].device_mut(did).unwrap());
                }
                self.ws.p.view = View::Topology;
                self.ws.p.sel = Sel::Device(did);
                self.ws.p.open_device = Some(did);
                self.ws.p.dev_tab = DevTab::Parameters;
                if what == "dl" {
                    let p = self.store.projects[i].clone();
                    match self.plan_for(&p, did, crate::bus::JobKind::Full) {
                        Ok(Some(plan)) => {
                            let d = p.device(did).unwrap();
                            self.dl_confirm = Some(crate::dl_ui::DlConfirm { device: did, kind: crate::bus::JobKind::Full, title: format!("{} to {} {}", plan.kind, d.address(), d.name), plan });
                        }
                        Ok(None) => eprintln!("no plan"),
                        Err(e) => eprintln!("plan error: {e}"),
                    }
                }
            }
            "problem" => {
                self.problem = Some(crate::dl_ui::Problem {
                    title: "Download all – 1.1.28 Pushbutton FF Serie".into(),
                    text: "This product was imported with an older version that ignored the application's automatic parameter rules. Import its product file or the project again (once) – then download.".into(),
                    reimport: true,
                });
            }
            "report" => {
                // Example report built from table contents as read in a real download log.
                let at = [0x00, 0x04, 0x18, 0x00, 0x18, 0x01, 0xF5, 0x00, 0xF5, 0x01];
                let asc = [0x00, 0x04, 0x00, 0x01, 0x00, 0x0E, 0x00, 0x02, 0x00, 0x0F, 0x00, 0x03, 0x00, 0x0E, 0x00, 0x04, 0x00, 0x0E];
                let mut app_dev: Vec<u8> = (0..64u8).collect();
                let app_exp = app_dev.clone();
                for b in &mut app_dev[24..36] {
                    *b = 0xA5;
                }
                let mut lines = vec!["Application: loaded".to_string(), "Group object table: loaded".into(), "Association table: loaded".into(), "Group address table: loaded".into(), "Application id in the device: 00 B6 45 44 06 – matches the project".into()];
                lines.extend(crate::knx::check_report(1, 0x3400, &at, &at));
                lines.extend(crate::knx::check_report(2, 0x36DC, &asc, &asc));
                lines.extend(crate::knx::check_report(4, 0x3F6E, &app_dev, &app_exp));
                self.report_view = Some(("Device check 1.1.28 Example device".into(), lines.join("\n")));
            }
            "home" => {
                self.screen = Screen::Overview;
                self.ov_tab = OvTab::Projects;
            }
            "settings" => {
                self.screen = Screen::Overview;
                self.ov_tab = OvTab::Settings;
            }
            "bus" => {
                self.screen = Screen::Overview;
                self.ov_tab = OvTab::Bus;
                self.bus.add_interface(crate::bus::Bus::manual_tunnel(std::net::Ipv4Addr::new(192, 168, 1, 20), 3671));
                self.bus.add_interface(crate::bus::Bus::routing_interface());
            }
            "buildings" => {
                self.ws.p.view = View::Buildings;
                self.ws.p.sel = room.map(Sel::Space).unwrap_or(Sel::None);
            }
            "groups" => {
                self.ws.p.view = View::GroupAddresses;
                self.ws.p.sel = ga.map(Sel::Ga).unwrap_or(Sel::None);
                self.ws.panes[1] = Pane::monitors(MonView::GroupMonitor);
            }
            "device" | "params" => {
                self.ws.p.view = View::Topology;
                self.ws.p.sel = dev.map(Sel::Device).unwrap_or(Sel::None);
                self.ws.p.open_device = dev;
                self.ws.p.sel_object = Some(0);
                self.ws.p.dev_tab = if what == "params" { DevTab::Parameters } else { DevTab::GroupObjects };
                self.ws.panes.truncate(1);
            }
            "monitor" => {
                self.ws.p.view = View::Topology;
                self.ws.p.sel = Sel::Line(1, 1);
                self.ws.panes[1] = Pane::monitors(MonView::BusMonitor);
            }
            "jobs" => {
                self.ws.p.view = View::Devices;
                if let Some(p) = self.store.projects.first() {
                    let p = p.clone();
                    for d in p.devices.iter().filter(|d| d.needs_download()) {
                        let _ = self.bus.enqueue(&p, d.id, crate::bus::JobKind::Full);
                    }
                }
                self.ws.panes[1] = Pane::monitors(MonView::Jobs);
            }
            "scan" | "devinfo" | "progmode" | "check" => {
                // Diagnostics pages, with addresses programmed in the simulated installation.
                let pi = self.store.projects.iter().position(|p| p.id == id).unwrap_or(0);
                for d in self.store.projects[pi].devices.iter_mut() {
                    d.adr_ok = d.has_app();
                }
                let mut p = self.store.projects[pi].clone();
                let first = p.devices.iter().find(|d| d.has_app() && d.area == 1 && d.line == 1).map(|d| d.address()).unwrap_or_default();
                let dimmer = dev.and_then(|d| p.device(d)).map(|d| d.address()).unwrap_or(first.clone());
                let (kind, target, view) = match what {
                    "scan" => (crate::diag::DiagKind::LineScan, "1.1".to_string(), MonView::LineScan),
                    "devinfo" => (crate::diag::DiagKind::DeviceInfo, dimmer.clone(), MonView::DeviceInfo),
                    "check" => (crate::diag::DiagKind::Check, "1.1.30".to_string(), MonView::AddressCheck),
                    _ => (crate::diag::DiagKind::SetProg(true), first.clone(), MonView::ProgMode),
                };
                let _ = self.bus.start_diag(kind, &target);
                for _ in 0..80 {
                    self.bus.tick(0.1, Some(&mut p));
                }
                if what == "progmode" {
                    let _ = self.bus.start_diag(crate::diag::DiagKind::ProgScan, "");
                    for _ in 0..40 {
                        self.bus.tick(0.1, Some(&mut p));
                    }
                }
                self.ws.p = Pane::monitors(view);
                self.ws.p.diag_addr = target.clone();
                self.ws.p.diag_line = "1.1".into();
                self.ws.panes.truncate(1);
            }
            "monitors" => {
                // Monitors in the first panel, like the original's monitor view
                self.ws.p = Pane::monitors(MonView::GroupMonitor);
                self.ws.panes.truncate(1);
                // A few telegrams, including time of day (10.001) and date (11.001)
                if let Some(p) = self.store.projects.first() {
                    let p = p.clone();
                    for g in &p.addresses {
                        let v = self.bus.random_value(&g.dpt);
                        let _ = self.bus.send_group(&p, g.id, crate::bus::Service::Write, &v, None);
                    }
                }
            }
            "values" => {
                // Group address list with last values, and one address with its linked objects
                if let Some(p) = self.store.projects.first() {
                    let p = p.clone();
                    for g in &p.addresses {
                        let v = self.bus.random_value(&g.dpt);
                        let _ = self.bus.send_group(&p, g.id, crate::bus::Service::Write, &v, None);
                    }
                }
                let gp = ga.and_then(|g| self.store.projects.first().and_then(|p| p.ga(g)).map(|x| (x.main, x.middle)));
                self.ws.p.view = View::GroupAddresses;
                self.ws.p.sel = gp.map(|(m, mi)| Sel::Middle(m, mi)).unwrap_or(Sel::None);
                self.ws.p.size = 330.0;
                let mut g = Pane::new(View::GroupAddresses);
                g.sel = ga.map(Sel::Ga).unwrap_or(Sel::None);
                self.ws.panes[1] = g;
            }
            "panes" => {
                // Three panels: topology with a device, group addresses, group monitor
                self.ws.p.view = View::Topology;
                self.ws.p.sel = dev.map(Sel::Device).unwrap_or(Sel::None);
                self.ws.p.open_device = dev;
                self.ws.p.dev_tab = DevTab::GroupObjects;
                self.ws.p.size = 300.0;
                let mut g = Pane::new(View::GroupAddresses);
                g.sel = ga.map(Sel::Ga).unwrap_or(Sel::None);
                g.size = 230.0;
                self.ws.panes[1] = g;
                self.ws.panes.push(Pane::monitors(MonView::GroupMonitor));
                self.tabs.push(Ws::second());
            }
            _ => {
                self.ws.p.view = View::Topology;
                self.ws.p.sel = Sel::Line(1, 1);
            }
        }
    }

    pub fn say(&mut self, m: impl Into<String>) {
        self.msg = m.into();
        self.msg_time = self.bus.clock;
    }

    /// Mark that the open project was changed this frame (for undo and saving).
    pub fn mark(&mut self) {
        self.changed = true;
    }
    /// Like [`mark`], but repeated edits (typing) are merged into one undo step.
    pub fn mark_typing(&mut self) {
        self.changed = true;
        self.coalesce = true;
    }

    fn save(&mut self) {
        if self.shot.is_some() {
            return;
        }
        self.store_layout();
        let path = store_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(&self.store) {
            Ok(s) => {
                if std::fs::write(&path, s).is_err() {
                    self.say("Could not save projects");
                }
            }
            Err(_) => self.say("Could not save projects"),
        }
        self.dirty = false;
    }

    fn project_index(&self, id: u64) -> Option<usize> {
        self.store.projects.iter().position(|p| p.id == id)
    }

    /// Remember the open project's views (for restoring them when it is opened again).
    pub fn store_layout(&mut self) {
        let Screen::Project(id) = self.screen else { return };
        self.store_layout_for(id);
    }

    fn store_layout_for(&mut self, id: u64) {
        let mut tabs: Vec<Ws> = self.tabs.iter().map(|w| w.saved()).collect();
        if self.active_tab < tabs.len() {
            tabs[self.active_tab] = self.ws.saved();
        }
        self.store.layouts.insert(id.to_string(), tabs);
        self.store.layout_active.insert(id.to_string(), self.active_tab);
    }

    pub fn open_project(&mut self, id: u64) {
        self.store_layout();
        self.screen = Screen::Project(id);
        // Restore the saved views of this project, or start with the default one.
        let saved = self.store.layouts.get(&id.to_string()).cloned().unwrap_or_default();
        let mut tabs: Vec<Ws> = saved.into_iter().map(|w| w.restored()).collect();
        if tabs.is_empty() {
            tabs.push(Ws::default());
        }
        let active = self.store.layout_active.get(&id.to_string()).copied().unwrap_or(0).min(tabs.len() - 1);
        self.ws = std::mem::take(&mut tabs[active]);
        self.tabs = tabs;
        self.active_tab = active;
        self.undo.clear();
        self.redo.clear();
        self.bus.log.clear();
        if let Some(p) = self.store.projects.iter().find(|p| p.id == id) {
            let name = p.name.clone();
            self.say(format!("Opened project \"{name}\""));
        }
    }

    fn apply_tab_op(&mut self, proj: &Project) {
        let Some(op) = self.tab_op.take() else { return };
        match op {
            TabOp::Switch(i) if i != self.active_tab && i < self.tabs.len() => {
                self.tabs[self.active_tab] = std::mem::take(&mut self.ws);
                self.ws = std::mem::take(&mut self.tabs[i]);
                self.active_tab = i;
            }
            TabOp::New => {
                self.tabs[self.active_tab] = std::mem::take(&mut self.ws);
                self.tabs.push(Ws::second());
                self.active_tab = self.tabs.len() - 1;
                self.ws = std::mem::take(&mut self.tabs[self.active_tab]);
                self.say(format!("Opened another view of \"{}\"", proj.name));
                self.dirty = true;
            }
            TabOp::CloseProject => {
                self.screen = Screen::Overview;
                self.say(format!("Closed project \"{}\"", proj.name));
            }
            TabOp::Close(i) if i < self.tabs.len() => {
                if self.tabs.len() == 1 {
                    // Closing the last view closes the project (the view is kept for next time).
                    self.screen = Screen::Overview;
                    self.say(format!("Closed project \"{}\"", proj.name));
                    return;
                }
                if i == self.active_tab {
                    self.tabs.remove(i);
                    self.active_tab = i.min(self.tabs.len() - 1);
                    self.ws = std::mem::take(&mut self.tabs[self.active_tab]);
                } else {
                    self.tabs.remove(i);
                    if i < self.active_tab {
                        self.active_tab -= 1;
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ frame

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.theme_applied {
            theme::apply(&ctx, self.store.dark, self.store.zoom);
            self.theme_applied = true;
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        // A click selects table rows; holding Shift allows selecting text instead.
        let shift = ctx.input(|i| i.modifiers.shift);
        if shift != self.sel_labels {
            self.sel_labels = shift;
            ctx.global_style_mut(|s| s.interaction.selectable_labels = shift);
        }

        self.mcp_poll(&ctx);
        match self.screen {
            Screen::Overview => {
                self.bus.tick(dt, None);
                self.overview(ui);
            }
            Screen::Project(id) => {
                if let Some(idx) = self.project_index(id) {
                    let mut proj = self.store.projects.remove(idx);
                    self.bus.tick(dt, Some(&mut proj));
                    let before = proj.clone();
                    self.changed = false;
                    self.coalesce = false;
                    self.workspace(ui, &mut proj);
                    self.apply_tab_op(&proj);
                    if self.screen != Screen::Project(id) {
                        // Project closed: keep its views for the next time
                        self.store_layout_for(id);
                        self.dirty = true;
                    }
                    if self.changed {
                        let now = self.bus.clock;
                        if !(self.coalesce && now - self.last_push < 1.5) {
                            self.undo.push(before);
                            if self.undo.len() > 100 {
                                self.undo.remove(0);
                            }
                            self.redo.clear();
                        }
                        self.last_push = now;
                        proj.touch();
                        self.dirty = true;
                    }
                    self.store.projects.insert(idx, proj);
                } else {
                    self.screen = Screen::Overview;
                }
            }
        }

        self.poll_import();
        self.import_dialogs(&ctx);
        if let Some(r) = self.bus.reports.pop() {
            self.report_view = Some(r);
        }
        self.report_dialog(&ctx);
        self.problem_dialog(&ctx);
        self.restore_dialog(&ctx);
        for n in std::mem::take(&mut self.bus.notices) {
            self.say(n);
        }
        self.dialogs(&ctx);

        // Saving
        self.save_timer += dt;
        if self.dirty && self.save_timer > 2.0 {
            self.save_timer = 0.0;
            self.save();
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.store_layout();
            self.save();
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        self.screenshot(&ctx);
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        ctx.request_repaint();
        if shot.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if shot.frames >= 120 && (!shot.requested || shot.frames % 120 == 0) {
            shot.requested = true;
            let path = shot.path.clone();
            let done = shot.done.clone();
            ctx.request_screenshot(move |img| {
                let [w, h] = img.size;
                let mut bytes = Vec::with_capacity(w * h * 4);
                for p in &img.pixels {
                    let [r, g, b, _] = p.to_array();
                    bytes.extend_from_slice(&[r, g, b, 255]);
                }
                if let Some(dir) = path.parent() {
                    if !dir.as_os_str().is_empty() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                }
                if let Some(im) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                    if let Err(e) = im.save(&path) {
                        eprintln!("could not save screenshot: {e}");
                    }
                }
                done.store(true, Ordering::SeqCst);
            });
        }
        if shot.frames > 1200 {
            eprintln!("screenshot timed out");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    // ------------------------------------------------------------------ undo

    pub fn do_undo(&mut self, proj: &mut Project) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(proj, prev));
            self.dirty = true;
            self.say("Undone");
        }
    }
    pub fn do_redo(&mut self, proj: &mut Project) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(proj, next));
            self.dirty = true;
            self.say("Redone");
        }
    }

    // ------------------------------------------------------------------ shared bars

    pub fn status_bar(&mut self, ui: &mut egui::Ui, right: &str) {
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(if self.store.dark { Color32::from_gray(22) } else { Color32::from_rgb(0xF3, 0xF3, 0xF3) }).inner_margin(Margin::symmetric(10, 3)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let dot = match self.bus.state {
                        crate::bus::LinkState::Connected => theme::OK,
                        crate::bus::LinkState::Connecting => theme::WARN,
                        _ => theme::BAD,
                    };
                    ui.label(RichText::new("●").color(dot));
                    let addr = if self.bus.connected { self.bus.own_address.clone() } else { self.bus.iface().address.clone() };
                    let state = self.bus.state_text();
                    ui.add(egui::Label::new(RichText::new(format!("{} ({addr}) – {state}", self.bus.iface().name)).small()).truncate())
                        .on_hover_text(state);
                    ui.separator();
                    ui.label(RichText::new(&self.msg).small());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(right).small().color(theme::MUTED));
                    });
                });
            });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("ov_header")
            .frame(egui::Frame::new().fill(theme::HEADER).inner_margin(Margin { left: 10, right: 14, top: 6, bottom: 0 }))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::chrome::brand(ui);
                    ui.label(RichText::new("BusCraft").size(16.0).strong().color(theme::HEADER_TEXT));
                    ui.add_space(28.0);
                    for (t, label) in [(OvTab::Projects, "Overview"), (OvTab::Bus, "Bus"), (OvTab::Catalogs, "Catalogs"), (OvTab::Settings, "Settings")] {
                        if theme::header_tab(ui, self.ov_tab == t, label).clicked() {
                            self.ov_tab = t;
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("?").color(theme::HEADER_TEXT)).fill(Color32::TRANSPARENT)).on_hover_text("About").clicked() {
                            self.about = true;
                        }
                    });
                });
            });
    }

    // ------------------------------------------------------------------ overview

    fn overview(&mut self, ui: &mut egui::Ui) {
        self.header(ui);
        let n = self.store.projects.len();
        self.status_bar(ui, &format!("{n} projects"));
        match self.ov_tab {
            OvTab::Projects => self.ov_projects(ui),
            OvTab::Bus => {
                egui::CentralPanel::default().show(ui, |ui| self.bus_page(ui));
            }
            OvTab::Catalogs => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.heading("Catalogs");
                    ui.label(RichText::new("Products available for your projects. Open a project to add them to a line or room.").color(theme::MUTED));
                    ui.add_space(6.0);
                    let _ = self.catalog_browser(ui, false, true);
                });
            }
            OvTab::Settings => {
                egui::CentralPanel::default().show(ui, |ui| self.settings_page(ui));
            }
        }
    }

    fn ov_projects(&mut self, ui: &mut egui::Ui) {
        // Detail panel on the right
        egui::Panel::right("ov_detail").default_size(330.0).min_size(260.0).frame(theme::surface_frame(ui).inner_margin(Margin::same(14))).show(ui, |ui| {
            self.ov_detail(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(14)).fill(ui.visuals().panel_fill)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Projects");
                ui.label(RichText::new(format!("({})", self.store.projects.len())).color(theme::MUTED));
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("+ New project").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                    self.new_project = Some(NewProject {
                        name: "New project".into(),
                        backbone: Medium::IP,
                        line: Medium::TP,
                        style: self.store.default_ga_style,
                        sample: false,
                    });
                }
                if ui.add_enabled(self.ov_sel.is_some(), egui::Button::new("Open")).clicked() {
                    if let Some(id) = self.ov_sel {
                        self.open_project(id);
                    }
                }
                if ui.button("Import…").clicked() {
                    self.pick_and_import(false);
                }
                if ui.add_enabled(self.ov_sel.is_some(), egui::Button::new("Export…")).clicked() {
                    self.export_project();
                }
                if ui.add_enabled(self.ov_sel.is_some(), egui::Button::new("Duplicate")).clicked() {
                    if let Some(i) = self.ov_sel.and_then(|id| self.project_index(id)) {
                        let mut p = self.store.projects[i].clone();
                        p.id = fresh_id();
                        p.name = format!("{} (copy)", p.name);
                        p.created = now_secs();
                        p.modified = now_secs();
                        self.ov_sel = Some(p.id);
                        self.store.projects.insert(i + 1, p);
                        self.dirty = true;
                        self.say("Project duplicated");
                    }
                }
                if ui.add_enabled(self.ov_sel.is_some(), egui::Button::new("Delete")).clicked() {
                    self.confirm_delete = self.ov_sel;
                }
                ui.separator();
                if ui.button("💾 Back up everything…").on_hover_text("All projects and imported products in one .zip file – for moving to another computer").clicked() {
                    self.backup_all();
                }
                if ui.button("⟲ Restore backup…").on_hover_text("Load a backup made with \"Back up everything\"").clicked() {
                    self.pick_restore();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.ov_filter).hint_text("🔍 Search projects").desired_width(200.0));
                });
            });
            ui.add_space(6.0);

            let filter = self.ov_filter.to_lowercase();
            let rows: Vec<usize> = self
                .store
                .projects
                .iter()
                .enumerate()
                .filter(|(_, p)| filter.is_empty() || p.name.to_lowercase().contains(&filter) || p.number.to_lowercase().contains(&filter))
                .map(|(i, _)| i)
                .collect();
            let mut clicked = None;
            let mut open = None;
            theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                TableBuilder::new(ui)
                    .id_salt("projects")
                    .striped(true)
                    .resizable(true)
                    .sense(Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::initial(240.0).at_least(120.0).clip(true))
                    .column(Column::initial(110.0).clip(true))
                    .column(Column::initial(110.0))
                    .column(Column::initial(70.0))
                    .column(Column::initial(130.0))
                    .column(Column::remainder().at_least(100.0))
                    .header(24.0, |mut h| {
                        for t in ["Name", "Project number", "Status", "Devices", "Last modified", "Created"] {
                            h.col(|ui| {
                                ui.strong(t);
                            });
                        }
                    })
                    .body(|mut body| {
                        for &i in &rows {
                            let p = &self.store.projects[i];
                            body.row(26.0, |mut row| {
                                row.set_selected(self.ov_sel == Some(p.id));
                                row.col(|ui| {
                                    ui.label("📁");
                                    ui.label(&p.name);
                                });
                                row.col(|ui| {
                                    ui.label(&p.number);
                                });
                                row.col(|ui| {
                                    ui.label(p.status.label());
                                });
                                row.col(|ui| {
                                    ui.label(p.device_count().to_string());
                                });
                                row.col(|ui| {
                                    ui.label(fmt_time(p.modified));
                                });
                                row.col(|ui| {
                                    ui.label(fmt_time(p.created));
                                });
                                let r = row.response();
                                if r.clicked() {
                                    clicked = Some(p.id);
                                }
                                if r.double_clicked() {
                                    open = Some(p.id);
                                }
                            });
                        }
                    });
                if rows.is_empty() {
                    ui.label(RichText::new("No projects found. Create a new project or import one.").color(theme::MUTED));
                }
            });
            if let Some(id) = clicked {
                self.ov_sel = Some(id);
            }
            if let Some(id) = open {
                self.open_project(id);
            }
        });
    }

    fn ov_detail(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.ov_sel.and_then(|id| self.project_index(id)) else {
            ui.label(RichText::new("Select a project to see its details.").color(theme::MUTED));
            return;
        };
        let mut open = false;
        let mut changed = false;
        {
            let p = &mut self.store.projects[i];
            ui.label(RichText::new(&p.name).size(18.0).strong());
            ui.label(RichText::new(format!("Last modified {}", fmt_time(p.modified))).color(theme::MUTED).small());
            ui.add_space(8.0);
            if ui.add(egui::Button::new(RichText::new("Open project").color(Color32::WHITE).size(14.0)).fill(theme::ACCENT).min_size(egui::vec2(ui.available_width(), 30.0))).clicked() {
                open = true;
            }
            ui.add_space(10.0);
            theme::section(ui, "Details");
            egui::Grid::new("pdetail").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Name");
                changed |= ui.text_edit_singleline(&mut p.name).changed();
                ui.end_row();
                ui.label("Project number");
                changed |= ui.text_edit_singleline(&mut p.number).changed();
                ui.end_row();
                ui.label("Contract number");
                changed |= ui.text_edit_singleline(&mut p.contract).changed();
                ui.end_row();
                ui.label("Status");
                egui::ComboBox::from_id_salt("pstatus").selected_text(p.status.label()).show_ui(ui, |ui| {
                    for s in ProjectStatus::ALL {
                        changed |= ui.selectable_value(&mut p.status, s, s.label()).changed();
                    }
                });
                ui.end_row();
                ui.label("Group addresses");
                ui.label(p.ga_style.label());
                ui.end_row();
                ui.label("Created");
                ui.label(fmt_time(p.created));
                ui.end_row();
            });
            ui.add_space(6.0);
            theme::section(ui, "Contents");
            let mut spaces = 0;
            fn count(s: &Space, n: &mut usize) {
                *n += 1;
                for c in &s.children {
                    count(c, n);
                }
            }
            for b in &p.buildings {
                count(b, &mut spaces);
            }
            let lines: usize = p.areas.iter().map(|a| a.lines.len()).sum();
            egui::Grid::new("pcount").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                for (k, v) in [
                    ("Building parts and rooms", spaces),
                    ("Areas", p.areas.len()),
                    ("Lines", lines),
                    ("Devices", p.devices.len()),
                    ("Group addresses", p.addresses.len()),
                    ("Devices needing download", p.devices.iter().filter(|d| d.needs_download()).count()),
                ] {
                    ui.label(k);
                    ui.label(RichText::new(v.to_string()).strong());
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            theme::section(ui, "Comment");
            changed |= ui.add(egui::TextEdit::multiline(&mut p.comment).desired_rows(4).desired_width(f32::INFINITY)).changed();
        }
        if changed {
            self.dirty = true;
        }
        if open {
            let id = self.store.projects[i].id;
            self.open_project(id);
        }
    }

    pub fn bus_page(&mut self, ui: &mut egui::Ui) {
        use crate::bus::{Bus, LinkState};
        ui.heading("Bus connections");
        ui.label(RichText::new("Choose the interface used for programming and monitoring. Real KNXnet/IP interfaces and routers on your network are supported (tunnelling and routing); the simulated interfaces work without any hardware.").color(theme::MUTED));
        ui.add_space(8.0);
        theme::card_frame(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Current interface:").strong());
                ui.label(self.bus.iface().name.clone());
                ui.add_space(12.0);
                let c = match self.bus.state {
                    LinkState::Connected => theme::OK,
                    LinkState::Connecting => theme::WARN,
                    _ => theme::BAD,
                };
                ui.add(egui::Label::new(RichText::new(format!("● {}", self.bus.state_text())).color(c)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    match self.bus.state {
                        LinkState::Connected | LinkState::Connecting => {
                            if ui.button("Disconnect").clicked() {
                                self.bus.disconnect();
                                self.say("Disconnected from the bus");
                            }
                        }
                        _ => {
                            if ui.add(egui::Button::new(RichText::new("Connect").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                                self.bus.connect();
                                let n = self.bus.iface().name.clone();
                                self.say(format!("Connecting to {n}…"));
                            }
                        }
                    }
                });
            });
            if self.bus.connected {
                ui.label(RichText::new(format!("Own individual address {} · {}", self.bus.own_address, self.bus.link_detail)).small().color(theme::MUTED));
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            theme::section(ui, "Interfaces");
            ui.add_space(12.0);
            let label = if self.bus.searching { "Searching…" } else { "🔍 Search network" };
            if ui.add_enabled(!self.bus.searching, egui::Button::new(label)).on_hover_text("Find KNXnet/IP interfaces and routers on the local network").clicked() {
                self.bus.search();
            }
            if self.bus.searching {
                ui.spinner();
            }
            ui.separator();
            ui.label("Add IP interface:");
            ui.add(egui::TextEdit::singleline(&mut self.add_ip).hint_text("192.168.1.20").desired_width(120.0));
            ui.label(":");
            ui.add(egui::TextEdit::singleline(&mut self.add_port).desired_width(48.0));
            if ui.button("Add").clicked() {
                match (self.add_ip.trim().parse::<std::net::Ipv4Addr>(), self.add_port.trim().parse::<u16>()) {
                    (Ok(ip), Ok(port)) => {
                        let i = self.bus.add_interface(Bus::manual_tunnel(ip, port));
                        let key = format!("{ip}:{port}");
                        if !self.store.saved_interfaces.contains(&key) {
                            self.store.saved_interfaces.push(key);
                            self.dirty = true;
                        }
                        self.bus.select(i);
                        self.say(format!("Added IP interface {ip}:{port}"));
                    }
                    _ => self.say("Enter an IPv4 address like 192.168.1.20 and a port (usually 3671)"),
                }
            }
            if ui.button("Add IP routing").on_hover_text("Listen and send on the KNX multicast group (needs an IP router or IP devices)").clicked() {
                let i = self.bus.add_interface(Bus::routing_interface());
                if !self.store.saved_interfaces.iter().any(|s| s == "routing") {
                    self.store.saved_interfaces.push("routing".into());
                    self.dirty = true;
                }
                self.bus.select(i);
            }
        });
        let mut pick = None;
        let mut remove = None;
        theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt("ifaces")
                .striped(true)
                .sense(Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::initial(240.0).clip(true))
                .column(Column::initial(160.0))
                .column(Column::initial(120.0))
                .column(Column::initial(320.0).clip(true))
                .column(Column::remainder())
                .header(24.0, |mut h| {
                    for t in ["Name", "Type", "Individual address", "Details", ""] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|mut body| {
                    for (i, f) in self.bus.interfaces.iter().enumerate() {
                        body.row(26.0, |mut row| {
                            row.set_selected(i == self.bus.current);
                            row.col(|ui| {
                                ui.label(if f.is_real() { "🔌" } else { "🖥" });
                                ui.label(&f.name);
                            });
                            row.col(|ui| {
                                ui.label(&f.kind);
                            });
                            row.col(|ui| {
                                ui.label(&f.address);
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(&f.detail).color(theme::MUTED));
                            });
                            row.col(|ui| {
                                if f.is_real() && i != self.bus.current && ui.small_button("Remove").clicked() {
                                    remove = Some(i);
                                }
                            });
                            if row.response().clicked() {
                                pick = Some(i);
                            }
                        });
                    }
                });
        });
        if let Some(i) = remove {
            let f = self.bus.interfaces.remove(i);
            if i < self.bus.current {
                self.bus.current -= 1;
            }
            let key = match f.link {
                crate::bus::LinkKind::Tunnel(a) => a.to_string(),
                _ => "routing".into(),
            };
            self.store.saved_interfaces.retain(|s| *s != key);
            self.dirty = true;
        }
        if let Some(i) = pick {
            if i != self.bus.current {
                self.bus.select(i);
                let n = self.bus.iface().name.clone();
                self.say(format!("Selected interface {n}. Press Connect to open it."));
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Tip: secure (encrypted) tunnelling and USB interfaces are not supported yet. On a real bus you can monitor telegrams, send group values and program individual addresses.").small().color(theme::MUTED));
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.add_space(8.0);
        theme::card_frame(ui).show(ui, |ui| {
            ui.set_max_width(560.0);
            theme::section(ui, "Appearance");
            ui.horizontal(|ui| {
                let mut dark = self.store.dark;
                ui.radio_value(&mut dark, false, "Light");
                ui.radio_value(&mut dark, true, "Dark");
                if dark != self.store.dark {
                    self.store.dark = dark;
                    self.theme_applied = false;
                    self.dirty = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Text size");
                let mut z = self.store.zoom;
                if ui.add(egui::Slider::new(&mut z, 0.8..=1.5).step_by(0.05)).changed() {
                    self.store.zoom = z;
                    self.theme_applied = false;
                    self.dirty = true;
                }
            });
            ui.add_space(8.0);
            theme::section(ui, "New projects");
            ui.label("Group address style");
            for s in [GaStyle::ThreeLevel, GaStyle::TwoLevel, GaStyle::Free] {
                if ui.radio_value(&mut self.store.default_ga_style, s, s.label()).changed() {
                    self.dirty = true;
                }
            }
            ui.add_space(8.0);
            theme::section(ui, "Imported products");
            ui.horizontal(|ui| {
                ui.label("Language for product texts");
                self.language_picker(ui);
            });
            ui.label(RichText::new(format!("{} imported product(s) in the library. The language applies to future imports.", self.library.products.len())).small().color(theme::MUTED));
            ui.add_space(8.0);
            self.mcp_settings(ui);
            ui.add_space(8.0);
            theme::section(ui, "Data");
            ui.label(RichText::new(format!("Projects and imported products are saved automatically in {}", data_dir().display())).color(theme::MUTED).small());
            ui.add_space(4.0);
            ui.label("Moving to another computer (Windows, Linux or macOS): make a backup here, copy the file over, and restore it there.");
            ui.horizontal_wrapped(|ui| {
                if ui.button("💾 Back up everything…").on_hover_text("All projects, imported products and download logs in one .zip file").clicked() {
                    self.backup_all();
                }
                if ui.button("⟲ Restore a backup…").on_hover_text("Replace the projects and products on this computer with a backup").clicked() {
                    self.pick_restore();
                }
                if ui.button("📂 Open data folder").clicked() {
                    open_folder(&data_dir());
                }
            });
            ui.add_space(6.0);
            if ui.button("Add sample project").clicked() {
                let p = sample_project(&self.catalog);
                let mut p = p;
                p.id = fresh_id();
                self.ov_sel = Some(p.id);
                self.store.projects.push(p);
                self.dirty = true;
                self.say("Sample project added");
            }
        });
    }

    /// Settings for the built-in MCP server (AI assistants).
    fn mcp_settings(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "AI assistants (MCP server)");
        ui.label("Lets an AI assistant read and change your projects, watch the group monitor and send values. Only programs on this computer can connect. Changes to the open project can be undone.");
        ui.horizontal(|ui| {
            let mut on = self.store.mcp_enabled;
            if ui.checkbox(&mut on, "MCP server on").changed() {
                self.store.mcp_enabled = on;
                self.dirty = true;
                if on {
                    self.mcp.start(self.store.mcp_port);
                } else {
                    self.mcp.stop();
                }
            }
            ui.label("Port");
            if self.mcp_port_text.is_empty() {
                self.mcp_port_text = self.store.mcp_port.to_string();
            }
            ui.add(egui::TextEdit::singleline(&mut self.mcp_port_text).desired_width(60.0));
            let port = self.mcp_port_text.trim().parse::<u16>().ok().filter(|p| *p >= 1024);
            if ui.add_enabled(port.is_some() && port != Some(self.store.mcp_port), egui::Button::new("Apply")).clicked() {
                self.store.mcp_port = port.unwrap();
                self.dirty = true;
                if self.store.mcp_enabled {
                    self.mcp.start(self.store.mcp_port);
                } else {
                    self.mcp.port = self.store.mcp_port;
                }
            }
        });
        let (dot, col) = if self.mcp.running { ("●", theme::OK) } else { ("○", theme::MUTED) };
        ui.horizontal(|ui| {
            ui.label(RichText::new(dot).color(col));
            ui.label(&self.mcp.status);
            if self.mcp.calls > 0 {
                ui.label(RichText::new(format!("· {} call(s), last: {}", self.mcp.calls, self.mcp.last_call)).color(theme::MUTED));
            }
        });
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "buscraft".into());
        let url = format!("http://127.0.0.1:{}/mcp", self.store.mcp_port);
        let mut args = vec!["--mcp-stdio".to_string()];
        if self.store.mcp_port != crate::mcp::DEFAULT_PORT {
            args.extend(["--port".to_string(), self.store.mcp_port.to_string()]);
        }
        let desktop = serde_json::to_string_pretty(&serde_json::json!({"mcpServers": {"buscraft": {"command": exe, "args": args}}})).unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label("Address for assistants that connect by URL:");
            ui.label(RichText::new(&url).monospace());
            if ui.small_button("Copy").clicked() {
                ui.ctx().copy_text(url.clone());
                self.say("MCP address copied");
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("For assistants that start a program (e.g. Claude Desktop), add this to their MCP settings; BusCraft must be running:");
            if ui.small_button("Copy settings").clicked() {
                ui.ctx().copy_text(desktop.clone());
                self.say("MCP settings copied");
            }
        });
        ui.add(egui::Label::new(RichText::new(&desktop).monospace().small().color(theme::MUTED)).wrap());
    }

    pub(crate) fn backup_all(&mut self) {
        if self.shot.is_some() {
            return;
        }
        self.save();
        let day = crate::model::fmt_time(crate::model::now_secs()).split(' ').next().unwrap_or("").to_string();
        let Some(path) = rfd::FileDialog::new().add_filter("BusCraft backup", &["zip"]).set_file_name(format!("BusCraft-backup-{day}.zip")).save_file() else { return };
        match crate::backup::create(&data_dir(), &path) {
            Ok(n) => self.say(format!("Backup saved: {} ({n} files)", path.display())),
            Err(e) => self.say(e),
        }
    }

    pub(crate) fn pick_restore(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("BusCraft backup", &["zip"]).pick_file() else { return };
        match crate::backup::describe(&path) {
            Ok(d) => self.confirm_restore = Some((path, d)),
            Err(e) => self.say(e),
        }
    }

    /// Confirmation for restoring a backup, then the restore and reloading everything.
    fn restore_dialog(&mut self, ctx: &egui::Context) {
        let Some((path, desc)) = self.confirm_restore.clone() else { return };
        let mut close = false;
        let mut go = false;
        egui::Window::new("Restore a backup").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.label(RichText::new(path.display().to_string()).strong());
            ui.label(RichText::new(desc).small().color(theme::MUTED));
            ui.add_space(6.0);
            ui.label("The projects and imported products on this computer are replaced by the ones in the backup. The current state is kept as a safety copy in the data folder (backups).");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Restore").strong()).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if go {
            self.confirm_restore = None;
            self.save();
            match crate::backup::restore(&path, &data_dir()) {
                Ok((n, _)) => {
                    self.reload_data();
                    self.say(format!("Backup restored ({n} files) – {} project(s)", self.store.projects.len()));
                }
                Err(e) => self.say(e),
            }
        } else if close {
            self.confirm_restore = None;
        }
    }

    /// Load projects and the product library again from the data folder.
    fn reload_data(&mut self) {
        self.library = crate::library::Library::load(Some(crate::library::Library::default_dir()));
        let mut cat = catalog();
        cat.extend(self.library.products.iter().cloned());
        self.catalog = cat;
        if let Some(s) = std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str::<Store>(&s).ok()) {
            self.store = s;
        }
        self.screen = Screen::Overview;
        self.ov_sel = self.store.projects.first().map(|p| p.id);
        self.ws = Ws::default();
        self.tabs = vec![Ws::default()];
        self.active_tab = 0;
        self.undo.clear();
        self.redo.clear();
        self.theme_applied = false;
        self.dirty = false;
    }

    /// Catalog browser. Returns the index of a product the user wants to add.
    /// Manufacturers as the tree of a catalog panel.
    pub fn catalog_tree(&mut self, ui: &mut egui::Ui) {
        let mut manus: Vec<String> = self.catalog.iter().map(|p| p.manufacturer.clone()).collect();
        manus.sort();
        manus.dedup();
        egui::ScrollArea::vertical().id_salt(("manus", self.ws.p.id)).auto_shrink([false; 2]).show(ui, |ui| {
            ui.label(RichText::new("Manufacturers").strong());
            if ui.selectable_label(self.ws.p.cat_manu.is_none(), format!("All manufacturers ({})", self.catalog.len())).clicked() {
                self.ws.p.cat_manu = None;
            }
            for m in &manus {
                let n = self.catalog.iter().filter(|p| &p.manufacturer == m).count();
                if ui.selectable_label(self.ws.p.cat_manu.as_ref() == Some(m), format!("{m} ({n})")).clicked() {
                    self.ws.p.cat_manu = Some(m.clone());
                }
            }
        });
    }

    pub fn catalog_browser(&mut self, ui: &mut egui::Ui, can_add: bool, with_manus: bool) -> Option<usize> {
        let mut add = None;
        let mut manus: Vec<String> = self.catalog.iter().map(|p| p.manufacturer.clone()).collect();
        manus.sort();
        manus.dedup();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.ws.p.cat_filter).hint_text("🔍 Search products, order numbers").desired_width(260.0));
            if ui.button("📥 Import products…").on_hover_text("Import manufacturer products from .knxprod files (or the products used in a .knxproj)").clicked() {
                self.pick_and_import(true);
            }
        });
        ui.add_space(4.0);
        let f = self.ws.p.cat_filter.to_lowercase();
        let rows: Vec<usize> = self
            .catalog
            .iter()
            .enumerate()
            .filter(|(_, p)| self.ws.p.cat_manu.as_ref().map(|m| &p.manufacturer == m).unwrap_or(true))
            .filter(|(_, p)| f.is_empty() || p.name.to_lowercase().contains(&f) || p.order_no.to_lowercase().contains(&f) || p.category.to_lowercase().contains(&f))
            .map(|(i, _)| i)
            .collect();
        let mut clicked = None;
        let mut sb = egui_extras::StripBuilder::new(ui);
        if with_manus {
            sb = sb.size(egui_extras::Size::exact(200.0));
        }
        sb.size(egui_extras::Size::remainder().at_least(250.0))
            .size(egui_extras::Size::exact(260.0))
            .horizontal(|mut strip| {
            // Manufacturer list
            if with_manus { strip.cell(|ui| {
                theme::card_frame(ui).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    egui::ScrollArea::vertical().id_salt("manus").show(ui, |ui| {
                        if ui.selectable_label(self.ws.p.cat_manu.is_none(), "All manufacturers").clicked() {
                            self.ws.p.cat_manu = None;
                        }
                        for m in &manus {
                            let n = self.catalog.iter().filter(|p| &p.manufacturer == m).count();
                            if ui.selectable_label(self.ws.p.cat_manu.as_ref() == Some(m), format!("{m} ({n})")).clicked() {
                                self.ws.p.cat_manu = Some(m.clone());
                            }
                        }
                    });
                });
            }); }
            // Product table
            strip.cell(|ui| {
                theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    TableBuilder::new(ui)
                        .id_salt("catalog")
                        .striped(true)
                        .resizable(true)
                        .sense(Sense::click())
                        .cell_layout(Layout::left_to_right(Align::Center))
                        .column(Column::initial(230.0).clip(true))
                        .column(Column::initial(95.0).clip(true))
                        .column(Column::initial(125.0).clip(true))
                        .column(Column::initial(100.0).clip(true))
                        .column(Column::remainder().at_least(60.0).clip(true))
                        .header(22.0, |mut h| {
                            for t in ["Product", "Order number", "Manufacturer", "Category", "Application"] {
                                h.col(|ui| {
                                    ui.strong(t);
                                });
                            }
                        })
                        .body(|mut body| {
                            for &i in &rows {
                                let p = &self.catalog[i];
                                body.row(24.0, |mut row| {
                                    row.set_selected(self.ws.p.cat_sel == Some(i));
                                    row.col(|ui| {
                                        ui.label(&p.name);
                                    });
                                    row.col(|ui| {
                                        ui.label(&p.order_no);
                                    });
                                    row.col(|ui| {
                                        ui.label(&p.manufacturer);
                                    });
                                    row.col(|ui| {
                                        ui.label(&p.category);
                                    });
                                    row.col(|ui| {
                                        ui.label(if p.app_program.is_empty() { "–" } else { &p.app_program });
                                    });
                                    let r = row.response();
                                    if r.clicked() {
                                        clicked = Some(i);
                                    }
                                    if r.double_clicked() && can_add {
                                        add = Some(i);
                                    }
                                });
                            }
                        });
                });
            });
            // Product details
            strip.cell(|ui| {
                theme::card_frame(ui).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    if let Some(p) = self.ws.p.cat_sel.and_then(|i| self.catalog.get(i)) {
                        ui.label(RichText::new(&p.name).strong());
                        ui.label(RichText::new(&p.manufacturer).color(theme::MUTED));
                        ui.add_space(4.0);
                        ui.label(&p.description);
                        ui.add_space(4.0);
                        egui::Grid::new("catd").num_columns(2).show(ui, |ui| {
                            ui.label("Order number");
                            ui.label(&p.order_no);
                            ui.end_row();
                            ui.label("Medium");
                            ui.label(p.medium.short());
                            ui.end_row();
                            ui.label("Group objects");
                            ui.label(p.objects.len().to_string());
                            ui.end_row();
                            ui.label("Parameters");
                            ui.label(p.params.len().to_string());
                            ui.end_row();
                        });
                        if can_add {
                            ui.add_space(6.0);
                            if ui.add(egui::Button::new(RichText::new("Add to project").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                                add = self.ws.p.cat_sel;
                            }
                        }
                    } else {
                        ui.label(RichText::new("Select a product to see details.").color(theme::MUTED));
                    }
                });
            });
        });
        if let Some(i) = clicked {
            self.ws.p.cat_sel = Some(i);
        }
        add
    }

    // ------------------------------------------------------------------ files

    fn export_project(&mut self) {
        let Some(i) = self.ov_sel.and_then(|id| self.project_index(id)) else { return };
        let p = &self.store.projects[i];
        let Some(path) = rfd::FileDialog::new().add_filter("BusCraft project", &["bcproj"]).set_file_name(format!("{}.bcproj", p.name)).save_file() else { return };
        match serde_json::to_string_pretty(p).map_err(|e| e.to_string()).and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string())) {
            Ok(()) => self.say(format!("Exported to {}", path.display())),
            Err(e) => self.say(format!("Export failed: {e}")),
        }
    }

    // ------------------------------------------------------------------ dialogs

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(np) = &mut self.new_project {
            let mut create = false;
            let mut cancel = false;
            egui::Window::new("New project").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).default_width(380.0).show(ctx, |ui| {
                egui::Grid::new("np").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut np.name);
                    ui.end_row();
                    ui.label("Backbone");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut np.backbone, Medium::IP, "IP");
                        ui.radio_value(&mut np.backbone, Medium::TP, "TP");
                    });
                    ui.end_row();
                    ui.label("First line medium");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut np.line, Medium::TP, "TP");
                        ui.radio_value(&mut np.line, Medium::IP, "IP");
                        ui.radio_value(&mut np.line, Medium::RF, "RF");
                    });
                    ui.end_row();
                    ui.label("Group address style");
                    ui.vertical(|ui| {
                        for s in [GaStyle::ThreeLevel, GaStyle::TwoLevel, GaStyle::Free] {
                            ui.radio_value(&mut np.style, s, s.label());
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut np.sample, "Fill with sample content");
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!np.name.trim().is_empty(), egui::Button::new(RichText::new("Create project").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                        create = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
            if create {
                let np = self.new_project.take().unwrap();
                let mut p = if np.sample {
                    let mut s = sample_project(&self.catalog);
                    s.name = np.name.trim().to_string();
                    s.status = ProjectStatus::Planning;
                    s.ga_style = np.style;
                    s
                } else {
                    Project::new(np.name.trim(), np.backbone, np.line, np.style)
                };
                p.id = fresh_id();
                let id = p.id;
                self.store.projects.insert(0, p);
                self.ov_sel = Some(id);
                self.dirty = true;
                self.open_project(id);
            } else if cancel {
                self.new_project = None;
            }
        }

        if let Some(id) = self.confirm_delete {
            let name = self.store.projects.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
            let mut close = false;
            egui::Window::new("Delete project").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("Delete the project \"{name}\"? This cannot be undone."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(theme::BAD)).clicked() {
                        self.store.projects.retain(|p| p.id != id);
                        self.ov_sel = self.store.projects.first().map(|p| p.id);
                        self.dirty = true;
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.confirm_delete = None;
                if !self.store.projects.iter().any(|p| p.id == id) {
                    self.say(format!("Deleted project \"{name}\""));
                }
            }
        }

        if self.about {
            let mut open = true;
            egui::Window::new("About BusCraft").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(RichText::new("BusCraft 0.2").strong());
                ui.label("Plan, configure, program and monitor building automation installations.");
                ui.add_space(4.0);
                ui.label(RichText::new("Bus access is simulated in this version.").color(theme::MUTED));
            });
            self.about = open;
        }
    }
}

/// Show a folder in the system's file manager.
fn open_folder(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(dir).spawn();
}