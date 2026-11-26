//! Real bus access over KNXnet/IP: interface discovery (search), tunnelling and routing,
//! cEMI frame encoding/decoding, datapoint value conversion and individual address programming.
//!
//! The connection runs on its own thread; the UI talks to it through channels.

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

pub const PORT: u16 = 3671;
pub const MCAST: Ipv4Addr = Ipv4Addr::new(224, 0, 23, 12);

const SEARCH_REQ: u16 = 0x0201;
const SEARCH_RES: u16 = 0x0202;
const CONNECT_REQ: u16 = 0x0205;
const CONNECT_RES: u16 = 0x0206;
const CONNSTATE_REQ: u16 = 0x0207;
const CONNSTATE_RES: u16 = 0x0208;
const DISCONNECT_REQ: u16 = 0x0209;
const DISCONNECT_RES: u16 = 0x020A;
const TUNNEL_REQ: u16 = 0x0420;
const TUNNEL_ACK: u16 = 0x0421;
const ROUTING_IND: u16 = 0x0530;

pub const L_DATA_REQ: u8 = 0x11;
pub const L_DATA_CON: u8 = 0x2E;
pub const L_DATA_IND: u8 = 0x29;

// ------------------------------------------------------------------ addresses

pub fn fmt_ia(a: u16) -> String {
    format!("{}.{}.{}", a >> 12, (a >> 8) & 0x0f, a & 0xff)
}

pub fn parse_ia(s: &str) -> Option<u16> {
    let p: Vec<u16> = s.trim().split('.').map(|x| x.parse().ok()).collect::<Option<Vec<_>>>()?;
    if p.len() != 3 || p[0] > 15 || p[1] > 15 || p[2] > 255 {
        return None;
    }
    Some((p[0] << 12) | (p[1] << 8) | p[2])
}

pub fn fmt_ga3(a: u16) -> String {
    format!("{}/{}/{}", a >> 11, (a >> 8) & 0x07, a & 0xff)
}

// ------------------------------------------------------------------ frames

/// A link-layer telegram (cEMI L_Data).
#[derive(Clone, Debug)]
pub struct Frame {
    pub code: u8,
    pub ctrl1: u8,
    pub ctrl2: u8,
    pub src: u16,
    pub dst: u16,
    /// TPCI/APCI byte(s) followed by data.
    pub tpdu: Vec<u8>,
}

impl Frame {
    /// A new L_Data.req. `group`: destination is a group address.
    pub fn request(dst: u16, group: bool, tpdu: Vec<u8>) -> Self {
        Frame {
            code: L_DATA_REQ,
            ctrl1: 0xBC, // standard frame, no repeat, broadcast, low priority
            ctrl2: if group { 0xE0 } else { 0x60 }, // hop count 6
            src: 0,
            dst,
            tpdu,
        }
    }
    pub fn is_group(&self) -> bool {
        self.ctrl2 & 0x80 != 0
    }
    pub fn hops(&self) -> u8 {
        (self.ctrl2 >> 4) & 0x07
    }
    pub fn priority(&self) -> &'static str {
        match (self.ctrl1 >> 2) & 0x03 {
            0 => "System",
            1 => "Normal",
            2 => "Urgent",
            _ => "Low",
        }
    }
    /// Application-layer service, if the frame carries one.
    pub fn apci(&self) -> Option<u16> {
        if self.tpdu.len() < 2 {
            return None;
        }
        Some((((self.tpdu[0] & 0x03) as u16) << 8) | self.tpdu[1] as u16)
    }
    pub fn service(&self) -> u16 {
        self.apci().map(|a| a & 0x3C0).unwrap_or(0xFFFF)
    }
    /// Value payload: (bytes, small) where small means a ≤6-bit value packed into the APCI.
    pub fn value(&self) -> (Vec<u8>, bool) {
        if self.tpdu.len() == 2 {
            (vec![self.tpdu[1] & 0x3F], true)
        } else if self.tpdu.len() > 2 {
            (self.tpdu[2..].to_vec(), false)
        } else {
            (vec![], true)
        }
    }

    pub fn to_cemi(&self) -> Vec<u8> {
        let mut b = vec![self.code, 0x00, self.ctrl1, self.ctrl2];
        b.extend_from_slice(&self.src.to_be_bytes());
        b.extend_from_slice(&self.dst.to_be_bytes());
        b.push(self.tpdu.len().saturating_sub(1) as u8);
        b.extend_from_slice(&self.tpdu);
        b
    }

    pub fn from_cemi(b: &[u8]) -> Option<Frame> {
        if b.len() < 2 {
            return None;
        }
        let code = b[0];
        let i = 2 + b[1] as usize;
        if b.len() < i + 7 {
            return None;
        }
        let len = b[i + 6] as usize;
        let end = (i + 8 + len).min(b.len());
        Some(Frame {
            code,
            ctrl1: b[i],
            ctrl2: b[i + 1],
            src: u16::from_be_bytes([b[i + 2], b[i + 3]]),
            dst: u16::from_be_bytes([b[i + 4], b[i + 5]]),
            tpdu: b[(i + 7).min(end)..end].to_vec(),
        })
    }

    pub fn raw_hex(&self) -> String {
        self.to_cemi().iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
    }
}

pub fn service_label(s: u16) -> &'static str {
    match s {
        0x000 => "GroupValueRead",
        0x040 => "GroupValueResponse",
        0x080 => "GroupValueWrite",
        0x0C0 => "IndividualAddressWrite",
        0x100 => "IndividualAddressRead",
        0x140 => "IndividualAddressResponse",
        0x300 => "DeviceDescriptorRead",
        0x340 => "DeviceDescriptorResponse",
        0x380 => "Restart",
        0xFFFF => "Transport control",
        _ => "Other service",
    }
}

/// Group value telegram payload.
pub fn group_tpdu(service: u16, data: &[u8], small: bool) -> Vec<u8> {
    let hi = ((service >> 8) & 0x03) as u8;
    let lo = (service & 0xC0) as u8;
    if small {
        vec![hi, lo | (data.first().copied().unwrap_or(0) & 0x3F)]
    } else {
        let mut v = vec![hi, lo];
        v.extend_from_slice(data);
        v
    }
}

// ------------------------------------------------------------------ datapoint values

fn dpt_main(dpt: &str) -> &str {
    dpt.split(['.', ' ']).next().unwrap_or("")
}

pub fn encode_f16(v: f64) -> [u8; 2] {
    let mut m = (v * 100.0).round();
    let mut e = 0u8;
    while !(-2048.0..=2047.0).contains(&m) && e < 15 {
        m = (m / 2.0).round();
        e += 1;
    }
    let mi = m as i32;
    let sign: u16 = if mi < 0 { 0x8000 } else { 0 };
    let mant = (mi as u16) & 0x07FF;
    let w = sign | ((e as u16) << 11) | mant;
    w.to_be_bytes()
}

pub fn decode_f16(b: &[u8]) -> f64 {
    if b.len() < 2 {
        return 0.0;
    }
    let w = u16::from_be_bytes([b[0], b[1]]);
    let e = ((w >> 11) & 0x0F) as i32;
    let mut m = (w & 0x07FF) as i32;
    if w & 0x8000 != 0 {
        m -= 2048;
    }
    0.01 * m as f64 * 2f64.powi(e)
}

/// Turn a typed value into telegram data for the given datapoint type.
pub fn encode_value(dpt: &str, text: &str) -> Result<(Vec<u8>, bool), String> {
    let t = text.trim();
    let num = || t.replace(',', ".").trim_end_matches(['%', '°', 'C', ' ']).parse::<f64>().map_err(|_| format!("\"{t}\" is not a number"));
    match dpt_main(dpt) {
        "1" => {
            let v = match t.to_lowercase().as_str() {
                "1" | "on" | "true" | "yes" | "down" | "close" | "increase" | "alarm" => 1,
                "0" | "off" | "false" | "no" | "up" | "open" | "decrease" => 0,
                _ => return Err(format!("\"{t}\" is not a 1-bit value (use 0 or 1)")),
            };
            Ok((vec![v], true))
        }
        "3" => {
            let v = num()? as i64;
            if !(0..=15).contains(&v) {
                return Err("4-bit value must be 0…15".into());
            }
            Ok((vec![v as u8], true))
        }
        "5" => {
            let v = num()?;
            if dpt.starts_with("5.001") {
                if !(0.0..=100.0).contains(&v) {
                    return Err("Percentage must be 0…100".into());
                }
                Ok((vec![(v * 255.0 / 100.0).round() as u8], false))
            } else {
                if !(0.0..=255.0).contains(&v) {
                    return Err("Value must be 0…255".into());
                }
                Ok((vec![v as u8], false))
            }
        }
        "20" => {
            let v = num()?;
            Ok((vec![v.clamp(0.0, 255.0) as u8], false))
        }
        "9" => Ok((encode_f16(num()?).to_vec(), false)),
        "14" => Ok(((num()? as f32).to_be_bytes().to_vec(), false)),
        "16" => {
            let mut b: Vec<u8> = t.bytes().filter(|c| c.is_ascii()).take(14).collect();
            b.resize(14, 0);
            Ok((b, false))
        }
        _ => {
            let v = num()?;
            if (0.0..=63.0).contains(&v) && v.fract() == 0.0 {
                Ok((vec![v as u8], true))
            } else if (0.0..=255.0).contains(&v) {
                Ok((vec![v as u8], false))
            } else {
                Ok((encode_f16(v).to_vec(), false))
            }
        }
    }
}

/// A plain value string (as used by the simulation) from telegram data.
pub fn decode_value(dpt: &str, data: &[u8], small: bool) -> String {
    let first = data.first().copied().unwrap_or(0);
    match dpt_main(dpt) {
        "1" => format!("{}", first & 1),
        "3" => format!("{}", first & 0x0f),
        "5" if dpt.starts_with("5.001") => format!("{}", (first as f64 * 100.0 / 255.0).round()),
        "5" | "20" => format!("{first}"),
        "9" => format!("{:.2}", decode_f16(data)).trim_end_matches('0').trim_end_matches('.').to_string(),
        "14" if data.len() >= 4 => format!("{}", f32::from_be_bytes([data[0], data[1], data[2], data[3]])),
        "16" => String::from_utf8_lossy(data).trim_end_matches('\0').to_string(),
        _ => {
            if small {
                format!("{first}")
            } else {
                data.iter().map(|x| format!("${x:02X}")).collect::<Vec<_>>().join(" ")
            }
        }
    }
}

// ------------------------------------------------------------------ KNXnet/IP packets

fn header(service: u16, body_len: usize) -> Vec<u8> {
    let total = (6 + body_len) as u16;
    let mut b = vec![0x06, 0x10];
    b.extend_from_slice(&service.to_be_bytes());
    b.extend_from_slice(&total.to_be_bytes());
    b
}

fn hpai(ip: Ipv4Addr, port: u16) -> Vec<u8> {
    let mut b = vec![0x08, 0x01];
    b.extend_from_slice(&ip.octets());
    b.extend_from_slice(&port.to_be_bytes());
    b
}

fn packet(service: u16, body: &[u8]) -> Vec<u8> {
    let mut p = header(service, body.len());
    p.extend_from_slice(body);
    p
}

fn parse_packet(b: &[u8]) -> Option<(u16, &[u8])> {
    if b.len() < 6 || b[0] != 0x06 || b[1] != 0x10 {
        return None;
    }
    let svc = u16::from_be_bytes([b[2], b[3]]);
    let len = (u16::from_be_bytes([b[4], b[5]]) as usize).min(b.len());
    Some((svc, &b[6..len]))
}

fn connect_status(s: u8) -> String {
    match s {
        0x22 => "The interface does not support this connection type".into(),
        0x23 => "The interface does not support the requested options".into(),
        0x24 => "All tunnelling connections of the interface are in use".into(),
        0x29 => "The interface refused the connection (secure tunnelling or wrong address?)".into(),
        _ => format!("The interface refused the connection (status 0x{s:02X})"),
    }
}

// ------------------------------------------------------------------ discovery

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub name: String,
    pub addr: SocketAddrV4,
    pub ia: u16,
    pub tunnelling: bool,
    pub routing: bool,
    pub mac: String,
}

/// The local IPv4 address used to reach the KNX multicast group.
pub fn local_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect((MCAST, PORT)).ok()?;
    match s.local_addr().ok()? {
        SocketAddr::V4(a) if !a.ip().is_unspecified() => Some(*a.ip()),
        _ => None,
    }
}

/// Search the local network for KNXnet/IP interfaces (blocks for `wait`).
pub fn discover(wait: Duration) -> Result<Vec<Found>, String> {
    let ip = local_ip().ok_or("No network connection found")?;
    let sock = UdpSocket::bind((ip, 0)).map_err(|e| format!("Could not open a network socket: {e}"))?;
    let port = sock.local_addr().map_err(|e| e.to_string())?.port();
    let _ = sock.set_multicast_ttl_v4(4);
    sock.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let req = packet(SEARCH_REQ, &hpai(ip, port));
    sock.send_to(&req, (MCAST, PORT)).map_err(|e| format!("Could not send the search request: {e}"))?;
    let _ = sock.send_to(&req, (Ipv4Addr::BROADCAST, PORT));
    let mut out: Vec<Found> = vec![];
    let end = Instant::now() + wait;
    let mut buf = [0u8; 512];
    while Instant::now() < end {
        let Ok((n, _from)) = sock.recv_from(&mut buf) else { continue };
        let Some((svc, body)) = parse_packet(&buf[..n]) else { continue };
        if svc != SEARCH_RES || body.len() < 8 + 54 {
            continue;
        }
        let ctrl_ip = Ipv4Addr::new(body[2], body[3], body[4], body[5]);
        let ctrl_port = u16::from_be_bytes([body[6], body[7]]);
        let dib = &body[8..];
        let dib_len = dib[0] as usize;
        let ia = u16::from_be_bytes([dib[4], dib[5]]);
        let mac = dib[18..24].iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(":");
        let name = String::from_utf8_lossy(&dib[24..54]).trim_end_matches('\0').trim().to_string();
        let (mut tunnelling, mut routing) = (false, false);
        if dib.len() > dib_len + 2 {
            let svc_dib = &dib[dib_len..];
            let l = (svc_dib[0] as usize).min(svc_dib.len());
            for pair in svc_dib[2..l].chunks(2) {
                match pair[0] {
                    0x04 => tunnelling = true,
                    0x05 => routing = true,
                    _ => {}
                }
            }
        }
        let f = Found {
            name: if name.is_empty() { "KNXnet/IP interface".into() } else { name },
            addr: SocketAddrV4::new(ctrl_ip, if ctrl_port == 0 { PORT } else { ctrl_port }),
            ia,
            tunnelling,
            routing,
            mac,
        };
        if !out.iter().any(|x| x.addr == f.addr) {
            out.push(f);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ link thread

#[derive(Clone, Debug, PartialEq)]
pub enum Endpoint {
    Tunnel(SocketAddrV4),
    Routing,
}

pub enum Cmd {
    Send(Frame),
    ProgramAddress { job: u64, new_ia: u16 },
    Download { job: u64, ia: u16, steps: Vec<crate::knxload::Step> },
    Diag { job: u64, req: crate::diag::DiagReq },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    PressButton,
    Running,
    Done,
    Failed,
}

pub enum Event {
    Connected { ia: u16, detail: String },
    Closed(String),
    Failed(String),
    Frame(Frame),
    Job { job: u64, phase: Phase, progress: f32, msg: String },
    /// Result text of a device check.
    Report { job: u64, lines: Vec<String> },
    /// Progress or result of a diagnostic.
    Diag(crate::diag::DiagUpdate),
}

pub struct Link {
    tx: Sender<Cmd>,
    pub rx: Receiver<Event>,
}

impl Link {
    pub fn open(ep: Endpoint) -> Link {
        let (tx, crx) = channel();
        let (etx, rx) = channel();
        std::thread::spawn(move || {
            let res = match ep {
                Endpoint::Tunnel(addr) => Conn::tunnel(addr, etx.clone()),
                Endpoint::Routing => Conn::routing(etx.clone()),
            };
            match res {
                Ok(mut c) => c.run(crx),
                Err(e) => {
                    let _ = etx.send(Event::Failed(e));
                }
            }
        });
        Link { tx, rx }
    }
    pub fn send(&self, f: Frame) {
        let _ = self.tx.send(Cmd::Send(f));
    }
    pub fn program_address(&self, job: u64, new_ia: u16) {
        let _ = self.tx.send(Cmd::ProgramAddress { job, new_ia });
    }
    pub fn download(&self, job: u64, ia: u16, steps: Vec<crate::knxload::Step>) {
        let _ = self.tx.send(Cmd::Download { job, ia, steps });
    }
    pub fn diag(&self, job: u64, req: crate::diag::DiagReq) {
        let _ = self.tx.send(Cmd::Diag { job, req });
    }
    pub fn close(&self) {
        let _ = self.tx.send(Cmd::Close);
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Close);
    }
}

enum Mode {
    Tunnel { server: SocketAddrV4, data: SocketAddrV4, channel: u8, seq: u8, rx_seq: Option<u8> },
    Routing,
}

struct Conn {
    sock: UdpSocket,
    mode: Mode,
    ev: Sender<Event>,
    /// Frames received while waiting for something else (used by programming).
    inbox: VecDeque<Frame>,
    own_ia: u16,
    last_state: Instant,
    state_pending: Option<(Instant, u8)>,
    /// Telegram trace of the current download (for the log file).
    trace: Option<Vec<String>>,
    /// Findings of a device check.
    report: Vec<String>,
    check_diffs: usize,
}

impl Conn {
    fn tunnel(server: SocketAddrV4, ev: Sender<Event>) -> Result<Conn, String> {
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("Could not open a network socket: {e}"))?;
        sock.set_read_timeout(Some(Duration::from_millis(50))).ok();
        // NAT mode: all-zero endpoints, the interface answers to where the packet came from.
        let mut body = hpai(Ipv4Addr::UNSPECIFIED, 0);
        body.extend(hpai(Ipv4Addr::UNSPECIFIED, 0));
        body.extend([0x04, 0x04, 0x02, 0x00]); // tunnel connection, link layer
        let req = packet(CONNECT_REQ, &body);
        let mut buf = [0u8; 512];
        for _attempt in 0..3 {
            sock.send_to(&req, server).map_err(|e| format!("Could not reach {server}: {e}"))?;
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
                let Some((svc, b)) = parse_packet(&buf[..n]) else { continue };
                if svc != CONNECT_RES || b.len() < 2 {
                    continue;
                }
                if b[1] != 0 {
                    return Err(connect_status(b[1]));
                }
                if b.len() < 14 {
                    return Err("Unexpected answer from the interface".into());
                }
                let channel = b[0];
                let dip = Ipv4Addr::new(b[4], b[5], b[6], b[7]);
                let dport = u16::from_be_bytes([b[8], b[9]]);
                let data = if dip.is_unspecified() || dport == 0 { server } else { SocketAddrV4::new(dip, dport) };
                let ia = if b.len() >= 14 { u16::from_be_bytes([b[12], b[13]]) } else { 0 };
                let _ = ev.send(Event::Connected { ia, detail: format!("Tunnelling via {server}, channel {channel}") });
                return Ok(Conn {
                    sock,
                    mode: Mode::Tunnel { server, data, channel, seq: 0, rx_seq: None },
                    ev,
                    inbox: VecDeque::new(),
                    own_ia: ia,
                    last_state: Instant::now(),
                    state_pending: None,
            trace: None,
            report: vec![],
            check_diffs: 0,
                });
            }
        }
        Err(format!("No answer from {server}. Check the address and that the interface is switched on."))
    }

    fn routing(ev: Sender<Event>) -> Result<Conn, String> {
        use socket2::{Domain, Protocol, SockAddr, Socket, Type};
        let ip = local_ip().ok_or("No network connection found")?;
        let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|e| e.to_string())?;
        s.set_reuse_address(true).map_err(|e| e.to_string())?;
        s.bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT)))
            .map_err(|e| format!("Could not listen on port {PORT}: {e}"))?;
        s.join_multicast_v4(&MCAST, &ip).map_err(|e| format!("Could not join the routing multicast group: {e}"))?;
        s.set_multicast_if_v4(&ip).ok();
        s.set_multicast_loop_v4(false).ok();
        s.set_multicast_ttl_v4(16).ok();
        let sock: UdpSocket = s.into();
        sock.set_read_timeout(Some(Duration::from_millis(50))).ok();
        let _ = ev.send(Event::Connected { ia: 0, detail: format!("Routing on {MCAST}:{PORT} via {ip}") });
        Ok(Conn {
            sock,
            mode: Mode::Routing,
            ev,
            inbox: VecDeque::new(),
            own_ia: 0,
            last_state: Instant::now(),
            state_pending: None,
            trace: None,
            report: vec![],
            check_diffs: 0,
        })
    }

    fn run(&mut self, cmds: Receiver<Cmd>) {
        loop {
            match cmds.try_recv() {
                Ok(Cmd::Send(f)) => {
                    if let Err(e) = self.send_frame(&f) {
                        let _ = self.ev.send(Event::Failed(e));
                        return;
                    }
                }
                Ok(Cmd::ProgramAddress { job, new_ia }) => {
                    let r = self.program_address(job, new_ia);
                    let (phase, msg) = match r {
                        Ok(()) => (Phase::Done, format!("Individual address {} programmed", fmt_ia(new_ia))),
                        Err(e) => (Phase::Failed, e),
                    };
                    let _ = self.ev.send(Event::Job { job, phase, progress: 1.0, msg });
                }
                Ok(Cmd::Download { job, ia, steps }) => {
                    let r = self.download(job, ia, &steps);
                    if !self.report.is_empty() {
                        let _ = self.ev.send(Event::Report { job, lines: std::mem::take(&mut self.report) });
                    }
                    let (phase, msg) = match r {
                        Ok(m) => (Phase::Done, m),
                        Err(e) => (Phase::Failed, e),
                    };
                    let _ = self.ev.send(Event::Job { job, phase, progress: 1.0, msg });
                }
                Ok(Cmd::Diag { job, req }) => {
                    let done = self.diag(job, req);
                    let _ = self.ev.send(Event::Diag(crate::diag::DiagUpdate { job, progress: 1.0, done: Some(done), ..Default::default() }));
                }
                Ok(Cmd::Close) | Err(TryRecvError::Disconnected) => {
                    self.disconnect();
                    let _ = self.ev.send(Event::Closed("Disconnected".into()));
                    return;
                }
                Err(TryRecvError::Empty) => {}
            }
            if let Err(e) = self.pump(Duration::from_millis(50)) {
                let _ = self.ev.send(Event::Closed(e));
                return;
            }
            // Pass on whatever arrived.
            while let Some(f) = self.inbox.pop_front() {
                let _ = self.ev.send(Event::Frame(f));
            }
        }
    }

    fn ctrl_hpai(&self) -> Vec<u8> {
        hpai(Ipv4Addr::UNSPECIFIED, 0)
    }

    fn disconnect(&mut self) {
        if let Mode::Tunnel { server, channel, .. } = self.mode {
            let mut body = vec![channel, 0];
            body.extend(self.ctrl_hpai());
            let _ = self.sock.send_to(&packet(DISCONNECT_REQ, &body), server);
        }
    }

    /// Receive and handle packets for up to `dur`. Frames go to the inbox.
    /// Returns Err when the connection is lost.
    fn pump(&mut self, dur: Duration) -> Result<Option<u8>, String> {
        let end = Instant::now() + dur;
        let mut acked = None;
        let mut buf = [0u8; 1024];
        loop {
            // Heartbeat for tunnelling connections.
            if let Mode::Tunnel { server, channel, .. } = self.mode {
                if self.state_pending.is_none() && self.last_state.elapsed() > Duration::from_secs(60) {
                    let mut body = vec![channel, 0];
                    body.extend(self.ctrl_hpai());
                    let _ = self.sock.send_to(&packet(CONNSTATE_REQ, &body), server);
                    self.state_pending = Some((Instant::now(), 1));
                } else if let Some((t, tries)) = self.state_pending {
                    if t.elapsed() > Duration::from_secs(10) {
                        if tries >= 3 {
                            return Err("The interface stopped answering".into());
                        }
                        let mut body = vec![channel, 0];
                        body.extend(self.ctrl_hpai());
                        let _ = self.sock.send_to(&packet(CONNSTATE_REQ, &body), server);
                        self.state_pending = Some((Instant::now(), tries + 1));
                    }
                }
            }
            match self.sock.recv_from(&mut buf) {
                Ok((n, from)) => {
                    let pkt = buf[..n].to_vec();
                    if let Some(a) = self.handle(&pkt, from)? {
                        acked = Some(a);
                    }
                }
                Err(_) => {}
            }
            if Instant::now() >= end {
                return Ok(acked);
            }
        }
    }

    fn handle(&mut self, pkt: &[u8], _from: SocketAddr) -> Result<Option<u8>, String> {
        let Some((svc, b)) = parse_packet(pkt) else { return Ok(None) };
        match (&mut self.mode, svc) {
            (Mode::Tunnel { data, channel, rx_seq, .. }, TUNNEL_REQ) => {
                if b.len() < 4 || b[1] != *channel {
                    return Ok(None);
                }
                let seq = b[2];
                let ack = packet(TUNNEL_ACK, &[0x04, *channel, seq, 0x00]);
                let _ = self.sock.send_to(&ack, *data);
                if *rx_seq == Some(seq) {
                    return Ok(None); // repeated
                }
                *rx_seq = Some(seq);
                if let Some(f) = Frame::from_cemi(&b[4..]) {
                    if f.code == L_DATA_IND || f.code == L_DATA_CON {
                        if f.code == L_DATA_IND {
                            self.inbox.push_back(f);
                        }
                    }
                }
                Ok(None)
            }
            (Mode::Tunnel { channel, .. }, TUNNEL_ACK) => {
                if b.len() >= 4 && b[1] == *channel {
                    if b[3] != 0 {
                        return Ok(None);
                    }
                    return Ok(Some(b[2]));
                }
                Ok(None)
            }
            (Mode::Tunnel { .. }, CONNSTATE_RES) => {
                if b.len() >= 2 && b[1] != 0 {
                    return Err("The interface closed the connection".into());
                }
                self.state_pending = None;
                self.last_state = Instant::now();
                Ok(None)
            }
            (Mode::Tunnel { server, channel, .. }, DISCONNECT_REQ) => {
                let _ = self.sock.send_to(&packet(DISCONNECT_RES, &[*channel, 0]), *server);
                Err("The interface closed the connection".into())
            }
            (Mode::Routing, ROUTING_IND) => {
                if let Some(f) = Frame::from_cemi(b) {
                    if f.code == L_DATA_IND {
                        self.inbox.push_back(f);
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn send_frame(&mut self, f: &Frame) -> Result<(), String> {
        let (s, p, data) = match &mut self.mode {
            Mode::Routing => {
                let mut f = f.clone();
                f.code = L_DATA_IND;
                let p = packet(ROUTING_IND, &f.to_cemi());
                self.sock.send_to(&p, (MCAST, PORT)).map_err(|e| format!("Send failed: {e}"))?;
                return Ok(());
            }
            Mode::Tunnel { data, channel, seq, .. } => {
                let s = *seq;
                let mut body = vec![0x04, *channel, s, 0x00];
                body.extend(f.to_cemi());
                *seq = seq.wrapping_add(1);
                (s, packet(TUNNEL_REQ, &body), *data)
            }
        };
        for _ in 0..2 {
            self.sock.send_to(&p, data).map_err(|e| format!("Send failed: {e}"))?;
            let end = Instant::now() + Duration::from_secs(1);
            while Instant::now() < end {
                if self.pump(Duration::from_millis(20))? == Some(s) {
                    return Ok(());
                }
            }
        }
        Err("The interface did not confirm a telegram".into())
    }

    /// Collect frames for `dur`, keeping those that match `want`. Others are passed on to the UI.
    fn collect(&mut self, dur: Duration, want: impl Fn(&Frame) -> bool) -> Result<Vec<Frame>, String> {
        let end = Instant::now() + dur;
        let mut got = vec![];
        while Instant::now() < end {
            self.pump(Duration::from_millis(50))?;
            while let Some(f) = self.inbox.pop_front() {
                if want(&f) {
                    got.push(f.clone());
                }
                let _ = self.ev.send(Event::Frame(f));
            }
        }
        Ok(got)
    }

    fn job(&self, job: u64, phase: Phase, progress: f32, msg: &str) {
        let _ = self.ev.send(Event::Job { job, phase, progress, msg: msg.into() });
    }

    /// Devices currently in programming mode (answers to IndividualAddress_Read).
    fn read_prog_mode(&mut self) -> Result<Vec<u16>, String> {
        self.send_frame(&Frame::request(0x0000, true, vec![0x01, 0x00]))?;
        let got = self.collect(Duration::from_millis(2500), |f| f.service() == 0x140)?;
        let mut v: Vec<u16> = got.iter().map(|f| f.src).collect();
        v.sort();
        v.dedup();
        Ok(v)
    }

    /// Is any device answering at this address?
    fn address_in_use(&mut self, ia: u16) -> Result<bool, String> {
        self.send_frame(&Frame::request(ia, false, vec![0x80]))?; // T_Connect
        self.send_frame(&Frame::request(ia, false, vec![0x43, 0x00]))?; // DeviceDescriptor_Read, seq 0
        let got = self.collect(Duration::from_secs(2), |f| f.src == ia && (f.service() == 0x340 || f.tpdu.first().map(|t| t & 0xC3 == 0xC2).unwrap_or(false)))?;
        if let Some(r) = got.iter().find(|f| f.service() == 0x340) {
            let seq = (r.tpdu[0] >> 2) & 0x0f;
            let _ = self.send_frame(&Frame::request(ia, false, vec![0xC2 | (seq << 2)])); // T_ACK
        }
        let _ = self.send_frame(&Frame::request(ia, false, vec![0x81])); // T_Disconnect
        Ok(!got.is_empty())
    }

    fn program_address(&mut self, job: u64, new_ia: u16) -> Result<(), String> {
        let target = fmt_ia(new_ia);
        self.job(job, Phase::Running, 0.05, &format!("Checking whether {target} is free…"));
        if self.address_in_use(new_ia)? {
            // A device already answers at this address. It might be the one we want to program.
            let in_prog = self.read_prog_mode()?;
            if !in_prog.contains(&new_ia) {
                return Err(format!("Another device already uses {target}"));
            }
        }
        self.job(job, Phase::PressButton, 0.15, "Press the programming button on the device");
        let start = Instant::now();
        let found = loop {
            let devs = self.read_prog_mode()?;
            match devs.len() {
                0 => {}
                1 => break devs[0],
                n => return Err(format!("{n} devices are in programming mode – only one may be")),
            }
            if start.elapsed() > Duration::from_secs(60) {
                return Err("No device was put into programming mode within 60 seconds".into());
            }
        };
        self.job(job, Phase::Running, 0.5, &format!("Device found (currently {}). Writing {target}…", fmt_ia(found)));
        if found != new_ia {
            let [hi, lo] = new_ia.to_be_bytes();
            self.send_frame(&Frame::request(0x0000, true, vec![0x00, 0xC0, hi, lo]))?;
        }
        self.job(job, Phase::Running, 0.7, "Checking the new address…");
        let after = self.read_prog_mode()?;
        if !after.contains(&new_ia) {
            return Err(format!("The device did not take the address {target}"));
        }
        self.job(job, Phase::Running, 0.9, "Restarting the device…");
        self.send_frame(&Frame::request(new_ia, false, vec![0x80]))?; // T_Connect
        self.send_frame(&Frame::request(new_ia, false, vec![0x43, 0x80]))?; // A_Restart
        let _ = self.collect(Duration::from_millis(500), |_| false);
        let _ = self.send_frame(&Frame::request(new_ia, false, vec![0x81])); // T_Disconnect
        let _ = self.own_ia;
        Ok(())
    }
}

// ------------------------------------------------------------------ application download

fn ga_text(g: u16) -> String {
    fmt_ga3(g)
}

/// Compare one table / memory part read from the device with the expected content and
/// describe it in plain lines. Lines with "⚠" count as differences.
pub(crate) fn check_report(obj: u8, base: u32, dev: &[u8], exp: &[u8]) -> Vec<String> {
    let name = match obj {
        1 => "Group address table",
        2 => "Association table (links)",
        3 => "Group object table",
        _ => "Application memory (parameters)",
    };
    let mut out = vec![String::new(), format!("{name} at {base:06X}:")];
    if base == 0 || dev.is_empty() {
        out.push("  ⚠ the device has no memory reserved for it (not loaded)".into());
        return out;
    }
    let u16s = |b: &[u8]| -> Vec<u16> { b.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect() };
    match obj {
        1 => {
            let d = u16s(dev);
            let n = d.first().copied().unwrap_or(0) as usize;
            let list: Vec<String> = d.iter().skip(1).take(n).map(|g| ga_text(*g)).collect();
            out.push(format!("  In the device: {n} group address(es): {}", if list.is_empty() { "none".into() } else { list.join(", ") }));
        }
        2 => {
            let d = u16s(dev);
            let n = d.first().copied().unwrap_or(0) as usize;
            out.push(format!("  In the device: {n} link(s) (connection number → group object):"));
            for p in d.get(1..).unwrap_or(&[]).chunks(2).take(n.min(40)) {
                if p.len() == 2 {
                    out.push(format!("    {} → object {}", p[0], p[1]));
                }
            }
        }
        3 => {
            let d = u16s(dev);
            let n = d.first().copied().unwrap_or(0) as usize;
            let active: Vec<String> = d.iter().skip(1).take(n).enumerate().filter(|(_, x)| (*x >> 8) & 0x04 != 0).map(|(i, _)| (i + 1).to_string()).collect();
            out.push(format!("  In the device: {n} group objects, communication enabled for: {}", if active.is_empty() { "none".into() } else { active.join(", ") }));
        }
        _ => {}
    }
    let diffs: Vec<usize> = (0..exp.len()).filter(|i| dev.get(*i) != Some(&exp[*i])).collect();
    if diffs.is_empty() {
        out.push(format!("  ✔ identical to the project ({} bytes)", exp.len()));
    } else {
        out.push(format!("  ⚠ {} of {} bytes differ from the project", diffs.len(), exp.len()));
        // Group into ranges
        let mut ranges: Vec<(usize, usize)> = vec![];
        for i in diffs {
            match ranges.last_mut() {
                Some((_, e)) if *e + 1 == i => *e = i,
                _ => ranges.push((i, i)),
            }
        }
        for (s, e) in ranges.iter().take(12) {
            let show = |b: &[u8]| b.iter().take(8).map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
            out.push(format!(
                "    offset {s:04X}–{e:04X}: device {} | project {}",
                show(&dev[*s..(*e + 1).min(dev.len())]),
                show(&exp[*s..*e + 1])
            ));
        }
        if ranges.len() > 12 {
            out.push(format!("    … {} more ranges", ranges.len() - 12));
        }
    }
    out
}

/// Save the telegram trace of a download; returns the file path.
fn write_download_log(ia: u16, lines: &[String]) -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    let dir = crate::app::data_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("download-{}-{}.txt", fmt_ia(ia).replace('.', "-"), crate::model::fmt_time(secs).replace([' ', ':'], "_")));
    std::fs::write(&path, lines.join("\r\n")).ok()?;
    Some(path)
}

/// What answer a request waits for.
#[derive(Clone, Copy)]
enum Want {
    /// Exact application service code
    Exact(u16),
    /// Service class (upper four bits of the code)
    Class(u16),
}

struct Tc {
    dst: u16,
    seq: u8,
    last_rx: Option<u8>,
}

const LS_UNLOADED: u8 = 0;
const LS_LOADED: u8 = 1;
const LS_LOADING: u8 = 2;
const LS_ERROR: u8 = 3;

fn ls_name(s: u8) -> &'static str {
    match s {
        0 => "unloaded",
        1 => "loaded",
        2 => "loading",
        3 => "error",
        4 => "unloading",
        5 => "completing",
        _ => "unknown",
    }
}

impl Conn {
    /// Wait for a frame matching `pred`; other frames are passed on to the monitor.
    fn wait_for(&mut self, dur: Duration, pred: impl Fn(&Frame) -> bool) -> Result<Option<Frame>, String> {
        let end = Instant::now() + dur;
        while Instant::now() < end {
            self.pump(Duration::from_millis(15))?;
            while let Some(f) = self.inbox.pop_front() {
                if pred(&f) {
                    if let Some(tr) = &mut self.trace {
                        tr.push(format!("← {}  {}", fmt_ia(f.src), f.tpdu.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")));
                    }
                    return Ok(Some(f));
                }
                let _ = self.ev.send(Event::Frame(f));
            }
        }
        Ok(None)
    }

    fn tc_send(&mut self, dst: u16, tpdu: Vec<u8>) -> Result<(), String> {
        let f = Frame::request(dst, false, tpdu);
        if let Some(tr) = &mut self.trace {
            tr.push(format!("→ {}  {}", fmt_ia(dst), f.tpdu.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")));
        }
        self.send_frame(&f)
    }

    /// Send one connection-oriented request and (optionally) wait for its answer.
    fn tc_request(&mut self, tc: &mut Tc, apdu: &[u8], want: Option<Want>) -> Result<Option<Frame>, String> {
        let dst = tc.dst;
        for _attempt in 0..3 {
            let mut t = apdu.to_vec();
            t[0] = (t[0] & 0x03) | 0x40 | (tc.seq << 2);
            self.tc_send(dst, t)?;
            let seq = tc.seq;
            // Transport acknowledgement (T_ACK / T_NAK)
            let ack = self.wait_for(Duration::from_secs(3), |f| f.src == dst && !f.is_group() && f.tpdu.len() == 1 && f.tpdu[0] & 0xC0 == 0xC0)?;
            match ack {
                Some(f) if f.tpdu[0] & 0x03 == 0x02 && (f.tpdu[0] >> 2) & 0x0f == seq => {}
                Some(f) if f.tpdu[0] & 0x03 == 0x03 => continue, // NAK: repeat
                _ => continue,
            }
            tc.seq = (tc.seq + 1) & 0x0f;
            let Some(want) = want else { return Ok(None) };
            let end = Instant::now() + Duration::from_secs(4);
            while Instant::now() < end {
                let r = self.wait_for(Duration::from_millis(200), |f| f.src == dst && !f.is_group() && f.tpdu.len() >= 2 && f.tpdu[0] & 0xC0 == 0x40)?;
                let Some(r) = r else { continue };
                let rs = (r.tpdu[0] >> 2) & 0x0f;
                self.tc_send(dst, vec![0xC2 | (rs << 2)])?;
                if tc.last_rx == Some(rs) {
                    continue; // repeated answer
                }
                tc.last_rx = Some(rs);
                let apci = r.apci().unwrap_or(0);
                let ok = match want {
                    Want::Exact(c) => apci == c,
                    Want::Class(c) => apci & 0x3C0 == c,
                };
                if ok {
                    return Ok(Some(r));
                }
            }
            return Err("The device did not answer".into());
        }
        Err("The device does not acknowledge (is the address right and the device connected?)".into())
    }

    fn prop_read(&mut self, tc: &mut Tc, obj: u8, pid: u8, start: u16, count: u8) -> Result<Vec<u8>, String> {
        let apdu = [0x03, 0xD5, obj, pid, (count << 4) | ((start >> 8) as u8 & 0x0f), start as u8];
        let r = self.tc_request(tc, &apdu, Some(Want::Exact(0x3D6)))?.ok_or("no answer")?;
        if r.tpdu.len() < 6 || r.tpdu[4] >> 4 == 0 {
            return Err(format!("The device has no property {pid} in object {obj}"));
        }
        Ok(r.tpdu[6..].to_vec())
    }

    fn prop_write(&mut self, tc: &mut Tc, obj: u8, pid: u8, start: u16, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut apdu = vec![0x03, 0xD7, obj, pid, (1 << 4) | ((start >> 8) as u8 & 0x0f), start as u8];
        apdu.extend_from_slice(data);
        let r = self.tc_request(tc, &apdu, Some(Want::Exact(0x3D6)))?.ok_or("no answer")?;
        if r.tpdu.len() < 6 || r.tpdu[4] >> 4 == 0 {
            return Err(format!("The device refused writing property {pid} of object {obj}"));
        }
        Ok(r.tpdu[6..].to_vec())
    }

    /// Write a property without checking the answer. The product marks such writes with
    /// Verify="false": the device's answer is a read-back, which write-only properties
    /// (like the memory control table, property 27) answer negatively even though the
    /// write succeeded. Problems show up later in the load state checks.
    fn prop_write_unchecked(&mut self, tc: &mut Tc, obj: u8, pid: u8, start: u16, data: &[u8]) -> Result<(), String> {
        let mut apdu = vec![0x03, 0xD7, obj, pid, (1 << 4) | ((start >> 8) as u8 & 0x0f), start as u8];
        apdu.extend_from_slice(data);
        match self.tc_request(tc, &apdu, Some(Want::Exact(0x3D6))) {
            Ok(_) => Ok(()),
            Err(e) if e == "The device did not answer" => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn mem_write(&mut self, tc: &mut Tc, addr: u32, data: &[u8]) -> Result<(), String> {
        if addr + data.len() as u32 <= 0x1_0000 {
            let mut apdu = vec![0x02, 0x80 | data.len() as u8, (addr >> 8) as u8, addr as u8];
            apdu.extend_from_slice(data);
            self.tc_request(tc, &apdu, None)?;
            Ok(())
        } else {
            let mut apdu = vec![0x01, 0xFB, data.len() as u8, (addr >> 16) as u8, (addr >> 8) as u8, addr as u8];
            apdu.extend_from_slice(data);
            let r = self.tc_request(tc, &apdu, Some(Want::Exact(0x1FC)))?.ok_or("no answer")?;
            if r.tpdu.get(2).copied().unwrap_or(1) != 0 {
                return Err(format!("The device refused writing memory at {addr:06X}"));
            }
            Ok(())
        }
    }

    fn mem_read(&mut self, tc: &mut Tc, addr: u32, n: usize) -> Result<Vec<u8>, String> {
        if addr + n as u32 <= 0x1_0000 {
            let apdu = [0x02, n as u8 & 0x3f, (addr >> 8) as u8, addr as u8];
            // A_Memory_Response is 0x240 | count
            let r = self.tc_request(tc, &apdu, Some(Want::Class(0x240)))?.ok_or("no answer")?;
            Ok(r.tpdu.get(4..).map(|x| x.to_vec()).unwrap_or_default())
        } else {
            let apdu = [0x01, 0xFD, n as u8, (addr >> 16) as u8, (addr >> 8) as u8, addr as u8];
            let r = self.tc_request(tc, &apdu, Some(Want::Exact(0x1FE)))?.ok_or("no answer")?;
            if r.tpdu.get(2).copied().unwrap_or(1) != 0 {
                return Err(format!("The device refused reading memory at {addr:06X}"));
            }
            let d = &r.tpdu[..];
            Ok(d[d.len().saturating_sub(n)..].to_vec())
        }
    }

    /// Write a load event to a load state machine and return the new state.
    fn load_event(&mut self, tc: &mut Tc, lsm: u8, data: [u8; 10]) -> Result<u8, String> {
        let r = self.prop_write(tc, lsm, 5, 1, &data)?;
        Ok(r.first().copied().unwrap_or(0xff))
    }

    fn load_state(&mut self, tc: &mut Tc, lsm: u8, event: u8, extra: &[u8], expect: &[u8], what: &str) -> Result<(), String> {
        let mut data = [0u8; 10];
        data[0] = event;
        for (i, b) in extra.iter().enumerate().take(9) {
            data[i + 1] = *b;
        }
        let mut st = self.load_event(tc, lsm, data)?;
        // Completing can take a moment: poll the state.
        let end = Instant::now() + Duration::from_secs(10);
        while !expect.contains(&st) && st != LS_ERROR && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(300));
            st = self.prop_read(tc, lsm, 5, 1, 1)?.first().copied().unwrap_or(0xff);
        }
        if expect.contains(&st) {
            Ok(())
        } else {
            Err(format!("{what}: the device reports state \"{}\"", ls_name(st)))
        }
    }

    pub(crate) fn download(&mut self, job: u64, ia: u16, steps: &[crate::knxload::Step]) -> Result<String, String> {
        let mut tc = Tc { dst: ia, seq: 0, last_rx: None };
        let is_check = steps.iter().any(|s| matches!(s, crate::knxload::Step::CheckMem { .. }));
        self.report.clear();
        self.check_diffs = 0;
        self.trace = Some(vec![format!("Download to {} – {} steps", fmt_ia(ia), steps.len())]);
        let mut step_no = 0usize;
        let r = self.download_steps(job, &mut tc, steps, &mut step_no);
        if r.is_err() {
            let _ = self.tc_send(ia, vec![0x81]);
        }
        let mut lines = self.trace.take().unwrap_or_default();
        match &r {
            Ok(()) => lines.push("Result: finished".into()),
            Err(e) => lines.push(format!("Result: failed in step {} – {e}", step_no + 1)),
        }
        if !self.report.is_empty() {
            lines.push(String::new());
            lines.extend(self.report.iter().cloned());
        }
        let log = write_download_log(ia, &lines);
        let r = r.map(|()| {
            if is_check {
                if self.check_diffs == 0 { "Device check: everything matches the project".to_string() } else { format!("Device check: {} difference(s) found – see the report", self.check_diffs) }
            } else {
                "Download finished".to_string()
            }
        });
        r.map_err(|e| {
            let step = steps.get(step_no).map(|s| s.describe()).unwrap_or_default();
            match log {
                Some(p) => format!("Step {} ({step}): {e}. Details: {}", step_no + 1, p.display()),
                None => format!("Step {} ({step}): {e}", step_no + 1),
            }
        })
    }

    fn download_steps(&mut self, job: u64, tc: &mut Tc, steps: &[crate::knxload::Step], step_no: &mut usize) -> Result<(), String> {
        use crate::knxload::Step;
        let total: u64 = steps.iter().map(|s| s.weight()).sum::<u64>().max(1);
        let mut done = 0u64;
        let mut images: HashMap<(u8, u8), Vec<u8>> = HashMap::new();
        let target = fmt_ia(tc.dst);
        for (i, s) in steps.iter().enumerate() {
            *step_no = i;
            if let Some(tr) = &mut self.trace {
                tr.push(format!("--- Step {}: {}", i + 1, s.describe()));
            }
            let p = done as f32 / total as f32;
            self.job(job, Phase::Running, p, &s.describe());
            match s {
                Step::Connect => {
                    self.tc_send(tc.dst, vec![0x80])?;
                    tc.seq = 0;
                    tc.last_rx = None;
                }
                Step::CheckMask(m) => {
                    let r = self.tc_request(tc, &[0x03, 0x00], Some(Want::Class(0x340)))?.ok_or("no answer")?;
                    let got = r.tpdu.get(2..4).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0);
                    if got != *m {
                        return Err(format!("The device at {target} is a different type (mask {got:04X}) than the product (mask {m:04X}). Nothing was changed."));
                    }
                }
                Step::Authorize => {
                    // Default key; devices without access protection accept it.
                    let _ = self.tc_request(tc, &[0x03, 0xD1, 0x00, 0xFF, 0xFF, 0xFF, 0xFF], Some(Want::Exact(0x3D2)));
                }
                Step::Unload { lsm, ignore_errors } => match self.load_state(tc, *lsm, 4, &[], &[LS_UNLOADED, 4], "Unloading") {
                    Err(_) if *ignore_errors => {}
                    r => r?,
                },
                Step::Load { lsm } => self.load_state(tc, *lsm, 1, &[], &[LS_LOADING], "Start loading")?,
                Step::LoadCompleted { lsm } => self.load_state(tc, *lsm, 2, &[], &[LS_LOADED], "Finishing")?,
                Step::Alloc { lsm, size, mode, fill } => {
                    let s = size.to_be_bytes();
                    self.load_state(tc, *lsm, 3, &[0x0B, s[0], s[1], s[2], s[3], *mode, *fill, 0, 0], &[LS_LOADING], "Reserving memory")?;
                }
                Step::WriteProp { obj, pid, start, data, verify } => {
                    if *verify {
                        let echo = self.prop_write(tc, *obj, *pid, *start, data)?;
                        if echo.len() >= data.len() && echo[..data.len()] != data[..] {
                            return Err(format!("Property {pid} of object {obj} could not be verified"));
                        }
                    } else {
                        self.prop_write_unchecked(tc, *obj, *pid, *start, data)?;
                    }
                }
                Step::PatchProp { obj, pid, bit, bits, value } => {
                    let mut cur = self.prop_read(tc, *obj, *pid, 1, 1).unwrap_or_default();
                    crate::knxload::patch_prop(&mut cur, *bit, *bits, value)?;
                    self.prop_write(tc, *obj, *pid, 1, &cur)?;
                }
                Step::ImageProp { obj, pid, count } => {
                    let v = self.prop_read(tc, *obj, *pid, 1, *count).unwrap_or_default();
                    images.insert((*obj, *pid), v);
                }
                Step::CompareProp { obj, pid, start, data } => {
                    let cur = self.prop_read(tc, *obj, *pid, *start, 1)?;
                    let expected = images.get(&(*obj, *pid)).cloned().unwrap_or_else(|| data.clone());
                    if cur.len() >= expected.len() && cur[..expected.len()] != expected[..] {
                        return Err("The device's memory layout differs from the last download – please run a full download".into());
                    }
                }
                Step::WriteMem { obj, offset, data, verify, skip_fill } => {
                    let ptr = self.prop_read(tc, *obj, 7, 1, 1)?;
                    if ptr.len() < 2 {
                        return Err("The device reported no memory address".into());
                    }
                    let base = if ptr.len() >= 4 { u32::from_be_bytes([ptr[0], ptr[1], ptr[2], ptr[3]]) } else { u16::from_be_bytes([ptr[0], ptr[1]]) as u32 };
                    if base == 0 {
                        return Err("The device has no memory reserved for this part".into());
                    }
                    let start = base + offset;
                    let chunk = if start + data.len() as u32 <= 0x1_0000 { 12 } else { 10 };
                    let mut i = 0usize;
                    let mut n = 0usize;
                    while i < data.len() {
                        let end = (i + chunk).min(data.len());
                        let part = &data[i..end];
                        if skip_fill.map(|f| part.iter().all(|b| *b == f)).unwrap_or(false) {
                            i = end;
                            continue;
                        }
                        let addr = start + i as u32;
                        self.mem_write(tc, addr, part)?;
                        if *verify {
                            let back = self.mem_read(tc, addr, part.len())?;
                            if back.len() < part.len() || back[..part.len()] != *part {
                                return Err(format!("Memory at {addr:06X} could not be verified"));
                            }
                        }
                        i = end;
                        n += 1;
                        if n % 8 == 0 {
                            let p = (done + i as u64) as f32 / total as f32;
                            self.job(job, Phase::Running, p, &format!("{} – {} of {} bytes", s.describe(), i, data.len()));
                        }
                    }
                }
                Step::ReadState { lsm } => {
                    let name = match lsm {
                        1 => "Group address table",
                        2 => "Association table",
                        3 => "Group object table",
                        _ => "Application",
                    };
                    match self.prop_read(tc, *lsm, 5, 1, 1) {
                        Ok(v) => {
                            let st = v.first().copied().unwrap_or(0xff);
                            if st != LS_LOADED {
                                self.check_diffs += 1;
                            }
                            self.report.push(format!("{name}: {}{}", ls_name(st), if st == LS_LOADED { "" } else { "  ⚠ not loaded" }));
                        }
                        Err(e) => {
                            self.check_diffs += 1;
                            self.report.push(format!("{name}: state could not be read ({e})"));
                        }
                    }
                }
                Step::CheckProp { obj, pid, data } => {
                    let cur = self.prop_read(tc, *obj, *pid, 1, 1).unwrap_or_default();
                    let hexs = |b: &[u8]| b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
                    if cur.len() >= data.len() && cur[..data.len()] == data[..] {
                        self.report.push(format!("Application id in the device: {} – matches the project", hexs(&cur)));
                    } else {
                        self.check_diffs += 1;
                        self.report.push(format!("Application id in the device: {} – the project expects {}  ⚠", hexs(&cur), hexs(data)));
                    }
                }
                Step::CheckMem { obj, data } => {
                    let ptr = self.prop_read(tc, *obj, 7, 1, 1)?;
                    let base = match ptr.len() {
                        n if n >= 4 => u32::from_be_bytes([ptr[0], ptr[1], ptr[2], ptr[3]]),
                        2 | 3 => u16::from_be_bytes([ptr[0], ptr[1]]) as u32,
                        _ => 0,
                    };
                    let mut dev_bytes = vec![];
                    if base != 0 {
                        let chunk = if base + data.len() as u32 <= 0x1_0000 { 12 } else { 10 };
                        let mut i = 0usize;
                        while i < data.len() {
                            let n = chunk.min(data.len() - i);
                            let part = self.mem_read(tc, base + i as u32, n)?;
                            dev_bytes.extend_from_slice(&part[..n.min(part.len())]);
                            i += n;
                            if i % 240 == 0 {
                                let p = (done + i as u64) as f32 / total as f32;
                                self.job(job, Phase::Running, p, &format!("{} – {} of {} bytes", s.describe(), i, data.len()));
                            }
                        }
                    }
                    let lines = check_report(*obj, base, &dev_bytes, data);
                    if lines.iter().any(|l| l.contains('⚠')) {
                        self.check_diffs += 1;
                    }
                    self.report.extend(lines);
                }
                Step::Restart => {
                    let _ = self.tc_request(tc, &[0x03, 0x80], None);
                    let _ = self.tc_send(tc.dst, vec![0x81]);
                }
                Step::Disconnect => {
                    self.tc_send(tc.dst, vec![0x81])?;
                }
            }
            done += s.weight();
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ diagnostics

impl Conn {
    fn diag_update(&self, job: u64, progress: f32, msg: &str, info: Vec<(String, String)>, hits: Vec<crate::diag::ScanHit>) {
        let _ = self.ev.send(Event::Diag(crate::diag::DiagUpdate { job, progress, msg: msg.into(), info, hits, done: None }));
    }

    /// Run one diagnostic; returns the summary for the result line.
    fn diag(&mut self, job: u64, req: crate::diag::DiagReq) -> Result<String, String> {
        use crate::diag::{DiagReq, ScanHit};
        match req {
            DiagReq::DeviceInfo(ia) => self.device_info(job, ia),
            DiagReq::ProgScan => {
                let v = self.read_prog_mode()?;
                let hits = v.iter().map(|a| ScanHit { ia: *a, mask: None }).collect();
                self.diag_update(job, 0.9, "", vec![], hits);
                Ok(match v.len() {
                    0 => "No device is in programming mode".into(),
                    1 => "1 device is in programming mode".into(),
                    n => format!("{n} devices are in programming mode"),
                })
            }
            DiagReq::SetProg(ia, on) => self.set_prog_mode(ia, on),
            DiagReq::Check(ia) => match self.probe(ia)? {
                None => Ok(format!("No device answers at {} – the address is free", fmt_ia(ia))),
                Some(mask) => {
                    let info = mask.map(|m| vec![("Mask version".to_string(), crate::diag::mask_text(m))]).unwrap_or_default();
                    self.diag_update(job, 0.9, "", info, vec![ScanHit { ia, mask }]);
                    Ok(format!("A device answers at {}", fmt_ia(ia)))
                }
            },
            DiagReq::LineScan { area, line, cancel } => {
                let mut found = 0usize;
                for n in 0..=255u16 {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return Ok(format!("Stopped at {area}.{line}.{n} – {found} device(s) found"));
                    }
                    let ia = ((area as u16) << 12) | ((line as u16) << 8) | n;
                    let hit = self.probe(ia)?.map(|mask| ScanHit { ia, mask });
                    if hit.is_some() {
                        found += 1;
                    }
                    self.diag_update(job, (n + 1) as f32 / 256.0, &format!("Scanning {} – {found} found", fmt_ia(ia)), vec![], hit.into_iter().collect());
                }
                Ok(format!("Line {area}.{line} scanned: {found} device(s) answered"))
            }
        }
    }

    /// Is there a device at `ia`? Some(mask version if it told us) or None.
    /// Opens a connection, asks for the device descriptor and closes again.
    fn probe(&mut self, ia: u16) -> Result<Option<Option<u16>>, String> {
        use crate::diag::{PROBE_ACK_WAIT_MS, PROBE_ANSWER_WAIT_MS};
        self.send_frame(&Frame::request(ia, false, vec![0x80]))?; // T_Connect
        self.send_frame(&Frame::request(ia, false, vec![0x43, 0x00]))?; // DeviceDescriptor_Read, seq 0
        let mut acked = false;
        let mut mask = None;
        let mut end = Instant::now() + Duration::from_millis(PROBE_ACK_WAIT_MS);
        while Instant::now() < end && mask.is_none() {
            let Some(f) = self.wait_for(Duration::from_millis(30), |f| f.src == ia && !f.is_group())? else { continue };
            if f.tpdu.len() == 1 && f.tpdu[0] & 0xC3 == 0xC2 {
                if !acked {
                    acked = true;
                    end = Instant::now() + Duration::from_millis(PROBE_ANSWER_WAIT_MS);
                }
            } else if f.tpdu.len() == 1 && f.tpdu[0] == 0x81 {
                acked = true; // busy with another connection, but it exists
                break;
            } else if f.service() == 0x340 && f.tpdu.len() >= 4 {
                mask = Some(u16::from_be_bytes([f.tpdu[2], f.tpdu[3]]));
                let seq = (f.tpdu[0] >> 2) & 0x0f;
                let _ = self.send_frame(&Frame::request(ia, false, vec![0xC2 | (seq << 2)])); // T_ACK
            }
        }
        let _ = self.send_frame(&Frame::request(ia, false, vec![0x81])); // T_Disconnect
        Ok(if acked || mask.is_some() { Some(mask) } else { None })
    }

    fn device_info(&mut self, job: u64, ia: u16) -> Result<String, String> {
        use crate::diag::*;
        let target = fmt_ia(ia);
        let mut tc = Tc { dst: ia, seq: 0, last_rx: None };
        self.tc_send(ia, vec![0x80])?;
        let r = self
            .tc_request(&mut tc, &[0x03, 0x00], Some(Want::Class(0x340)))
            .map_err(|_| format!("No device answers at {target}"))?
            .ok_or_else(|| format!("No device answers at {target}"))?;
        let mask = r.tpdu.get(2..4).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0);
        let rows = vec![("Individual address".to_string(), target.clone()), ("Mask version".to_string(), mask_text(mask))];
        self.diag_update(job, 0.15, "Reading identity…", rows, vec![]);
        let na = || "not available".to_string();
        let result = (|| -> Result<(), String> {
            if uses_properties(mask) {
                let steps: [(&str, u8); 6] = [("Manufacturer", 12), ("Order number", 15), ("Serial number", 11), ("Hardware type", 78), ("Firmware revision", 9), ("Programming mode", 54)];
                for (i, (name, pid)) in steps.iter().enumerate() {
                    let text = match (self.prop_read(&mut tc, 0, *pid, 1, 1), *pid) {
                        (Ok(b), 12) if b.len() >= 2 => format!("M-{:04X}", u16::from_be_bytes([b[0], b[1]])),
                        (Ok(b), 15) if !b.is_empty() => order_text(&b),
                        (Ok(b), 11) if !b.is_empty() => serial_text(&b),
                        (Ok(b), 9) if !b.is_empty() => b[0].to_string(),
                        (Ok(b), 54) if !b.is_empty() => if b[0] & 1 != 0 { "On".into() } else { "Off".into() },
                        (Ok(b), _) if !b.is_empty() => hex(&b),
                        _ => na(),
                    };
                    self.diag_update(job, 0.2 + i as f32 * 0.08, &format!("Reading {}…", name.to_lowercase()), vec![(name.to_string(), text)], vec![]);
                }
                let app = match self.prop_read(&mut tc, 4, 13, 1, 1) {
                    Ok(b) if b.len() >= 5 => format!("M-{:04X}, number {:04X}, version {}.{}", u16::from_be_bytes([b[0], b[1]]), u16::from_be_bytes([b[2], b[3]]), b[4] >> 4, b[4] & 0x0f),
                    Ok(b) if !b.is_empty() => hex(&b),
                    _ => na(),
                };
                self.diag_update(job, 0.75, "Reading load states…", vec![("Application program".into(), app)], vec![]);
                for (name, obj) in [("Group address table", 1u8), ("Association table", 2), ("Application", 4)] {
                    let s = match self.prop_read(&mut tc, obj, 5, 1, 1) {
                        Ok(b) if !b.is_empty() => load_state_text(b[0]),
                        _ => na(),
                    };
                    self.diag_update(job, 0.85, "", vec![(name.into(), s)], vec![]);
                }
            } else {
                // System 1/2: identity and programming mode live at fixed memory addresses.
                let prog = match self.mem_read(&mut tc, 0x0060, 1) {
                    Ok(b) if !b.is_empty() => if b[0] & 1 != 0 { "On".into() } else { "Off".into() },
                    _ => na(),
                };
                let app = match self.mem_read(&mut tc, 0x0104, 4) {
                    Ok(b) if b.len() >= 4 => format!("M-{:04X}, device type {:04X}, version {}", b[0], u16::from_be_bytes([b[1], b[2]]), b[3]),
                    _ => na(),
                };
                self.diag_update(job, 0.8, "", vec![("Programming mode".into(), prog), ("Application program".into(), app)], vec![]);
            }
            Ok(())
        })();
        let _ = self.tc_send(ia, vec![0x81]);
        result?;
        Ok(format!("Device info read from {target}"))
    }

    fn set_prog_mode(&mut self, ia: u16, on: bool) -> Result<String, String> {
        let target = fmt_ia(ia);
        let mut tc = Tc { dst: ia, seq: 0, last_rx: None };
        self.tc_send(ia, vec![0x80])?;
        let r = (|| -> Result<(), String> {
            let d = self.tc_request(&mut tc, &[0x03, 0x00], Some(Want::Class(0x340)))?.ok_or("no answer")?;
            let mask = d.tpdu.get(2..4).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0);
            if crate::diag::uses_properties(mask) && self.prop_write(&mut tc, 0, 54, 1, &[on as u8]).is_ok() {
                return Ok(());
            }
            // Memory 0x60: bit 0 programming mode, bit 7 parity.
            self.mem_write(&mut tc, 0x0060, &[if on { 0x81 } else { 0x00 }])
        })();
        let _ = self.tc_send(ia, vec![0x81]);
        r.map_err(|e| format!("{target}: {e}"))?;
        // Confirm with a programming mode search.
        let now = self.read_prog_mode()?;
        if now.contains(&ia) == on {
            Ok(format!("Programming mode of {target} switched {}", if on { "on" } else { "off" }))
        } else {
            Err(format!("{target} accepted the request but its programming mode is still {}", if on { "off" } else { "on" }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float16_roundtrip() {
        for v in [0.0, 21.5, -5.25, 1000.0, 670760.96, -671088.64] {
            let d = decode_f16(&encode_f16(v));
            assert!((d - v).abs() <= v.abs() * 0.01 + 0.02, "{v} -> {d}");
        }
        assert_eq!(encode_f16(21.5), [0x0C, 0x33]);
    }

    #[test]
    fn cemi_roundtrip() {
        let f = Frame::request(0x0801, true, group_tpdu(0x080, &[1], true));
        let b = f.to_cemi();
        assert_eq!(b, vec![0x11, 0x00, 0xBC, 0xE0, 0x00, 0x00, 0x08, 0x01, 0x01, 0x00, 0x81]);
        let g = Frame::from_cemi(&b).unwrap();
        assert_eq!(g.dst, 0x0801);
        assert_eq!(g.service(), 0x080);
        assert_eq!(g.value(), (vec![1], true));
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_ia("1.1.65"), Some(0x1141));
        assert_eq!(fmt_ia(0x1141), "1.1.65");
        assert_eq!(fmt_ga3(0x0801), "1/0/1");
    }

    /// A tiny fake tunnelling server on localhost: accepts a connection, acknowledges
    /// a telegram and sends one group telegram back.
    #[test]
    fn tunnel_against_fake_server() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = match server.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            server.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            // CONNECT_REQUEST
            let (n, client) = server.recv_from(&mut buf).unwrap();
            assert_eq!(parse_packet(&buf[..n]).unwrap().0, CONNECT_REQ);
            let mut body = vec![7, 0];
            body.extend(hpai(Ipv4Addr::UNSPECIFIED, 0));
            body.extend([0x04, 0x04, 0x11, 0xFE]); // assigned 1.1.254
            server.send_to(&packet(CONNECT_RES, &body), client).unwrap();
            // TUNNELLING_REQUEST from client -> ACK
            let (n, client) = server.recv_from(&mut buf).unwrap();
            let (svc, b) = parse_packet(&buf[..n]).unwrap();
            assert_eq!(svc, TUNNEL_REQ);
            let f = Frame::from_cemi(&b[4..]).unwrap();
            assert_eq!(f.dst, 0x0801);
            server.send_to(&packet(TUNNEL_ACK, &[4, 7, b[2], 0]), client).unwrap();
            // Indication to client
            let mut ind = Frame::request(0x0802, true, group_tpdu(0x080, &[0], true));
            ind.code = L_DATA_IND;
            ind.src = 0x1105;
            let mut body = vec![4, 7, 0, 0];
            body.extend(ind.to_cemi());
            server.send_to(&packet(TUNNEL_REQ, &body), client).unwrap();
            let (n, _) = server.recv_from(&mut buf).unwrap();
            assert_eq!(parse_packet(&buf[..n]).unwrap().0, TUNNEL_ACK);
        });
        let link = Link::open(Endpoint::Tunnel(addr));
        match link.rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Connected { ia, .. } => assert_eq!(fmt_ia(ia), "1.1.254"),
            _ => panic!("expected Connected"),
        }
        link.send(Frame::request(0x0801, true, group_tpdu(0x080, &[1], true)));
        let got = loop {
            match link.rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::Frame(f) => break f,
                Event::Failed(e) | Event::Closed(e) => panic!("{e}"),
                _ => {}
            }
        };
        assert_eq!(got.src, 0x1105);
        assert_eq!(got.dst, 0x0802);
        t.join().unwrap();
    }

    #[test]
    fn values() {
        assert_eq!(encode_value("5.001 percentage", "100").unwrap(), (vec![255], false));
        assert_eq!(encode_value("1.001 switch", "on").unwrap(), (vec![1], true));
        assert_eq!(decode_value("9.001", &[0x0C, 0x33], false), "21.5");
    }
}
