//! Bus diagnostics, like the original's Monitors › Diagnostics pages: device info,
//! devices in programming mode, individual address check and line scan.
//! Each runs either on a real KNXnet/IP connection (in the link thread, see knx.rs)
//! or against the built-in simulation (the open project's devices).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bus::Bus;
use crate::knx::{self, fmt_ia};
use crate::model::Project;

/// Timing and protocol values; see MECHANICS.md.
pub const PROBE_ACK_WAIT_MS: u64 = 600;
pub const PROBE_ANSWER_WAIT_MS: u64 = 1500;
pub const PROG_SCAN_WAIT_MS: u64 = 2500;
pub const PROG_SCAN_REPEAT_S: f32 = 3.0;
/// Simulation: addresses probed per second during a line scan.
pub const SIM_SCAN_RATE: f32 = 64.0;
/// Factory address of a device that hasn't been given one.
pub const FACTORY_IA: u16 = 0xFFFF;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagKind {
    DeviceInfo,
    ProgScan,
    SetProg(bool),
    Check,
    LineScan,
}

impl DiagKind {
    /// Which page shows the result.
    pub fn page(self) -> DiagPage {
        match self {
            DiagKind::DeviceInfo => DiagPage::Info,
            DiagKind::ProgScan | DiagKind::SetProg(_) => DiagPage::Prog,
            DiagKind::Check => DiagPage::Check,
            DiagKind::LineScan => DiagPage::Scan,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagPage {
    Info,
    Prog,
    Check,
    Scan,
}

/// What the link thread is asked to do.
pub enum DiagReq {
    DeviceInfo(u16),
    ProgScan,
    SetProg(u16, bool),
    Check(u16),
    LineScan { area: u8, line: u8, cancel: Arc<AtomicBool> },
}

/// A device that answered during a scan.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanHit {
    pub ia: u16,
    pub mask: Option<u16>,
}

/// Progress or result from the link thread (fields add to what came before).
#[derive(Clone, Debug, Default)]
pub struct DiagUpdate {
    pub job: u64,
    pub progress: f32,
    pub msg: String,
    pub info: Vec<(String, String)>,
    pub hits: Vec<ScanHit>,
    /// Set when finished: Ok(summary) or Err(reason).
    pub done: Option<Result<String, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagState {
    Running,
    Done,
    Failed,
    Stopped,
}

#[derive(Clone, Debug)]
pub struct DiagTask {
    pub id: u64,
    pub kind: DiagKind,
    /// Address or line it is about, as text ("1.1.5", "1.1").
    pub target: String,
    pub state: DiagState,
    pub progress: f32,
    pub message: String,
    pub info: Vec<(String, String)>,
    pub hits: Vec<ScanHit>,
    pub real: bool,
    pub cancel: Arc<AtomicBool>,
    pub started: String,
    // simulation
    sim_ia: u16,
    sim_line: (u8, u8),
    sim_pos: f32,
    sim_timer: f32,
}

impl DiagTask {
    pub fn running(&self) -> bool {
        self.state == DiagState::Running
    }
}

/// Plain description of a mask version (device descriptor type 0).
pub fn mask_text(m: u16) -> String {
    let medium = match m >> 12 {
        0 => "TP1",
        1 => "PL110",
        2 => "RF",
        5 => "IP",
        _ => "other medium",
    };
    let kind = match m & 0x0FFF {
        0x010..=0x013 => "System 1 (BCU 1)",
        0x020..=0x025 => "System 2 (BCU 2)",
        0x300 => "USB interface",
        0x310 | 0x311 => "USB interface (cEMI)",
        0x700 | 0x701 => "System 7 (BIM M 112)",
        0x705 => "System 7",
        0x7B0 => "System B",
        0x910 | 0x911 => "Line / area coupler",
        0x912 => "Media coupler",
        0x91A => "Line / area coupler (System B)",
        _ => "unknown type",
    };
    format!("{m:04X} – {kind}, {medium}")
}

/// Does this mask version use interface object properties (rather than fixed memory) for
/// identity, programming mode and load states?
pub fn uses_properties(m: u16) -> bool {
    !matches!(m & 0x0FFF, 0x010..=0x013 | 0x020..=0x025)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

pub fn serial_text(b: &[u8]) -> String {
    if b.len() >= 6 {
        format!("{:02X}{:02X}:{:02X}{:02X}{:02X}{:02X}", b[0], b[1], b[2], b[3], b[4], b[5])
    } else {
        hex(b)
    }
}

/// Order info as text when it is printable, otherwise as hex.
pub fn order_text(b: &[u8]) -> String {
    let t: String = b.iter().take_while(|c| **c != 0).map(|c| *c as char).collect();
    if !t.is_empty() && t.chars().all(|c| c.is_ascii_graphic() || c == ' ') { t.trim().to_string() } else { hex(b) }
}

pub fn load_state_text(s: u8) -> String {
    match s {
        0 => "Unloaded".into(),
        1 => "Loaded".into(),
        2 => "Loading".into(),
        3 => "Error".into(),
        4 => "Unloading".into(),
        5 => "Load completing".into(),
        x => format!("Unknown ({x})"),
    }
}

/// Parse "1.1" or "1/1" as area and line.
pub fn parse_line(s: &str) -> Option<(u8, u8)> {
    let mut it = s.trim().split(['.', '/']);
    let a: u8 = it.next()?.trim().parse().ok()?;
    let l: u8 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || a > 15 || l > 15 {
        return None;
    }
    Some((a, l))
}

/// Where a simulated device can be reached: its programmed address, or the factory address.
fn sim_address(d: &crate::model::Device) -> Option<u16> {
    if !d.has_app() {
        return None;
    }
    if d.adr_ok { knx::parse_ia(&d.address()) } else { Some(FACTORY_IA) }
}

/// Mask version a simulated device reports (estimated from what it is).
fn sim_mask(d: &crate::model::Device) -> u16 {
    let n = d.product.to_lowercase();
    if n.contains("coupler") || n.contains("router") { 0x091A } else { 0x07B0 }
}

impl Bus {
    pub fn diag_latest(&self, page: DiagPage) -> Option<&DiagTask> {
        self.diags.iter().rev().find(|t| t.kind.page() == page)
    }

    pub fn diag_busy(&self) -> bool {
        self.diags.iter().any(|t| t.running())
    }

    /// Start a diagnostic. `target` is an individual address or a line ("1.1").
    pub fn start_diag(&mut self, kind: DiagKind, target: &str) -> Result<(), String> {
        if !self.connected {
            return Err("Connect to the bus first (Bus tab on the start page)".into());
        }
        if self.diag_busy() {
            return Err("Another diagnostic is still running – wait for it or stop it".into());
        }
        if self.is_real() && self.jobs.iter().any(|j| j.real && matches!(j.state, crate::bus::JobState::Waiting | crate::bus::JobState::PressButton | crate::bus::JobState::Running)) {
            return Err("A download is running on the bus – wait until it has finished".into());
        }
        let needs_ia = matches!(kind, DiagKind::DeviceInfo | DiagKind::Check | DiagKind::SetProg(_));
        let ia = if needs_ia {
            Some(knx::parse_ia(target).ok_or_else(|| format!("\"{}\" is not an individual address (like 1.1.5)", target.trim()))?)
        } else {
            None
        };
        let line = if kind == DiagKind::LineScan {
            Some(parse_line(target).ok_or_else(|| format!("\"{}\" is not a line (like 1.1)", target.trim()))?)
        } else {
            None
        };
        let id = self.next_diag;
        self.next_diag += 1;
        let real = self.is_real();
        let cancel = Arc::new(AtomicBool::new(false));
        let target_text = match (ia, line) {
            (Some(a), _) => fmt_ia(a),
            (_, Some((a, l))) => format!("{a}.{l}"),
            _ => String::new(),
        };
        let first = match kind {
            DiagKind::DeviceInfo => format!("Reading {target_text}…"),
            DiagKind::ProgScan => "Looking for devices in programming mode…".into(),
            DiagKind::SetProg(on) => format!("Switching programming mode {} on {target_text}…", if on { "on" } else { "off" }),
            DiagKind::Check => format!("Checking {target_text}…"),
            DiagKind::LineScan => format!("Scanning line {target_text}…"),
        };
        let task = DiagTask {
            id,
            kind,
            target: target_text,
            state: DiagState::Running,
            progress: 0.0,
            message: first,
            info: vec![],
            hits: vec![],
            real,
            cancel: cancel.clone(),
            started: crate::model::fmt_time(crate::model::now_secs()),
            sim_ia: ia.unwrap_or(0),
            sim_line: line.unwrap_or((0, 0)),
            sim_pos: 0.0,
            sim_timer: match kind {
                DiagKind::DeviceInfo => 1.2,
                DiagKind::ProgScan => PROG_SCAN_WAIT_MS as f32 / 1000.0,
                DiagKind::SetProg(_) => 0.6,
                DiagKind::Check => 0.8,
                DiagKind::LineScan => 0.0,
            },
        };
        if real {
            let req = match kind {
                DiagKind::DeviceInfo => DiagReq::DeviceInfo(ia.unwrap()),
                DiagKind::ProgScan => DiagReq::ProgScan,
                DiagKind::SetProg(on) => DiagReq::SetProg(ia.unwrap(), on),
                DiagKind::Check => DiagReq::Check(ia.unwrap()),
                DiagKind::LineScan => {
                    let (area, line) = line.unwrap();
                    DiagReq::LineScan { area, line, cancel }
                }
            };
            self.send_diag(id, req)?;
        }
        // Keep the last few results per page.
        self.diags.push(task);
        if self.diags.len() > 24 {
            self.diags.remove(0);
        }
        Ok(())
    }

    /// Stop a running line scan (or forget a running simulated diagnostic).
    pub fn stop_diag(&mut self) {
        for t in self.diags.iter_mut().filter(|t| t.running()) {
            t.cancel.store(true, Ordering::Relaxed);
            if !t.real {
                t.state = DiagState::Stopped;
                t.message = format!("Stopped – {} device(s) found so far", t.hits.len());
            } else {
                t.message = "Stopping…".into();
            }
        }
    }

    /// Apply an update from the link thread.
    pub(crate) fn apply_diag(&mut self, u: DiagUpdate) {
        let Some(t) = self.diags.iter_mut().find(|t| t.id == u.job) else { return };
        t.progress = u.progress;
        if !u.msg.is_empty() {
            t.message = u.msg;
        }
        t.info.extend(u.info);
        t.hits.extend(u.hits);
        if let Some(r) = u.done {
            match r {
                Ok(m) => {
                    t.state = if t.cancel.load(Ordering::Relaxed) { DiagState::Stopped } else { DiagState::Done };
                    t.message = m;
                    t.progress = 1.0;
                }
                Err(e) => {
                    t.state = DiagState::Failed;
                    t.message = e;
                }
            }
        }
    }

    pub(crate) fn fail_real_diags(&mut self, msg: &str) {
        for t in self.diags.iter_mut().filter(|t| t.real && t.running()) {
            t.state = DiagState::Failed;
            t.message = format!("Connection lost: {msg}");
        }
    }

    /// Simulation: devices of the open project answer like real ones.
    pub(crate) fn tick_diags(&mut self, dt: f32, proj: Option<&Project>) {
        let sim_prog = self.sim_prog.clone();
        let Some(i) = self.diags.iter().position(|t| t.running() && !t.real) else { return };
        let Some(proj) = proj else {
            let t = &mut self.diags[i];
            t.state = DiagState::Failed;
            t.message = "The simulation needs an open project".into();
            return;
        };
        // Simulated installation: (address, device id, mask)
        let devices: Vec<(u16, u32, u16)> = proj.devices.iter().filter_map(|d| sim_address(d).map(|a| (a, d.id, sim_mask(d)))).collect();
        let t = &mut self.diags[i];
        if t.kind == DiagKind::LineScan {
            let (area, line) = t.sim_line;
            let before = t.sim_pos as u32;
            t.sim_pos = (t.sim_pos + dt * SIM_SCAN_RATE).min(256.0);
            let after = t.sim_pos as u32;
            for n in before..after {
                let ia = ((area as u16) << 12) | ((line as u16) << 8) | n as u16;
                if let Some((_, _, m)) = devices.iter().find(|(a, _, _)| *a == ia) {
                    if !t.hits.iter().any(|h| h.ia == ia) {
                        t.hits.push(ScanHit { ia, mask: Some(*m) });
                    }
                }
            }
            t.progress = t.sim_pos / 256.0;
            t.message = format!("Scanning {area}.{line}.{} (simulated) – {} found", after.min(255), t.hits.len());
            if after >= 256 {
                t.state = DiagState::Done;
                t.progress = 1.0;
                t.message = format!("Line {area}.{line} scanned (simulated): {} device(s) answered", t.hits.len());
            }
            return;
        }
        t.sim_timer -= dt;
        t.progress = (t.progress + dt).min(0.95);
        if t.sim_timer > 0.0 {
            return;
        }
        let ia = t.sim_ia;
        let at: Vec<&(u16, u32, u16)> = devices.iter().filter(|(a, _, _)| *a == ia).collect();
        t.progress = 1.0;
        t.state = DiagState::Done;
        match t.kind {
            DiagKind::DeviceInfo => {
                let Some((_, id, mask)) = at.first() else {
                    t.state = DiagState::Failed;
                    t.message = format!("No device answers at {} (simulated)", fmt_ia(ia));
                    return;
                };
                let d = proj.device(*id).unwrap();
                let mut info = vec![
                    ("Individual address".to_string(), fmt_ia(ia)),
                    ("Mask version".into(), mask_text(*mask)),
                    ("Manufacturer".into(), d.manufacturer.clone()),
                    ("Order number".into(), if d.order_no.is_empty() { "–".into() } else { d.order_no.clone() }),
                    ("Serial number".into(), if d.serial.is_empty() { sim_serial(d.id) } else { d.serial.clone() }),
                    ("Firmware revision".into(), "1".into()),
                    ("Programming mode".into(), if sim_prog.contains(&ia) { "On".into() } else { "Off".into() }),
                    ("Application program".into(), if d.prg_ok { d.app_program.clone() } else { "– (not loaded)".into() }),
                ];
                let ls = |ok: bool| if ok { load_state_text(1) } else { load_state_text(0) };
                info.push(("Group address table".into(), ls(d.grp_ok)));
                info.push(("Association table".into(), ls(d.grp_ok)));
                info.push(("Application".into(), ls(d.prg_ok && d.par_ok)));
                if at.len() > 1 {
                    info.push(("Note".into(), format!("{} devices share this address", at.len())));
                }
                t.info = info;
                t.message = format!("Device info read from {} (simulated)", fmt_ia(ia));
            }
            DiagKind::Check => {
                t.info.clear();
                match at.len() {
                    0 => t.message = format!("No device answers at {} – the address is free (simulated)", fmt_ia(ia)),
                    1 => {
                        t.info.push(("Mask version".into(), mask_text(at[0].2)));
                        t.message = format!("A device answers at {} (simulated)", fmt_ia(ia));
                    }
                    n => t.message = format!("{n} devices answer at {} – the address is used more than once (simulated)", fmt_ia(ia)),
                }
                t.hits = at.iter().map(|(a, _, m)| ScanHit { ia: *a, mask: Some(*m) }).collect();
            }
            DiagKind::SetProg(on) => {
                if at.is_empty() {
                    t.state = DiagState::Failed;
                    t.message = format!("No device answers at {} (simulated)", fmt_ia(ia));
                    return;
                }
                if on {
                    self.sim_prog.insert(ia);
                } else {
                    self.sim_prog.remove(&ia);
                }
                let t = &mut self.diags[i];
                t.message = format!("Programming mode of {} switched {} (simulated)", fmt_ia(ia), if on { "on" } else { "off" });
            }
            DiagKind::ProgScan => {
                let mut v: Vec<u16> = sim_prog.iter().copied().collect();
                v.sort();
                t.hits = v.iter().map(|a| ScanHit { ia: *a, mask: devices.iter().find(|(x, _, _)| x == a).map(|d| d.2) }).collect();
                t.message = match v.len() {
                    0 => "No device is in programming mode (simulated)".into(),
                    1 => "1 device is in programming mode (simulated)".into(),
                    n => format!("{n} devices are in programming mode (simulated)"),
                };
            }
            DiagKind::LineScan => {}
        }
    }
}

/// A stable made-up serial number for a simulated device.
fn sim_serial(id: u32) -> String {
    let x = (id as u64).wrapping_mul(0x9E37_79B9).wrapping_add(0x00C5_0000_0000) & 0xFFFF_FFFF_FFFF;
    let b = x.to_be_bytes();
    serial_text(&b[2..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_masks() {
        assert_eq!(parse_line("1.1"), Some((1, 1)));
        assert_eq!(parse_line(" 15/0 "), Some((15, 0)));
        assert_eq!(parse_line("1.1.1"), None);
        assert_eq!(parse_line("16.1"), None);
        assert!(mask_text(0x07B0).contains("System B"));
        assert!(mask_text(0x57B0).contains("IP"));
        assert!(!uses_properties(0x0012));
        assert!(uses_properties(0x0705));
        assert_eq!(serial_text(&[0, 0xC5, 1, 2, 3, 4]), "00C5:01020304");
        assert_eq!(order_text(b"AB-12\0\0\0\0\0"), "AB-12");
    }

    #[test]
    fn simulated_line_scan_and_prog_mode() {
        let mut proj = crate::model::sample_project(&crate::model::catalog());
        for d in proj.devices.iter_mut() {
            d.adr_ok = d.has_app();
        }
        let expect: Vec<u16> = proj.devices.iter().filter(|d| d.has_app() && d.area == 1 && d.line == 1).filter_map(|d| knx::parse_ia(&d.address())).collect();
        let mut bus = Bus::new();
        bus.start_diag(DiagKind::LineScan, "1.1").unwrap();
        for _ in 0..100 {
            bus.tick(0.1, Some(&mut proj));
        }
        let t = bus.diag_latest(DiagPage::Scan).unwrap();
        assert_eq!(t.state, DiagState::Done);
        let mut got: Vec<u16> = t.hits.iter().map(|h| h.ia).collect();
        got.sort();
        let mut e = expect.clone();
        e.sort();
        e.dedup();
        assert_eq!(got, e);

        let ia = fmt_ia(e[0]);
        bus.start_diag(DiagKind::SetProg(true), &ia).unwrap();
        for _ in 0..20 {
            bus.tick(0.1, Some(&mut proj));
        }
        bus.start_diag(DiagKind::ProgScan, "").unwrap();
        for _ in 0..40 {
            bus.tick(0.1, Some(&mut proj));
        }
        let t = bus.diag_latest(DiagPage::Prog).unwrap();
        assert_eq!(t.hits.len(), 1);
        assert_eq!(t.hits[0].ia, e[0]);

        bus.start_diag(DiagKind::DeviceInfo, &ia).unwrap();
        for _ in 0..20 {
            bus.tick(0.1, Some(&mut proj));
        }
        let t = bus.diag_latest(DiagPage::Info).unwrap();
        assert_eq!(t.state, DiagState::Done);
        assert!(t.info.iter().any(|(k, v)| k == "Programming mode" && v == "On"));

        bus.start_diag(DiagKind::Check, "1.1.250").unwrap();
        for _ in 0..20 {
            bus.tick(0.1, Some(&mut proj));
        }
        assert!(bus.diag_latest(DiagPage::Check).unwrap().message.contains("free"));
    }
}
