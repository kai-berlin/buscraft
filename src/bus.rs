//! Bus access: simulated interfaces and real KNXnet/IP interfaces (see `knx`),
//! the telegram log (group/bus monitor) and commissioning jobs.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use crate::knx::{self, Endpoint, Event, Frame, Phase};
use crate::model::{Device, Project};

pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
    Write,
    Read,
    Response,
}
impl Service {
    pub fn label(self) -> &'static str {
        match self {
            Service::Write => "GroupValueWrite",
            Service::Read => "GroupValueRead",
            Service::Response => "GroupValueResponse",
        }
    }
    fn code(self) -> u16 {
        match self {
            Service::Read => 0x000,
            Service::Response => 0x040,
            Service::Write => 0x080,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Telegram {
    pub seq: u64,
    pub time: String,
    pub from_bus: bool,
    pub prio: &'static str,
    pub source: String,
    pub source_name: String,
    pub dest: String,
    pub dest_name: String,
    pub rout: u8,
    pub service: &'static str,
    pub dpt: String,
    pub info: String,
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkKind {
    Simulated,
    Tunnel(SocketAddrV4),
    Routing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Interface {
    pub name: String,
    pub kind: String,
    pub address: String,
    pub detail: String,
    pub link: LinkKind,
}

impl Interface {
    pub fn is_real(&self) -> bool {
        self.link != LinkKind::Simulated
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkState {
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobKind {
    Full,
    Address,
    Application,
    Partial,
    Unload,
    /// Read the device's tables and memory back and compare (writes nothing).
    Check,
}
impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Full => "Download all",
            JobKind::Address => "Program individual address",
            JobKind::Application => "Download application",
            JobKind::Partial => "Partial download",
            JobKind::Unload => "Unload application",
            JobKind::Check => "Check device",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobState {
    Waiting,
    PressButton,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub project: u64,
    pub device: u32,
    pub address: String,
    pub name: String,
    pub kind: JobKind,
    pub state: JobState,
    pub progress: f32,
    pub timer: f32,
    pub message: String,
    pub real: bool,
    /// Download plan (application download); None for address programming.
    pub steps: Option<Vec<crate::knxload::Step>>,
    /// Step descriptions (for showing progress).
    pub step_names: Vec<String>,
    /// Report of a device check.
    pub report: Option<String>,
}

pub struct Bus {
    pub interfaces: Vec<Interface>,
    pub current: usize,
    pub connected: bool,
    pub state: LinkState,
    link: Option<knx::Link>,
    pub link_detail: String,
    /// Our own individual address on a real connection.
    pub own_address: String,
    discovery: Option<Receiver<Result<Vec<knx::Found>, String>>>,
    pub searching: bool,
    /// Messages for the status line (taken by the app each frame).
    pub notices: Vec<String>,
    /// Finished device checks to show: (title, text).
    pub reports: Vec<(String, String)>,
    pub log: Vec<Telegram>,
    /// Last value seen per group address (as shown in the monitor): address text -> (value, time).
    pub last_values: std::collections::HashMap<String, (String, String)>,
    pub seq: u64,
    pub monitor_running: bool,
    pub simulate_traffic: bool,
    pub traffic_timer: f32,
    pub clock: f64,
    pub rng: Rng,
    pub jobs: Vec<Job>,
    next_job: u64,
    /// Diagnostics (device info, programming mode, address check, line scan), newest last.
    pub diags: Vec<crate::diag::DiagTask>,
    pub(crate) next_diag: u64,
    /// Simulation: addresses of devices whose programming mode is on.
    pub sim_prog: std::collections::HashSet<u16>,
}

impl Bus {
    pub fn new() -> Self {
        let sim = |name: &str, kind: &str, address: &str, detail: &str| Interface {
            name: name.into(),
            kind: kind.into(),
            address: address.into(),
            detail: detail.into(),
            link: LinkKind::Simulated,
        };
        Bus {
            interfaces: vec![
                sim("Simulated USB interface", "Simulation", "15.15.255", "Built-in simulated installation – no hardware needed"),
                sim("Simulated IP tunnel", "Simulation", "1.0.250", "Built-in simulated installation – no hardware needed"),
            ],
            current: 0,
            connected: true,
            state: LinkState::Connected,
            link: None,
            link_detail: String::new(),
            own_address: "15.15.255".into(),
            discovery: None,
            searching: false,
            notices: vec![],
            reports: vec![],
            log: vec![],
            last_values: Default::default(),
            seq: 0,
            monitor_running: true,
            simulate_traffic: true,
            traffic_timer: 0.0,
            clock: 0.0,
            rng: Rng::new(0x9E37_79B9_7F4A_7C15 ^ crate::model::now_secs()),
            jobs: vec![],
            next_job: 1,
            diags: vec![],
            next_diag: 1,
            sim_prog: Default::default(),
        }
    }

    pub fn iface(&self) -> &Interface {
        &self.interfaces[self.current.min(self.interfaces.len() - 1)]
    }

    pub fn is_real(&self) -> bool {
        self.iface().is_real()
    }

    // ------------------------------------------------------------ connection

    /// Choose another interface. Closes the current connection.
    pub fn select(&mut self, i: usize) {
        if i == self.current || i >= self.interfaces.len() {
            return;
        }
        self.disconnect();
        self.current = i;
    }

    pub fn connect(&mut self) {
        let iface = self.iface().clone();
        match iface.link {
            LinkKind::Simulated => {
                self.state = LinkState::Connected;
                self.connected = true;
                self.own_address = iface.address.clone();
                self.link_detail = "Simulation".into();
            }
            LinkKind::Tunnel(addr) => {
                self.link = Some(knx::Link::open(Endpoint::Tunnel(addr)));
                self.state = LinkState::Connecting;
                self.connected = false;
            }
            LinkKind::Routing => {
                self.link = Some(knx::Link::open(Endpoint::Routing));
                self.state = LinkState::Connecting;
                self.connected = false;
            }
        }
    }

    pub fn disconnect(&mut self) {
        if let Some(l) = self.link.take() {
            l.close();
        }
        self.state = LinkState::Disconnected;
        self.connected = false;
        self.fail_real_diags("Connection closed");
        for j in self.jobs.iter_mut().filter(|j| j.real && matches!(j.state, JobState::Waiting | JobState::PressButton | JobState::Running)) {
            j.state = JobState::Failed;
            j.message = "Connection closed".into();
        }
    }

    pub fn state_text(&self) -> String {
        match &self.state {
            LinkState::Disconnected => "Not connected".into(),
            LinkState::Connecting => "Connecting…".into(),
            LinkState::Connected => "Connected".into(),
            LinkState::Failed(e) => format!("Connection failed: {e}"),
        }
    }

    /// Start a network search for KNXnet/IP interfaces.
    pub fn search(&mut self) {
        if self.searching {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(knx::discover(Duration::from_secs(3)));
        });
        self.discovery = Some(rx);
        self.searching = true;
    }

    /// Add (or update) a real interface; returns its index.
    pub fn add_interface(&mut self, iface: Interface) -> usize {
        if let Some(i) = self.interfaces.iter().position(|x| x.link == iface.link) {
            self.interfaces[i] = iface;
            i
        } else {
            self.interfaces.push(iface);
            self.interfaces.len() - 1
        }
    }

    pub fn manual_tunnel(ip: Ipv4Addr, port: u16) -> Interface {
        Interface {
            name: format!("IP interface {ip}"),
            kind: "KNXnet/IP tunnelling".into(),
            address: "–".into(),
            detail: format!("{ip}:{port} (added by hand)"),
            link: LinkKind::Tunnel(SocketAddrV4::new(ip, port)),
        }
    }

    pub fn routing_interface() -> Interface {
        Interface {
            name: "IP routing (multicast)".into(),
            kind: "KNXnet/IP routing".into(),
            address: "–".into(),
            detail: format!("{}:{} via IP routers / IP devices", knx::MCAST, knx::PORT),
            link: LinkKind::Routing,
        }
    }

    // ------------------------------------------------------------ telegrams

    /// Hand a diagnostic to the link thread.
    pub(crate) fn send_diag(&mut self, job: u64, req: crate::diag::DiagReq) -> Result<(), String> {
        match &self.link {
            Some(l) => {
                l.diag(job, req);
                Ok(())
            }
            None => Err("No bus connection".into()),
        }
    }

    fn time_str(&self) -> String {
        let t = self.clock;
        let s = t as u64;
        format!("{:02}:{:02}:{:02}.{:03}", (s / 3600) % 24, (s / 60) % 60, s % 60, (t.fract() * 1000.0) as u32)
    }

    pub fn push(&mut self, mut t: Telegram) {
        if matches!(t.service, "GroupValueWrite" | "GroupValueResponse") {
            self.last_values.insert(t.dest.clone(), (t.info.clone(), self.time_str()));
        }
        if !self.monitor_running {
            return;
        }
        self.seq += 1;
        t.seq = self.seq;
        t.time = self.time_str();
        self.log.push(t);
        if self.log.len() > 5000 {
            self.log.drain(0..1000);
        }
    }

    /// Send a group telegram from the interface (or, in the simulation, from a device).
    pub fn send_group(&mut self, proj: &Project, ga_id: u32, service: Service, value: &str, from: Option<&Device>) -> Result<(), String> {
        let Some(ga) = proj.ga(ga_id) else { return Err("Unknown group address".into()) };
        if self.is_real() {
            if self.state != LinkState::Connected {
                return Err("Not connected to the bus".into());
            }
            let (data, small) = if service == Service::Read { (vec![], true) } else { knx::encode_value(&ga.dpt, value)? };
            let mut f = Frame::request(ga.raw(), true, knx::group_tpdu(service.code(), &data, small));
            if let Some(l) = &self.link {
                l.send(f.clone());
            }
            f.src = knx::parse_ia(&self.own_address).unwrap_or(0);
            let info = if service == Service::Read { String::new() } else { format_value(&ga.dpt, &knx::decode_value(&ga.dpt, &data, small)) };
            self.push(Telegram {
                seq: 0,
                time: String::new(),
                from_bus: false,
                prio: f.priority(),
                source: self.own_address.clone(),
                source_name: self.iface().name.clone(),
                dest: ga.fmt(proj.ga_style),
                dest_name: ga.name.clone(),
                rout: f.hops(),
                service: service.label(),
                dpt: ga.dpt.clone(),
                info,
                raw: f.raw_hex(),
            });
            return Ok(());
        }
        if !self.connected {
            return Err("Not connected to the bus".into());
        }
        let (src, src_name, from_bus) = match from {
            Some(d) => (d.address(), d.name.clone(), true),
            None => (self.iface().address.clone(), self.iface().name.clone(), false),
        };
        let info = if service == Service::Read { String::new() } else { format_value(&ga.dpt, value) };
        let (data, small) = if service == Service::Read { (vec![], true) } else { knx::encode_value(&ga.dpt, value).unwrap_or((vec![0], true)) };
        let mut f = Frame::request(ga.raw(), true, knx::group_tpdu(service.code(), &data, small));
        f.code = knx::L_DATA_IND;
        f.src = knx::parse_ia(&src).unwrap_or(0);
        self.push(Telegram {
            seq: 0,
            time: String::new(),
            from_bus,
            prio: "Low",
            source: src,
            source_name: src_name,
            dest: ga.fmt(proj.ga_style),
            dest_name: ga.name.clone(),
            rout: 6,
            service: service.label(),
            dpt: ga.dpt.clone(),
            info,
            raw: f.raw_hex(),
        });
        // A read gets an answer from the first linked object with the Read flag.
        if service == Service::Read {
            for (dev, oi) in proj.links_of(ga_id) {
                let d = proj.device(dev).unwrap();
                if d.objects[oi].flags.r {
                    let v = self.random_value(&ga.dpt);
                    let d = d.clone();
                    let _ = self.send_group(proj, ga_id, Service::Response, &v, Some(&d));
                    break;
                }
            }
        }
        Ok(())
    }

    /// Log a telegram received from a real bus.
    fn log_frame(&mut self, f: &Frame, proj: Option<&Project>) {
        let src = knx::fmt_ia(f.src);
        let source_name = proj
            .and_then(|p| p.devices.iter().find(|d| d.has_app() && d.address() == src))
            .map(|d| d.name.clone())
            .unwrap_or_default();
        let (dest, dest_name, dpt) = if f.is_group() {
            match proj.and_then(|p| p.addresses.iter().find(|g| g.raw() == f.dst).map(|g| (g.fmt(p.ga_style), g.name.clone(), g.dpt.clone()))) {
                Some(x) => x,
                None => (knx::fmt_ga3(f.dst), String::new(), String::new()),
            }
        } else {
            let d = knx::fmt_ia(f.dst);
            let n = proj.and_then(|p| p.devices.iter().find(|x| x.has_app() && x.address() == d)).map(|x| x.name.clone()).unwrap_or_default();
            (d, n, String::new())
        };
        let svc = f.service();
        let info = match svc {
            0x040 | 0x080 => {
                let (data, small) = f.value();
                if dpt.is_empty() {
                    knx::decode_value("", &data, small)
                } else {
                    format_value(&dpt, &knx::decode_value(&dpt, &data, small))
                }
            }
            0x0C0 if f.tpdu.len() >= 4 => format!("New address {}", knx::fmt_ia(u16::from_be_bytes([f.tpdu[2], f.tpdu[3]]))),
            _ => String::new(),
        };
        self.push(Telegram {
            seq: 0,
            time: String::new(),
            from_bus: true,
            prio: f.priority(),
            source: src,
            source_name,
            dest,
            dest_name,
            rout: f.hops(),
            service: knx::service_label(svc),
            dpt,
            info,
            raw: f.raw_hex(),
        });
    }

    pub fn random_value(&mut self, dpt: &str) -> String {
        match dpt.split('.').next().unwrap_or("") {
            "1" => if self.rng.below(2) == 0 { "0".into() } else { "1".into() },
            "3" => format!("{}", self.rng.below(16)),
            "5" => format!("{}", self.rng.below(101)),
            "9" => {
                if dpt.starts_with("9.001") {
                    format!("{:.1}", 18.0 + self.rng.below(60) as f32 / 10.0)
                } else if dpt.starts_with("9.004") {
                    format!("{}", 100 + self.rng.below(900))
                } else {
                    format!("{:.1}", self.rng.below(120) as f32 / 10.0)
                }
            }
            "20" => format!("{}", 1 + self.rng.below(4)),
            _ => format!("{}", self.rng.below(256)),
        }
    }

    // ------------------------------------------------------------ tick

    /// Handle connection events, simulated traffic and commissioning jobs.
    pub fn tick(&mut self, dt: f32, mut proj: Option<&mut Project>) {
        self.clock += dt as f64;

        // Search results
        if let Some(rx) = &self.discovery {
            if let Ok(res) = rx.try_recv() {
                self.discovery = None;
                self.searching = false;
                match res {
                    Ok(found) => {
                        let n = found.len();
                        for f in found {
                            if f.tunnelling || !f.routing {
                                self.add_interface(Interface {
                                    name: f.name.clone(),
                                    kind: "KNXnet/IP tunnelling".into(),
                                    address: knx::fmt_ia(f.ia),
                                    detail: format!("{} · MAC {}", f.addr, f.mac),
                                    link: LinkKind::Tunnel(f.addr),
                                });
                            }
                            if f.routing {
                                self.add_interface(Bus::routing_interface());
                            }
                        }
                        self.notices.push(if n == 0 { "No KNXnet/IP interfaces answered on this network".into() } else { format!("Found {n} KNXnet/IP interface(s)") });
                    }
                    Err(e) => self.notices.push(format!("Search failed: {e}")),
                }
            }
        }

        // Real connection events
        let mut events = vec![];
        if let Some(l) = &self.link {
            while let Ok(e) = l.rx.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            match e {
                Event::Connected { ia, detail } => {
                    self.state = LinkState::Connected;
                    self.connected = true;
                    self.own_address = if ia == 0 { "0.0.0".into() } else { knx::fmt_ia(ia) };
                    self.link_detail = detail.clone();
                    let n = self.iface().name.clone();
                    self.notices.push(format!("Connected to {n} as {} – {detail}", self.own_address));
                }
                Event::Failed(msg) => {
                    self.link = None;
                    self.connected = false;
                    self.state = LinkState::Failed(msg.clone());
                    self.notices.push(format!("Bus connection failed: {msg}"));
                    self.fail_real_jobs(&msg);
                    self.fail_real_diags(&msg);
                }
                Event::Closed(msg) => {
                    self.link = None;
                    self.connected = false;
                    if matches!(self.state, LinkState::Connected | LinkState::Connecting) {
                        self.state = if msg == "Disconnected" { LinkState::Disconnected } else { LinkState::Failed(msg.clone()) };
                    }
                    self.notices.push(format!("Bus connection closed: {msg}"));
                    self.fail_real_jobs(&msg);
                    self.fail_real_diags(&msg);
                }
                Event::Frame(f) => self.log_frame(&f, proj.as_deref()),
                Event::Diag(u) => self.apply_diag(u),
                Event::Report { job, lines } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == job) {
                        let text = lines.join("\n");
                        j.report = Some(text.clone());
                        self.reports.push((format!("Device check {} {}", j.address, j.name), text));
                    }
                }
                Event::Job { job, phase, progress, msg } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == job) {
                        j.progress = progress;
                        j.message = msg;
                        j.state = match phase {
                            Phase::PressButton => JobState::PressButton,
                            Phase::Running => JobState::Running,
                            Phase::Done => JobState::Done,
                            Phase::Failed => JobState::Failed,
                        };
                        if phase == Phase::Done {
                            if let Some(p) = proj.as_deref_mut() {
                                if p.id == j.project {
                                    if let Some(d) = p.device_mut(j.device) {
                                        match j.kind {
                                            JobKind::Address => d.adr_ok = true,
                                            JobKind::Full | JobKind::Application => {
                                                d.adr_ok = true;
                                                d.prg_ok = true;
                                                d.par_ok = true;
                                                d.grp_ok = true;
                                                d.cfg_ok = true;
                                            }
                                            JobKind::Partial => {
                                                d.par_ok = true;
                                                d.grp_ok = true;
                                                d.cfg_ok = true;
                                            }
                                            JobKind::Unload => {
                                                d.prg_ok = false;
                                                d.par_ok = false;
                                                d.grp_ok = false;
                                                d.cfg_ok = false;
                                            }
                                            JobKind::Check => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.tick_diags(dt, proj.as_deref());

        let Some(proj) = proj else { return };

        // Random traffic from sensors in the open project (simulation only).
        if !self.is_real() && self.connected && self.monitor_running && self.simulate_traffic {
            self.traffic_timer -= dt;
            if self.traffic_timer <= 0.0 {
                self.traffic_timer = 1.2 + self.rng.below(2500) as f32 / 1000.0;
                let senders: Vec<(u32, usize)> = proj
                    .devices
                    .iter()
                    .flat_map(|d| d.objects.iter().enumerate().filter(|(_, o)| o.flags.t && !o.links.is_empty()).map(move |(i, _)| (d.id, i)))
                    .collect();
                if !senders.is_empty() {
                    let (dev, oi) = senders[self.rng.below(senders.len() as u64) as usize];
                    let d = proj.device(dev).unwrap().clone();
                    let ga = d.objects[oi].links[0];
                    let dpt = proj.ga(ga).map(|g| g.dpt.clone()).unwrap_or_default();
                    let v = self.random_value(&dpt);
                    let _ = self.send_group(proj, ga, Service::Write, &v, Some(&d));
                }
            }
        }

        // Commissioning jobs: one runs at a time.
        let pid = proj.id;
        let real = self.is_real();
        let connected = self.connected;
        let busy_real = self.jobs.iter().any(|j| j.real && matches!(j.state, JobState::PressButton | JobState::Running));
        let Some(job) = self.jobs.iter_mut().find(|j| j.project == pid && matches!(j.state, JobState::Waiting | JobState::PressButton | JobState::Running)) else { return };
        if !connected {
            job.state = JobState::Failed;
            job.message = "No bus connection".into();
            return;
        }
        if real || job.real {
            if job.state != JobState::Waiting || busy_real {
                return; // the connection thread reports progress
            }
            job.real = true;
            if job.kind != JobKind::Address {
                let Some(steps) = job.steps.take() else {
                    job.state = JobState::Failed;
                    job.message = "No download data for this device".into();
                    return;
                };
                let Some(ia) = proj.device(job.device).and_then(|d| knx::parse_ia(&d.address())) else {
                    job.state = JobState::Failed;
                    job.message = "The device has no valid individual address".into();
                    return;
                };
                job.state = JobState::Running;
                job.message = "Starting download…".into();
                if let Some(l) = &self.link {
                    l.download(job.id, ia, steps);
                }
                return;
            }
            let Some(ia) = proj.device(job.device).and_then(|d| knx::parse_ia(&d.address())) else {
                job.state = JobState::Failed;
                job.message = "The device has no valid individual address".into();
                return;
            };
            job.state = JobState::Running;
            job.message = "Starting…".into();
            if let Some(l) = &self.link {
                l.program_address(job.id, ia);
            }
            return;
        }
        match job.state {
            JobState::Waiting => {
                if matches!(job.kind, JobKind::Full | JobKind::Address) && !proj.device(job.device).map(|d| d.adr_ok).unwrap_or(false)
                    || job.kind == JobKind::Address
                {
                    job.state = JobState::PressButton;
                    job.timer = 3.0;
                    job.message = "Please press the programming button on the device".into();
                } else {
                    job.state = JobState::Running;
                    job.message = "Connecting…".into();
                }
            }
            JobState::PressButton => {
                job.timer -= dt;
                if job.timer <= 0.0 {
                    job.state = JobState::Running;
                    job.message = "Programming button detected (simulated)".into();
                }
            }
            JobState::Running => {
                let speed = match job.kind {
                    JobKind::Address => 0.9,
                    JobKind::Partial => 0.6,
                    JobKind::Unload => 0.8,
                    _ => 0.3,
                };
                job.progress = (job.progress + dt * speed).min(1.0);
                job.message = if !job.step_names.is_empty() {
                    let i = ((job.progress * job.step_names.len() as f32) as usize).min(job.step_names.len() - 1);
                    format!("{} (simulated)", job.step_names[i])
                } else { match job.progress {
                    p if p < 0.15 => "Connecting…".into(),
                    p if p < 0.35 => "Writing individual address…".into(),
                    p if p < 0.7 => "Writing application…".into(),
                    p if p < 0.9 => "Writing group address table…".into(),
                    _ => "Restarting device…".into(),
                } };
                if job.progress >= 1.0 {
                    job.state = JobState::Done;
                    job.message = "Finished".into();
                    if let Some(d) = proj.device_mut(job.device) {
                        match job.kind {
                            JobKind::Address => d.adr_ok = true,
                            JobKind::Full | JobKind::Application => {
                                d.adr_ok = true;
                                d.prg_ok = true;
                                d.par_ok = true;
                                d.grp_ok = true;
                                d.cfg_ok = true;
                            }
                            JobKind::Partial => {
                                d.par_ok = true;
                                d.grp_ok = true;
                                d.cfg_ok = true;
                            }
                            JobKind::Unload => {
                                d.prg_ok = false;
                                d.par_ok = false;
                                d.grp_ok = false;
                                d.cfg_ok = false;
                            }
                            JobKind::Check => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn fail_real_jobs(&mut self, msg: &str) {
        for j in self.jobs.iter_mut().filter(|j| j.real && matches!(j.state, JobState::Waiting | JobState::PressButton | JobState::Running)) {
            j.state = JobState::Failed;
            j.message = format!("Connection lost: {msg}");
        }
    }

    pub fn enqueue(&mut self, proj: &Project, device: u32, kind: JobKind) -> Result<(), String> {
        self.enqueue_plan(proj, device, kind, None)
    }

    pub fn enqueue_plan(&mut self, proj: &Project, device: u32, kind: JobKind, steps: Option<Vec<crate::knxload::Step>>) -> Result<(), String> {
        let Some(d) = proj.device(device) else { return Err("Device not found".into()) };
        if !d.has_app() {
            return Err(format!("{} is not programmable (no application program)", d.name));
        }
        if kind == JobKind::Partial && !d.adr_ok {
            return Err(format!("{}: program the individual address first", d.name));
        }
        if self.is_real() && kind != JobKind::Address && steps.is_none() {
            return Err("This device has no download data – import its product file (.knxprod) or the project first".into());
        }
        if self.jobs.iter().any(|j| j.project == proj.id && j.device == device && matches!(j.state, JobState::Waiting | JobState::PressButton | JobState::Running)) {
            return Err(format!("{} is already queued", d.name));
        }
        let id = self.next_job;
        self.next_job += 1;
        self.jobs.push(Job {
            id,
            project: proj.id,
            device,
            address: d.address(),
            name: d.name.clone(),
            kind,
            state: JobState::Waiting,
            progress: 0.0,
            timer: 0.0,
            message: "Waiting".into(),
            real: self.is_real(),
            step_names: steps.as_ref().map(|s| s.iter().map(|x| x.describe()).collect()).unwrap_or_default(),
            report: None,
            steps,
        });
        Ok(())
    }
}

pub fn format_value(dpt: &str, v: &str) -> String {
    let major = dpt.split('.').next().unwrap_or("");
    match (major, dpt) {
        ("1", d) if d.starts_with("1.008") => if v == "1" { "Down".into() } else { "Up".into() },
        ("1", d) if d.starts_with("1.007") => if v == "1" { "Increase".into() } else { "Decrease".into() },
        ("1", d) if d.starts_with("1.005") => if v == "1" { "Alarm".into() } else { "No alarm".into() },
        ("1", _) => if v == "1" { "On".into() } else { "Off".into() },
        ("3", _) => format!("Step {v}"),
        ("5", d) if d.starts_with("5.001") => format!("{v} %"),
        ("9", d) if d.starts_with("9.001") => format!("{v} °C"),
        ("9", d) if d.starts_with("9.004") => format!("{v} lx"),
        ("9", d) if d.starts_with("9.005") => format!("{v} m/s"),
        ("20", _) => match v {
            "1" => "Comfort".into(),
            "2" => "Standby".into(),
            "3" => "Economy".into(),
            "4" => "Building protection".into(),
            _ => "Auto".into(),
        },
        _ => v.to_string(),
    }
}
