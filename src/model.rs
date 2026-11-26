//! Project data model: buildings, topology, devices, group addresses and a small sample catalog.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- time helpers

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A unique project id (time in microseconds plus a running counter).
pub fn fresh_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    micros.wrapping_mul(16) + (COUNTER.fetch_add(1, Ordering::SeqCst) % 16)
}

/// Format seconds since 1970 (UTC) as "YYYY-MM-DD hh:mm".
pub fn fmt_time(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------- buildings

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum SpaceKind {
    Building,
    BuildingPart,
    Floor,
    Room,
    Corridor,
    Stairway,
    DistributionBoard,
}

impl SpaceKind {
    pub fn label(self) -> &'static str {
        match self {
            SpaceKind::Building => "Building",
            SpaceKind::BuildingPart => "Building Part",
            SpaceKind::Floor => "Floor",
            SpaceKind::Room => "Room",
            SpaceKind::Corridor => "Corridor",
            SpaceKind::Stairway => "Stairway",
            SpaceKind::DistributionBoard => "Distribution Board",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            SpaceKind::Building => "🏢",
            SpaceKind::BuildingPart => "🏬",
            SpaceKind::Floor => "▤",
            SpaceKind::Room => "◻",
            SpaceKind::Corridor => "↔",
            SpaceKind::Stairway => "↗",
            SpaceKind::DistributionBoard => "⚡",
        }
    }
    /// What can be added below a space of this kind.
    pub fn child_kinds(self) -> &'static [SpaceKind] {
        use SpaceKind::*;
        match self {
            Building | BuildingPart => &[BuildingPart, Floor, Room, Corridor, Stairway, DistributionBoard],
            Floor => &[Room, Corridor, Stairway, DistributionBoard],
            Room | Corridor | Stairway => &[DistributionBoard],
            DistributionBoard => &[],
        }
    }
    pub fn holds_devices(self) -> bool {
        !matches!(self, SpaceKind::Building | SpaceKind::BuildingPart | SpaceKind::Floor)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Space {
    pub id: u32,
    pub name: String,
    pub kind: SpaceKind,
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub children: Vec<Space>,
    #[serde(default)]
    pub devices: Vec<u32>,
}

impl Space {
    pub fn new(id: u32, name: &str, kind: SpaceKind) -> Self {
        Space { id, name: name.into(), kind, number: String::new(), comment: String::new(), children: vec![], devices: vec![] }
    }
    pub fn find(&self, id: u32) -> Option<&Space> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }
    pub fn find_mut(&mut self, id: u32) -> Option<&mut Space> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(id))
    }
    /// Remove the device from this space and all below.
    pub fn unassign(&mut self, dev: u32) {
        self.devices.retain(|d| *d != dev);
        for c in &mut self.children {
            c.unassign(dev);
        }
    }
    pub fn space_of_device(&self, dev: u32) -> Option<&Space> {
        if self.devices.contains(&dev) {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.space_of_device(dev))
    }
    /// All devices in this space and below.
    pub fn all_devices(&self, out: &mut Vec<u32>) {
        out.extend(self.devices.iter().copied());
        for c in &self.children {
            c.all_devices(out);
        }
    }
}

// ---------------------------------------------------------------- topology

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Medium {
    TP,
    IP,
    RF,
}
impl Medium {
    pub fn label(self) -> &'static str {
        match self {
            Medium::TP => "TP (twisted pair)",
            Medium::IP => "IP",
            Medium::RF => "RF (radio)",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Medium::TP => "TP",
            Medium::IP => "IP",
            Medium::RF => "RF",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Line {
    pub number: u8,
    pub name: String,
    pub medium: Medium,
    #[serde(default)]
    pub comment: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Area {
    pub number: u8,
    pub name: String,
    #[serde(default)]
    pub comment: String,
    pub lines: Vec<Line>,
}

// ---------------------------------------------------------------- devices

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum ParamKind {
    Choice(Vec<String>),
    Number { min: i64, max: i64, unit: String },
    Flag,
    Text,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Param {
    pub page: String,
    pub name: String,
    pub kind: ParamKind,
    pub value: String,
    #[serde(default)]
    pub default: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub struct Flags {
    pub c: bool,
    pub r: bool,
    pub w: bool,
    pub t: bool,
    pub u: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct GroupObject {
    pub number: u16,
    pub name: String,
    pub function: String,
    #[serde(default)]
    pub description: String,
    pub dpt: String,
    pub flags: Flags,
    #[serde(default)]
    pub priority: String,
    /// Linked group address ids; the first one is the sending address.
    #[serde(default)]
    pub links: Vec<u32>,
    /// Reference id in an imported application program (empty for built-in products).
    #[serde(default)]
    pub ref_id: String,
    /// Object size as given by the product (e.g. "1 bit"); empty = derived from the data type.
    #[serde(default)]
    pub size: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Device {
    pub id: u32,
    pub area: u8,
    pub line: u8,
    pub number: u8,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub comment: String,
    pub manufacturer: String,
    pub product: String,
    pub order_no: String,
    pub app_program: String,
    #[serde(default)]
    pub serial: String,
    pub params: Vec<Param>,
    pub objects: Vec<GroupObject>,
    /// Commissioning state: Adr / Prg / Par / Grp / Cfg
    #[serde(default)]
    pub adr_ok: bool,
    #[serde(default)]
    pub prg_ok: bool,
    #[serde(default)]
    pub par_ok: bool,
    #[serde(default)]
    pub grp_ok: bool,
    #[serde(default)]
    pub cfg_ok: bool,
    /// Imported application program id (parameters and objects come from the library).
    #[serde(default)]
    pub knx_app: Option<String>,
    /// Parameter values of an imported application, by parameter reference id.
    #[serde(default)]
    pub knx_values: std::collections::BTreeMap<String, String>,
}

impl Device {
    /// Individual address. Devices without an application (e.g. power supplies) have none.
    pub fn address(&self) -> String {
        if self.has_app() {
            format!("{}.{}.{}", self.area, self.line, self.number)
        } else {
            format!("{}.{}.-", self.area, self.line)
        }
    }
    pub fn has_app(&self) -> bool {
        !self.app_program.is_empty()
    }
    pub fn needs_download(&self) -> bool {
        if !self.has_app() {
            return false;
        }
        !(self.adr_ok && self.prg_ok && self.par_ok && self.grp_ok && self.cfg_ok)
    }
    pub fn status_text(&self) -> &'static str {
        if !self.has_app() {
            "No application"
        } else if !self.adr_ok {
            "Address not programmed"
        } else if self.needs_download() {
            "Download required"
        } else {
            "Up to date"
        }
    }
}

// ---------------------------------------------------------------- group addresses

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum GaStyle {
    ThreeLevel,
    TwoLevel,
    Free,
}
impl GaStyle {
    pub fn label(self) -> &'static str {
        match self {
            GaStyle::ThreeLevel => "3-level (main/middle/sub)",
            GaStyle::TwoLevel => "2-level (main/sub)",
            GaStyle::Free => "Free (number)",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MiddleGroup {
    pub number: u8,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MainGroup {
    pub number: u8,
    pub name: String,
    pub middles: Vec<MiddleGroup>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct GroupAddress {
    pub id: u32,
    pub main: u8,
    pub middle: u8,
    pub sub: u8,
    pub name: String,
    #[serde(default)]
    pub dpt: String,
    #[serde(default)]
    pub central: bool,
    #[serde(default)]
    pub pass_through: bool,
    #[serde(default)]
    pub comment: String,
}

impl GroupAddress {
    pub fn raw(&self) -> u16 {
        ((self.main as u16 & 0x1f) << 11) | ((self.middle as u16 & 0x7) << 8) | self.sub as u16
    }
    pub fn fmt(&self, style: GaStyle) -> String {
        match style {
            GaStyle::ThreeLevel => format!("{}/{}/{}", self.main, self.middle, self.sub),
            GaStyle::TwoLevel => format!("{}/{}", self.main, self.raw() & 0x7ff),
            GaStyle::Free => format!("{}", self.raw()),
        }
    }
}

/// Common data point types offered in pickers.
pub const DPTS: &[&str] = &[
    "1.001 switch",
    "1.005 alarm",
    "1.007 step",
    "1.008 up/down",
    "1.009 open/close",
    "3.007 dimming control",
    "5.001 percentage (0..100%)",
    "5.010 counter pulses",
    "9.001 temperature (°C)",
    "9.004 lux (Lux)",
    "9.005 speed (m/s)",
    "10.001 time of day",
    "11.001 date",
    "14.056 power (W)",
    "16.000 character string",
    "19.001 date time",
    "20.102 HVAC mode",
];

pub fn dpt_size(dpt: &str) -> &'static str {
    match dpt.split('.').next().unwrap_or("") {
        "1" => "1 bit",
        "3" => "4 bit",
        "5" | "20" => "1 byte",
        "9" => "2 bytes",
        "10" | "11" => "3 bytes",
        "19" => "8 bytes",
        "14" => "4 bytes",
        "16" => "14 bytes",
        _ => "",
    }
}

// ---------------------------------------------------------------- project

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum ProjectStatus {
    Unknown,
    Planning,
    Design,
    Commissioning,
    Completed,
}
impl ProjectStatus {
    pub const ALL: [ProjectStatus; 5] = [
        ProjectStatus::Unknown,
        ProjectStatus::Planning,
        ProjectStatus::Design,
        ProjectStatus::Commissioning,
        ProjectStatus::Completed,
    ];
    pub fn label(self) -> &'static str {
        match self {
            ProjectStatus::Unknown => "Unknown",
            ProjectStatus::Planning => "Planning",
            ProjectStatus::Design => "Design",
            ProjectStatus::Commissioning => "Commissioning",
            ProjectStatus::Completed => "Completed",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Project {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub contract: String,
    #[serde(default)]
    pub comment: String,
    pub created: u64,
    pub modified: u64,
    pub status: ProjectStatus,
    pub ga_style: GaStyle,
    pub buildings: Vec<Space>,
    pub areas: Vec<Area>,
    pub main_groups: Vec<MainGroup>,
    pub addresses: Vec<GroupAddress>,
    pub devices: Vec<Device>,
    pub next_id: u32,
}

impl Project {
    pub fn new(name: &str, backbone: Medium, line_medium: Medium, ga_style: GaStyle) -> Self {
        let t = now_secs();
        let mut p = Project {
            id: fresh_id(),
            name: name.into(),
            number: String::new(),
            contract: String::new(),
            comment: String::new(),
            created: t,
            modified: t,
            status: ProjectStatus::Planning,
            ga_style,
            buildings: vec![],
            areas: vec![Area {
                number: 1,
                name: "New area".into(),
                comment: String::new(),
                lines: vec![
                    Line { number: 0, name: "Main line".into(), medium: backbone, comment: String::new() },
                    Line { number: 1, name: "New line".into(), medium: line_medium, comment: String::new() },
                ],
            }],
            main_groups: vec![MainGroup { number: 0, name: "New main group".into(), middles: vec![MiddleGroup { number: 0, name: "New middle group".into() }] }],
            addresses: vec![],
            devices: vec![],
            next_id: 1,
        };
        let id = p.alloc_id();
        p.buildings.push(Space::new(id, name, SpaceKind::Building));
        p
    }

    pub fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn touch(&mut self) {
        self.modified = now_secs();
    }

    pub fn device(&self, id: u32) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }
    pub fn device_mut(&mut self, id: u32) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.id == id)
    }
    pub fn ga(&self, id: u32) -> Option<&GroupAddress> {
        self.addresses.iter().find(|g| g.id == id)
    }
    /// The data type used to show a group address's values: its own, or else the type of a
    /// group object linked to it (so values still decode when the address has no type set).
    pub fn value_dpt(&self, g: &GroupAddress) -> String {
        if !g.dpt.is_empty() {
            return g.dpt.clone();
        }
        self.devices
            .iter()
            .flat_map(|d| d.objects.iter())
            .find(|o| !o.dpt.is_empty() && o.links.contains(&g.id))
            .map(|o| o.dpt.clone())
            .unwrap_or_default()
    }
    pub fn ga_mut(&mut self, id: u32) -> Option<&mut GroupAddress> {
        self.addresses.iter_mut().find(|g| g.id == id)
    }
    pub fn ga_text(&self, id: u32) -> String {
        self.ga(id).map(|g| g.fmt(self.ga_style)).unwrap_or_else(|| "?".into())
    }
    pub fn space(&self, id: u32) -> Option<&Space> {
        self.buildings.iter().find_map(|b| b.find(id))
    }
    pub fn space_mut(&mut self, id: u32) -> Option<&mut Space> {
        self.buildings.iter_mut().find_map(|b| b.find_mut(id))
    }
    pub fn room_of(&self, dev: u32) -> Option<&Space> {
        self.buildings.iter().find_map(|b| b.space_of_device(dev))
    }
    pub fn room_name(&self, dev: u32) -> String {
        self.room_of(dev).map(|s| s.name.clone()).unwrap_or_default()
    }
    pub fn line(&self, area: u8, line: u8) -> Option<&Line> {
        self.areas.iter().find(|a| a.number == area)?.lines.iter().find(|l| l.number == line)
    }
    pub fn line_mut(&mut self, area: u8, line: u8) -> Option<&mut Line> {
        self.areas.iter_mut().find(|a| a.number == area)?.lines.iter_mut().find(|l| l.number == line)
    }

    /// Next free device number on a line (1..255).
    pub fn free_device_number(&self, area: u8, line: u8) -> u8 {
        (1..=255u8)
            .find(|n| !self.devices.iter().any(|d| d.has_app() && d.area == area && d.line == line && d.number == *n))
            .unwrap_or(255)
    }
    pub fn address_taken(&self, area: u8, line: u8, number: u8, except: u32) -> bool {
        self.devices.iter().any(|d| d.id != except && d.has_app() && d.area == area && d.line == line && d.number == number)
    }

    pub fn free_sub(&self, main: u8, middle: u8) -> u8 {
        (1..=255u8)
            .find(|s| !self.addresses.iter().any(|g| g.main == main && g.middle == middle && g.sub == *s))
            .unwrap_or(255)
    }

    /// Group objects linked to a group address: (device id, object index).
    pub fn links_of(&self, ga: u32) -> Vec<(u32, usize)> {
        let mut out = vec![];
        for d in &self.devices {
            for (i, o) in d.objects.iter().enumerate() {
                if o.links.contains(&ga) {
                    out.push((d.id, i));
                }
            }
        }
        out
    }

    pub fn remove_device(&mut self, id: u32) {
        self.devices.retain(|d| d.id != id);
        for b in &mut self.buildings {
            b.unassign(id);
        }
    }

    pub fn remove_ga(&mut self, id: u32) {
        self.addresses.retain(|g| g.id != id);
        for d in &mut self.devices {
            let mut touched = false;
            for o in &mut d.objects {
                let before = o.links.len();
                o.links.retain(|l| *l != id);
                touched |= before != o.links.len();
            }
            if touched {
                d.grp_ok = false;
            }
        }
    }

    pub fn add_device_from(&mut self, prod: &Product, area: u8, line: u8) -> u32 {
        let id = self.alloc_id();
        let number = self.free_device_number(area, line);
        let dev = Device {
            id,
            area,
            line,
            number,
            name: prod.name.clone(),
            description: String::new(),
            comment: String::new(),
            manufacturer: prod.manufacturer.clone(),
            product: prod.name.clone(),
            order_no: prod.order_no.clone(),
            app_program: prod.app_program.clone(),
            serial: String::new(),
            params: prod.params.clone(),
            objects: prod.objects.clone(),
            adr_ok: false,
            prg_ok: false,
            par_ok: false,
            grp_ok: false,
            cfg_ok: false,
            knx_app: prod.app_id.clone(),
            knx_values: Default::default(),
        };
        self.devices.push(dev);
        id
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }
}

// ---------------------------------------------------------------- catalog

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Product {
    pub manufacturer: String,
    pub name: String,
    pub order_no: String,
    pub app_program: String,
    pub medium: Medium,
    pub category: String,
    pub description: String,
    pub params: Vec<Param>,
    pub objects: Vec<GroupObject>,
    /// Imported application program id.
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub imported: bool,
}

fn choice(page: &str, name: &str, opts: &[&str], val: &str) -> Param {
    Param {
        page: page.into(),
        name: name.into(),
        kind: ParamKind::Choice(opts.iter().map(|s| s.to_string()).collect()),
        value: val.into(),
        default: val.into(),
    }
}
fn number(page: &str, name: &str, min: i64, max: i64, unit: &str, val: i64) -> Param {
    Param {
        page: page.into(),
        name: name.into(),
        kind: ParamKind::Number { min, max, unit: unit.into() },
        value: val.to_string(),
        default: val.to_string(),
    }
}
fn flag(page: &str, name: &str, val: bool) -> Param {
    let v = if val { "1" } else { "0" };
    Param { page: page.into(), name: name.into(), kind: ParamKind::Flag, value: v.into(), default: v.into() }
}
fn text(page: &str, name: &str, val: &str) -> Param {
    Param { page: page.into(), name: name.into(), kind: ParamKind::Text, value: val.into(), default: val.into() }
}

const CWU: Flags = Flags { c: true, r: false, w: true, t: false, u: true };
const CW: Flags = Flags { c: true, r: false, w: true, t: false, u: false };
const CRT: Flags = Flags { c: true, r: true, w: false, t: true, u: false };
const CT: Flags = Flags { c: true, r: false, w: false, t: true, u: false };
const CWTU: Flags = Flags { c: true, r: false, w: true, t: true, u: true };

fn obj(number: u16, name: &str, function: &str, dpt: &str, flags: Flags) -> GroupObject {
    GroupObject {
        number,
        name: name.into(),
        function: function.into(),
        description: String::new(),
        dpt: dpt.into(),
        flags,
        priority: "Low".into(),
        links: vec![],
        ref_id: String::new(),
        size: String::new(),
    }
}

const CH: [&str; 8] = ["A", "B", "C", "D", "E", "F", "G", "H"];

pub fn catalog() -> Vec<Product> {
    let mut out = vec![];

    // Switch actuator
    {
        let mut params = vec![
            number("General", "Delay after bus voltage recovery", 0, 255, "s", 2),
            choice("General", "Cyclic \"in operation\" telegram", &["Disabled", "Every 1 min", "Every 15 min", "Every hour"], "Disabled"),
            choice("General", "Behaviour on bus voltage failure", &["Unchanged", "Off", "On"], "Unchanged"),
        ];
        let mut objects = vec![];
        for (i, c) in CH.iter().take(4).enumerate() {
            let page = format!("Channel {c}");
            params.push(choice(&page, "Operating mode", &["Normally open", "Normally closed"], "Normally open"));
            params.push(number(&page, "Switch-on delay", 0, 3600, "s", 0));
            params.push(number(&page, "Switch-off delay", 0, 3600, "s", 0));
            params.push(flag(&page, "Staircase light function", false));
            params.push(flag(&page, "Status object", true));
            objects.push(obj((i * 2) as u16, &format!("Channel {c}"), "Switching", "1.001 switch", CWU));
            objects.push(obj((i * 2 + 1) as u16, &format!("Channel {c}"), "Status switching", "1.001 switch", CRT));
        }
        out.push(Product { app_id: None, imported: false,
            manufacturer: "Northwind Controls".into(),
            name: "Switch actuator 4-fold, 16 A".into(),
            order_no: "NW-SA4.16".into(),
            app_program: "Switch 4f 1.2".into(),
            medium: Medium::TP,
            category: "Actuators".into(),
            description: "DIN rail switch actuator with four 16 A relay outputs.".into(),
            params,
            objects,
        });
    }
    // Dimming actuator
    {
        let mut params = vec![number("General", "Delay after bus voltage recovery", 0, 255, "s", 2)];
        let mut objects = vec![];
        for (i, c) in CH.iter().take(2).enumerate() {
            let page = format!("Channel {c}");
            params.push(choice(&page, "Load type", &["Automatic", "LED", "Incandescent", "Halogen"], "Automatic"));
            params.push(number(&page, "Minimum brightness", 1, 50, "%", 5));
            params.push(number(&page, "Maximum brightness", 50, 100, "%", 100));
            params.push(number(&page, "Dimming time 0-100%", 1, 60, "s", 5));
            params.push(number(&page, "Switch-on brightness", 1, 100, "%", 100));
            let b = (i * 5) as u16;
            objects.push(obj(b, &format!("Channel {c}"), "Switching", "1.001 switch", CWU));
            objects.push(obj(b + 1, &format!("Channel {c}"), "Relative dimming", "3.007 dimming control", CW));
            objects.push(obj(b + 2, &format!("Channel {c}"), "Brightness value", "5.001 percentage (0..100%)", CWU));
            objects.push(obj(b + 3, &format!("Channel {c}"), "Status switching", "1.001 switch", CRT));
            objects.push(obj(b + 4, &format!("Channel {c}"), "Status brightness value", "5.001 percentage (0..100%)", CRT));
        }
        out.push(Product { app_id: None, imported: false,
            manufacturer: "Lumen & Co".into(),
            name: "Universal dimming actuator 2-fold, 300 W".into(),
            order_no: "LC-UD2.300".into(),
            app_program: "Dimmer 2f 2.0".into(),
            medium: Medium::TP,
            category: "Actuators".into(),
            description: "Universal dimmer for LED and incandescent loads.".into(),
            params,
            objects,
        });
    }
    // Blind actuator
    {
        let mut params = vec![choice("General", "Blind reaction on bus voltage recovery", &["No reaction", "Up", "Down"], "No reaction")];
        let mut objects = vec![];
        for (i, c) in CH.iter().take(4).enumerate() {
            let page = format!("Channel {c}");
            params.push(choice(&page, "Type", &["Blind with slats", "Roller shutter", "Awning"], "Blind with slats"));
            params.push(number(&page, "Travel time up", 1, 600, "s", 45));
            params.push(number(&page, "Travel time down", 1, 600, "s", 42));
            params.push(flag(&page, "Wind alarm", false));
            let b = (i * 4) as u16;
            objects.push(obj(b, &format!("Channel {c}"), "Up/Down", "1.008 up/down", CW));
            objects.push(obj(b + 1, &format!("Channel {c}"), "Stop/Step", "1.007 step", CW));
            objects.push(obj(b + 2, &format!("Channel {c}"), "Position", "5.001 percentage (0..100%)", CW));
            objects.push(obj(b + 3, &format!("Channel {c}"), "Status position", "5.001 percentage (0..100%)", CRT));
        }
        out.push(Product { app_id: None, imported: false,
            manufacturer: "Aurel Elektro".into(),
            name: "Blind actuator 4-fold, 230 V".into(),
            order_no: "AE-JA4.230".into(),
            app_program: "Blind 4f 1.1".into(),
            medium: Medium::TP,
            category: "Actuators".into(),
            description: "Shutter and blind actuator with travel time detection.".into(),
            params,
            objects,
        });
    }
    // Heating actuator
    {
        let mut params = vec![choice("General", "Valve type", &["Normally closed", "Normally open"], "Normally closed")];
        let mut objects = vec![];
        for (i, c) in CH.iter().take(6).enumerate() {
            let page = format!("Channel {c}");
            params.push(choice(&page, "Control value type", &["1 bit (PWM)", "1 byte (continuous)"], "1 byte (continuous)"));
            params.push(number(&page, "PWM cycle time", 1, 60, "min", 15));
            objects.push(obj((i * 2) as u16, &format!("Channel {c}"), "Control value", "5.001 percentage (0..100%)", CW));
            objects.push(obj((i * 2 + 1) as u16, &format!("Channel {c}"), "Status valve", "1.001 switch", CRT));
        }
        out.push(Product { app_id: None, imported: false,
            manufacturer: "Northwind Controls".into(),
            name: "Heating actuator 6-fold".into(),
            order_no: "NW-HA6".into(),
            app_program: "Heating 6f 1.0".into(),
            medium: Medium::TP,
            category: "Actuators".into(),
            description: "Electrothermal valve drive actuator, 6 outputs.".into(),
            params,
            objects,
        });
    }
    // Push button
    {
        let mut params = vec![
            choice("General", "LED brightness", &["Off", "Low", "Medium", "High"], "Medium"),
            flag("General", "Long press for dimming/blinds", true),
        ];
        let mut objects = vec![];
        for i in 0..4u16 {
            let page = format!("Button {}", i + 1);
            params.push(choice(&page, "Function", &["Switch", "Toggle", "Dimming", "Blind", "Value 1 byte", "Scene"], "Toggle"));
            params.push(choice(&page, "LED status", &["Off", "On", "Follows object", "Inverted object"], "Follows object"));
            objects.push(obj(i * 2, &format!("Button {}", i + 1), "Switching", "1.001 switch", CWTU));
            objects.push(obj(i * 2 + 1, &format!("Button {}", i + 1), "Dimming / Stop", "3.007 dimming control", CT));
        }
        out.push(Product { app_id: None, imported: false,
            manufacturer: "Sentra Systems".into(),
            name: "Push button 4-fold with status LED".into(),
            order_no: "SS-PB4".into(),
            app_program: "Button 4f 3.1".into(),
            medium: Medium::TP,
            category: "Sensors".into(),
            description: "Flush-mounted push button with four rockers.".into(),
            params,
            objects,
        });
    }
    // Binary input
    {
        let mut params = vec![number("General", "Debounce time", 10, 255, "ms", 30)];
        let mut objects = vec![];
        for (i, c) in CH.iter().take(4).enumerate() {
            let page = format!("Input {c}");
            params.push(choice(&page, "Function", &["Switch", "Dimming", "Blind", "Counter", "Disabled"], "Switch"));
            params.push(choice(&page, "Rising edge", &["On", "Off", "Toggle", "No reaction"], "On"));
            params.push(choice(&page, "Falling edge", &["On", "Off", "Toggle", "No reaction"], "Off"));
            objects.push(obj(i as u16, &format!("Input {c}"), "Switching", "1.001 switch", CWTU));
        }
        out.push(Product { app_id: None, imported: false,
            manufacturer: "Aurel Elektro".into(),
            name: "Binary input 4-fold, 230 V".into(),
            order_no: "AE-BE4.230".into(),
            app_program: "Binary input 4f 1.0".into(),
            medium: Medium::TP,
            category: "Sensors".into(),
            description: "Four potential-free inputs for contacts.".into(),
            params,
            objects,
        });
    }
    // Presence detector
    out.push(Product { app_id: None, imported: false,
        manufacturer: "Sentra Systems".into(),
        name: "Presence detector 360°".into(),
        order_no: "SS-PD360".into(),
        app_program: "Presence 2.4".into(),
        medium: Medium::TP,
        category: "Sensors".into(),
        description: "Ceiling presence detector with light sensor.".into(),
        params: vec![
            number("General", "Sensitivity", 1, 10, "", 7),
            number("Light channel", "Brightness threshold", 10, 2000, "lux", 300),
            number("Light channel", "Follow-up time", 10, 3600, "s", 300),
            flag("Light channel", "Semi-automatic mode", false),
            number("Brightness", "Send on change of", 5, 500, "lux", 50),
        ],
        objects: vec![
            obj(0, "Light channel", "Switching", "1.001 switch", CT),
            obj(1, "Light channel", "Lock", "1.001 switch", CW),
            obj(2, "Brightness", "Lux value", "9.004 lux (Lux)", CRT),
            obj(3, "Presence", "Presence output", "1.001 switch", CRT),
        ],
    });
    // Room thermostat
    out.push(Product { app_id: None, imported: false,
        manufacturer: "Northwind Controls".into(),
        name: "Room temperature controller with display".into(),
        order_no: "NW-RTC1".into(),
        app_program: "Room controller 1.3".into(),
        medium: Medium::TP,
        category: "HVAC".into(),
        description: "Room controller with display and internal temperature sensor.".into(),
        params: vec![
            choice("General", "Controller type", &["Heating", "Cooling", "Heating and cooling"], "Heating"),
            number("Setpoints", "Comfort setpoint", 16, 28, "°C", 21),
            number("Setpoints", "Standby reduction", 1, 6, "K", 2),
            number("Setpoints", "Night reduction", 1, 8, "K", 4),
            number("Sensor", "Send actual temperature on change of", 1, 20, "x0.1 K", 3),
        ],
        objects: vec![
            obj(0, "Controller", "Actual temperature", "9.001 temperature (°C)", CRT),
            obj(1, "Controller", "Setpoint", "9.001 temperature (°C)", CWTU),
            obj(2, "Controller", "Heating control value", "5.001 percentage (0..100%)", CRT),
            obj(3, "Controller", "Operating mode", "20.102 HVAC mode", CWU),
        ],
    });
    // Weather station
    out.push(Product { app_id: None, imported: false,
        manufacturer: "Lumen & Co".into(),
        name: "Weather station basic".into(),
        order_no: "LC-WS1".into(),
        app_program: "Weather 1.0".into(),
        medium: Medium::TP,
        category: "Sensors".into(),
        description: "Measures wind, rain, brightness and outdoor temperature.".into(),
        params: vec![
            number("Wind", "Wind alarm threshold", 2, 30, "m/s", 10),
            number("Brightness", "Send cyclically", 0, 60, "min", 5),
        ],
        objects: vec![
            obj(0, "Wind", "Wind speed", "9.005 speed (m/s)", CRT),
            obj(1, "Wind", "Wind alarm", "1.005 alarm", CRT),
            obj(2, "Rain", "Rain alarm", "1.005 alarm", CRT),
            obj(3, "Brightness", "Lux value", "9.004 lux (Lux)", CRT),
            obj(4, "Temperature", "Outdoor temperature", "9.001 temperature (°C)", CRT),
            obj(5, "Clock", "Time of day", "10.001 time of day", CRT),
            obj(6, "Clock", "Date", "11.001 date", CRT),
        ],
    });
    // System devices
    out.push(Product { app_id: None, imported: false,
        manufacturer: "Northwind Controls".into(),
        name: "Power supply 640 mA with choke".into(),
        order_no: "NW-PS640".into(),
        app_program: String::new(),
        medium: Medium::TP,
        category: "System devices".into(),
        description: "Bus power supply. Has no application program.".into(),
        params: vec![],
        objects: vec![],
    });
    out.push(Product { app_id: None, imported: false,
        manufacturer: "Sentra Systems".into(),
        name: "IP router".into(),
        order_no: "SS-IPR".into(),
        app_program: "IP Router 1.0".into(),
        medium: Medium::IP,
        category: "System devices".into(),
        description: "Couples a TP line to the IP backbone.".into(),
        params: vec![
            flag("IP settings", "DHCP", true),
            text("IP settings", "IP address", "192.168.1.20"),
            text("IP settings", "Routing multicast address", "224.0.23.12"),
            choice("Routing", "Group telegrams main line → line", &["Filter", "Route", "Block"], "Filter"),
        ],
        objects: vec![],
    });
    out.push(Product { app_id: None, imported: false,
        manufacturer: "Aurel Elektro".into(),
        name: "Line coupler".into(),
        order_no: "AE-LK1".into(),
        app_program: "Coupler 1.1".into(),
        medium: Medium::TP,
        category: "System devices".into(),
        description: "Couples a line to the main line.".into(),
        params: vec![
            choice("Routing", "Group telegrams", &["Filter", "Route", "Block"], "Filter"),
            choice("Routing", "Physical telegrams", &["Filter", "Route", "Block"], "Filter"),
        ],
        objects: vec![],
    });
    out
}

// ---------------------------------------------------------------- sample project

pub fn sample_project(cat: &[Product]) -> Project {
    let mut p = Project::new("Sample House", Medium::IP, Medium::TP, GaStyle::ThreeLevel);
    p.number = "2026-001".into();
    p.status = ProjectStatus::Design;
    p.comment = "A small demonstration project.".into();
    p.areas = vec![Area {
        number: 1,
        name: "House".into(),
        comment: String::new(),
        lines: vec![
            Line { number: 0, name: "Main line".into(), medium: Medium::IP, comment: String::new() },
            Line { number: 1, name: "Ground floor".into(), medium: Medium::TP, comment: String::new() },
            Line { number: 2, name: "First floor".into(), medium: Medium::TP, comment: String::new() },
        ],
    }];

    // Building
    let bid = p.buildings[0].id;
    let mut ids = vec![];
    for _ in 0..9 {
        ids.push(p.alloc_id());
    }
    {
        let b = p.space_mut(bid).unwrap();
        let mut gf = Space::new(ids[0], "Ground floor", SpaceKind::Floor);
        gf.children.push(Space::new(ids[1], "Living room", SpaceKind::Room));
        gf.children.push(Space::new(ids[2], "Kitchen", SpaceKind::Room));
        gf.children.push(Space::new(ids[3], "Hallway", SpaceKind::Corridor));
        gf.children.push(Space::new(ids[4], "Main distribution", SpaceKind::DistributionBoard));
        let mut ff = Space::new(ids[5], "First floor", SpaceKind::Floor);
        ff.children.push(Space::new(ids[6], "Bedroom", SpaceKind::Room));
        ff.children.push(Space::new(ids[7], "Bathroom", SpaceKind::Room));
        ff.children.push(Space::new(ids[8], "Sub distribution", SpaceKind::DistributionBoard));
        b.children.push(gf);
        b.children.push(ff);
    }
    let (living, kitchen, hall, mdb, bed, sdb) = (ids[1], ids[2], ids[3], ids[4], ids[6], ids[8]);
    let _ = kitchen;

    // Group addresses
    p.main_groups = vec![
        MainGroup {
            number: 0,
            name: "Lighting".into(),
            middles: vec![MiddleGroup { number: 0, name: "Ground floor".into() }, MiddleGroup { number: 1, name: "First floor".into() }],
        },
        MainGroup { number: 1, name: "Blinds".into(), middles: vec![MiddleGroup { number: 0, name: "Ground floor".into() }] },
        MainGroup { number: 2, name: "Heating".into(), middles: vec![MiddleGroup { number: 0, name: "Bedroom".into() }] },
        MainGroup { number: 3, name: "Central".into(), middles: vec![MiddleGroup { number: 0, name: "Whole house".into() }] },
    ];
    let ga = |p: &mut Project, m: u8, mi: u8, s: u8, name: &str, dpt: &str, central: bool| -> u32 {
        let id = p.alloc_id();
        p.addresses.push(GroupAddress {
            id,
            main: m,
            middle: mi,
            sub: s,
            name: name.into(),
            dpt: dpt.into(),
            central,
            pass_through: false,
            comment: String::new(),
        });
        id
    };
    let g_lr_sw = ga(&mut p, 0, 0, 1, "Living room ceiling light switch", "1.001 switch", false);
    let g_lr_st = ga(&mut p, 0, 0, 2, "Living room ceiling light status", "1.001 switch", false);
    let g_lr_dim = ga(&mut p, 0, 0, 3, "Living room ceiling light dimming", "3.007 dimming control", false);
    let g_lr_val = ga(&mut p, 0, 0, 4, "Living room ceiling light value", "5.001 percentage (0..100%)", false);
    let g_k_sw = ga(&mut p, 0, 0, 5, "Kitchen light switch", "1.001 switch", false);
    let g_h_sw = ga(&mut p, 0, 0, 6, "Hallway light switch", "1.001 switch", false);
    let g_b_sw = ga(&mut p, 0, 1, 1, "Bedroom light switch", "1.001 switch", false);
    let g_bl_ud = ga(&mut p, 1, 0, 1, "Living room blind up/down", "1.008 up/down", false);
    let g_bl_st = ga(&mut p, 1, 0, 2, "Living room blind stop", "1.007 step", false);
    let g_t_act = ga(&mut p, 2, 0, 1, "Bedroom actual temperature", "9.001 temperature (°C)", false);
    let g_t_set = ga(&mut p, 2, 0, 2, "Bedroom setpoint", "9.001 temperature (°C)", false);
    let g_t_val = ga(&mut p, 2, 0, 3, "Bedroom heating valve", "5.001 percentage (0..100%)", false);
    let g_all_off = ga(&mut p, 3, 0, 1, "All lights off", "1.001 switch", true);
    ga(&mut p, 3, 0, 2, "System time", "10.001 time of day", true);
    ga(&mut p, 3, 0, 3, "System date", "11.001 date", true);

    let find = |name: &str| cat.iter().find(|c| c.name.starts_with(name)).cloned().unwrap();

    let ps = p.add_device_from(&find("Power supply"), 1, 1);
    let sw = p.add_device_from(&find("Switch actuator"), 1, 1);
    let dim = p.add_device_from(&find("Universal dimming"), 1, 1);
    let pb = p.add_device_from(&find("Push button"), 1, 1);
    let pd = p.add_device_from(&find("Presence detector"), 1, 1);
    let bl = p.add_device_from(&find("Blind actuator"), 1, 2);
    let rtc = p.add_device_from(&find("Room temperature"), 1, 2);
    let ha = p.add_device_from(&find("Heating actuator"), 1, 2);
    let ipr = p.add_device_from(&find("IP router"), 1, 1);
    {
        let d = p.device_mut(ps).unwrap();
        d.number = 0;
        d.name = "Power supply ground floor".into();
    }
    {
        let d = p.device_mut(ipr).unwrap();
        d.number = 0;
        d.area = 1;
        d.line = 1;
        d.name = "IP router ground floor".into();
        d.adr_ok = true;
        d.prg_ok = true;
        d.par_ok = true;
        d.grp_ok = true;
        d.cfg_ok = true;
    }
    {
        let d = p.device_mut(sw).unwrap();
        d.description = "Lights kitchen, hallway, bedroom".into();
        d.objects[0].links = vec![g_k_sw, g_all_off];
        d.objects[2].links = vec![g_h_sw, g_all_off];
        d.objects[4].links = vec![g_b_sw, g_all_off];
        d.adr_ok = true;
        d.prg_ok = true;
        d.par_ok = true;
        d.grp_ok = true;
        d.cfg_ok = true;
        d.serial = "00C5:0A1B22F3".into();
    }
    {
        let d = p.device_mut(dim).unwrap();
        d.description = "Living room ceiling light".into();
        d.objects[0].links = vec![g_lr_sw, g_all_off];
        d.objects[1].links = vec![g_lr_dim];
        d.objects[2].links = vec![g_lr_val];
        d.objects[3].links = vec![g_lr_st];
        d.adr_ok = true;
        d.prg_ok = true;
        d.par_ok = true;
        d.grp_ok = false;
        d.cfg_ok = true;
    }
    {
        let d = p.device_mut(pb).unwrap();
        d.name = "Push button living room".into();
        d.objects[0].links = vec![g_lr_sw, g_lr_st];
        d.objects[1].links = vec![g_lr_dim];
        d.objects[2].links = vec![g_bl_ud];
        d.objects[3].links = vec![g_bl_st];
        d.objects[6].links = vec![g_all_off];
    }
    {
        let d = p.device_mut(pd).unwrap();
        d.name = "Presence detector hallway".into();
        d.objects[0].links = vec![g_h_sw];
    }
    {
        let d = p.device_mut(bl).unwrap();
        d.objects[0].links = vec![g_bl_ud];
        d.objects[1].links = vec![g_bl_st];
        d.adr_ok = true;
    }
    {
        let d = p.device_mut(rtc).unwrap();
        d.name = "Room controller bedroom".into();
        d.objects[0].links = vec![g_t_act];
        d.objects[1].links = vec![g_t_set];
        d.objects[2].links = vec![g_t_val];
    }
    {
        let d = p.device_mut(ha).unwrap();
        d.objects[0].links = vec![g_t_val];
    }
    let assign = |p: &mut Project, room: u32, dev: u32| {
        if let Some(s) = p.space_mut(room) {
            s.devices.push(dev);
        }
    };
    assign(&mut p, mdb, ps);
    assign(&mut p, mdb, sw);
    assign(&mut p, mdb, dim);
    assign(&mut p, mdb, ipr);
    assign(&mut p, living, pb);
    assign(&mut p, hall, pd);
    assign(&mut p, sdb, bl);
    assign(&mut p, sdb, ha);
    assign(&mut p, bed, rtc);
    p
}
