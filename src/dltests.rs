//! Download tests: a plan is built from a small application definition and executed
//! against a simulated System B device behind a fake tunnelling interface.

use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use crate::knx::*;
use crate::knxload::*;
use crate::model::*;

fn hdr(svc: u16, body: &[u8]) -> Vec<u8> {
    let mut p = vec![0x06, 0x10];
    p.extend(svc.to_be_bytes());
    p.extend(((6 + body.len()) as u16).to_be_bytes());
    p.extend_from_slice(body);
    p
}

/// Simulated device: load state machines, memory pointers and memory.
struct FakeDevice {
    ia: u16,
    state: HashMap<u8, u8>,
    ptr: HashMap<u8, u32>,
    next_free: u32,
    mem: HashMap<u32, u8>,
    seq_out: u8,
    log: Vec<String>,
    /// Whether reserving memory fills it (not every device does).
    prefill: bool,
}

impl FakeDevice {
    /// Handle one telegram to the device; returns telegrams (tpdu) the device sends back.
    fn handle(&mut self, tpdu: &[u8]) -> Vec<Vec<u8>> {
        let mut out = vec![];
        if tpdu.len() == 1 {
            return out; // connect, disconnect, ack
        }
        if tpdu[0] & 0xC0 != 0x40 {
            return out;
        }
        let seq = (tpdu[0] >> 2) & 0x0f;
        out.push(vec![0xC2 | (seq << 2)]);
        let apci = (((tpdu[0] & 3) as u16) << 8) | tpdu[1] as u16;
        let resp = |apci: u16, data: &[u8], s: &mut u8| {
            let mut t = vec![0x40 | (*s << 2) | ((apci >> 8) as u8 & 3), apci as u8];
            t.extend_from_slice(data);
            *s = (*s + 1) & 0x0f;
            t
        };
        match apci {
            0x300 => out.push(resp(0x340, &[0x07, 0xB0], &mut self.seq_out)),
            0x3D1 => out.push(resp(0x3D2, &[0x00], &mut self.seq_out)),
            0x3D5 => {
                let (obj, pid) = (tpdu[2], tpdu[3]);
                let data: Vec<u8> = match pid {
                    5 => vec![*self.state.get(&obj).unwrap_or(&0)],
                    7 => self.ptr.get(&obj).copied().unwrap_or(0).to_be_bytes().to_vec(),
                    27 => vec![0; 10],
                    _ => vec![],
                };
                let mut d = tpdu[2..6].to_vec();
                if data.is_empty() {
                    d[2] &= 0x0f; // count 0 = error
                }
                d.extend(data);
                out.push(resp(0x3D6, &d, &mut self.seq_out));
            }
            0x3D7 => {
                let (obj, pid) = (tpdu[2], tpdu[3]);
                let data = &tpdu[6..];
                let mut d = tpdu[2..6].to_vec();
                if pid == 5 {
                    let st = self.state.entry(obj).or_insert(0);
                    match data[0] {
                        1 => *st = 2,
                        2 => *st = 1,
                        4 => *st = 0,
                        3 if data[1] == 0x0B => {
                            let size = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
                            self.ptr.insert(obj, self.next_free);
                            if data[6] == 1 && self.prefill {
                                for i in 0..size {
                                    self.mem.insert(self.next_free + i, data[7]);
                                }
                            }
                            self.next_free += size + 16;
                        }
                        _ => {}
                    }
                    self.log.push(format!("lsm {obj} event {} -> {}", data[0], self.state[&obj]));
                    d.push(self.state[&obj]);
                } else if pid == 27 {
                    // Like real devices: the memory control table is write-only, the answer is negative.
                    self.log.push(format!("mcb {obj} = {:02X?}", data));
                    d[2] &= 0x0f;
                } else {
                    self.log.push(format!("prop {obj}/{pid} = {:02X?}", data));
                    d.extend_from_slice(data);
                }
                out.push(resp(0x3D6, &d, &mut self.seq_out));
            }
            a if a & 0x3C0 == 0x280 => {
                let n = (a & 0x3f) as usize;
                let addr = u16::from_be_bytes([tpdu[2], tpdu[3]]) as u32;
                for i in 0..n {
                    self.mem.insert(addr + i as u32, tpdu[4 + i]);
                }
            }
            a if a & 0x3C0 == 0x200 => {
                let n = (a & 0x3f) as usize;
                let addr = u16::from_be_bytes([tpdu[2], tpdu[3]]) as u32;
                let mut d = vec![tpdu[2], tpdu[3]];
                for i in 0..n {
                    d.push(*self.mem.get(&(addr + i as u32)).unwrap_or(&0));
                }
                out.push(resp(0x240 | n as u16, &d, &mut self.seq_out));
            }
            0x380 => self.log.push("restart".into()),
            _ => {}
        }
        out
    }
}

fn run_fake_interface(dev: FakeDevice) -> (std::net::SocketAddrV4, std::thread::JoinHandle<FakeDevice>) {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = match server.local_addr().unwrap() {
        SocketAddr::V4(a) => a,
        _ => unreachable!(),
    };
    let h = std::thread::spawn(move || {
        let mut dev = dev;
        let mut buf = [0u8; 600];
        server.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut client: Option<SocketAddr>;
        let mut seq = 0u8;
        loop {
            let Ok((n, from)) = server.recv_from(&mut buf) else { break };
            client = Some(from);
            let svc = u16::from_be_bytes([buf[2], buf[3]]);
            let body = buf[6..n].to_vec();
            match svc {
                0x0205 => {
                    let mut b = vec![1, 0, 0x08, 0x01, 0, 0, 0, 0, 0, 0];
                    b.extend([0x04, 0x04, 0x11, 0xFE]);
                    server.send_to(&hdr(0x0206, &b), from).unwrap();
                }
                0x0420 => {
                    server.send_to(&hdr(0x0421, &[4, 1, body[2], 0]), from).unwrap();
                    let Some(f) = Frame::from_cemi(&body[4..]) else { continue };
                    if f.dst != dev.ia || f.is_group() {
                        continue;
                    }
                    for t in dev.handle(&f.tpdu) {
                        let mut ind = Frame::request(0x11FE, false, t);
                        ind.code = L_DATA_IND;
                        ind.src = dev.ia;
                        let mut b = vec![4, 1, seq, 0];
                        seq = seq.wrapping_add(1);
                        b.extend(ind.to_cemi());
                        server.send_to(&hdr(0x0420, &b), client.unwrap()).unwrap();
                    }
                }
                0x0209 => {
                    server.send_to(&hdr(0x020A, &[1, 0]), from).unwrap();
                    break;
                }
                _ => {}
            }
        }
        dev
    });
    (addr, h)
}

fn sample() -> (LoadInfo, MaskInfo, Device, Project) {
    use base64::Engine;
    let mut li = LoadInfo { mask: "MV-07B0".into(), style: "MergedProcedure".into(), manufacturer: 0x48, app_number: 0x0403, app_version: 0x10, addr_max: 10, assoc_max: 10, ..Default::default() };
    // Bytes 12..24 are zero: they must be written too (devices need not pre-fill memory).
    let data: Vec<u8> = (0..40u8).map(|i| if (12..24).contains(&i) { 0 } else { i }).collect();
    li.segments.push(Segment { id: "RS-1".into(), lsm: 4, size: 40, data: base64::engine::general_purpose::STANDARD.encode(&data) });
    li.mem.insert("P-1".into(), MemLoc { seg: "RS-1".into(), bit: 8 * 3 + 4, bits: 4, enc: Enc::Uint, union: None, union_default: false });
    li.ref_param.insert("P-1_R-1".into(), "P-1".into());
    li.defaults.insert("P-1".into(), "3".into());
    li.procedures.insert(2, vec![
        Ld::RelSegment { lsm: 4, size: 40, mode: 1, fill: 0, applies: "full".into() },
        Ld::RelSegment { lsm: 4, size: 40, mode: 0, fill: 255, applies: "par".into() },
        Ld::WriteProp { obj: 4, pid: 27, start: 1, data: "00000028003200000000".into(), verify: false, applies: String::new() },
    ]);
    li.procedures.insert(4, vec![Ld::WriteRelMem { obj: 4, offset: 0, size: 40, verify: true, applies: "full,par".into() }]);
    li.objects.insert(1, (Flags { c: true, w: true, ..Default::default() }, "1 bit".into()));
    li.objects.insert(2, (Flags { c: true, r: true, t: true, ..Default::default() }, "1 bit".into()));
    let steps = |v: &str| -> Vec<Ld> {
        v.split(';')
            .filter(|s| !s.is_empty())
            .map(|s| {
                let p: Vec<&str> = s.split(':').collect();
                let n = |i: usize| p.get(i).and_then(|x| x.parse().ok()).unwrap_or(0);
                match p[0] {
                    "C" => Ld::Connect,
                    "M" => Ld::Merge { id: n(1) as u32 },
                    "E" => Ld::MapError { mapped_to_ok: p[1] == "1" },
                    "U" => Ld::Unload { lsm: n(1) },
                    "L" => Ld::Load { lsm: n(1) },
                    "R" => Ld::RelSegment { lsm: n(1), size: 2, mode: 0, fill: 0, applies: String::new() },
                    "W" => Ld::WriteRelMem { obj: n(1), offset: 0, size: 1048576, verify: true, applies: String::new() },
                    "P" => Ld::WriteProp { obj: 4, pid: 13, start: 1, data: "0000000000".into(), verify: true, applies: String::new() },
                    "D" => Ld::LoadCompleted { lsm: n(1) },
                    "X" => Ld::Restart,
                    _ => unreachable!(),
                }
            })
            .collect()
    };
    let mask = MaskInfo {
        id: "MV-07B0".into(),
        number: 0x07B0,
        model: "SystemB".into(),
        procedures: vec![MaskProc {
            ptype: "Load".into(),
            subtype: "ap1".into(),
            steps: steps("C;M:1;E:1;U:5;E:0;U:4;U:3;U:2;U:1;L:4;M:2;L:3;R:3;L:1;R:1;L:2;R:2;M:4;W:3;W:2;W:1;P;D:4;D:3;D:2;D:1;M:7;X"),
        }],
    };
    let mut proj = Project::new("t", Medium::IP, Medium::TP, GaStyle::ThreeLevel);
    let ga = proj.alloc_id();
    proj.addresses.push(GroupAddress { id: ga, main: 1, middle: 0, sub: 5, name: "x".into(), dpt: String::new(), central: false, pass_through: false, comment: String::new() });
    let mut dev = Device {
        id: 99,
        area: 1,
        line: 1,
        number: 20,
        name: "Test".into(),
        description: String::new(),
        comment: String::new(),
        manufacturer: String::new(),
        product: String::new(),
        order_no: String::new(),
        app_program: "x".into(),
        serial: String::new(),
        params: vec![],
        objects: vec![],
        adr_ok: true,
        prg_ok: false,
        par_ok: false,
        grp_ok: false,
        cfg_ok: false,
        knx_app: Some("A".into()),
        knx_values: BTreeMap::new(),
    };
    dev.knx_values.insert("P-1_R-1".into(), "9".into());
    dev.objects.push(GroupObject {
        number: 1,
        name: "o".into(),
        function: String::new(),
        description: String::new(),
        dpt: String::new(),
        flags: Flags { c: true, w: true, ..Default::default() },
        priority: "Low".into(),
        links: vec![ga],
        ref_id: String::new(),
        size: "1 bit".into(),
    });
    (li, mask, dev, proj)
}

#[test]
fn images_and_plan() {
    let (li, mask, dev, proj) = sample();
    let img = build_images(&li, &dev, &proj).unwrap();
    // Parameter 9 written into the low nibble of byte 3 (default data there is 3)
    assert_eq!(img.by_obj[&4][3], 0x09);
    assert_eq!(img.by_obj[&1], vec![0, 1, 0x08, 0x05]);
    assert_eq!(img.by_obj[&2], vec![0, 1, 0, 1, 0, 1]);
    // count = highest object number; entry n (1-based) = descriptor of object n
    assert_eq!(img.by_obj[&3], vec![0, 2, 0x17, 0, 0x4B, 0]);
    let plan = build_plan(&li, &mask, &dev, &proj, DlKind::Full, None).unwrap();
    assert!(matches!(plan.steps[1], Step::CheckMask(0x07B0)));
    assert!(plan.steps.iter().any(|s| matches!(s, Step::Unload { lsm: 5, ignore_errors: true })));
    assert!(plan.steps.iter().any(|s| matches!(s, Step::Unload { lsm: 4, ignore_errors: false })));
    assert!(plan.steps.iter().any(|s| matches!(s, Step::WriteProp { pid: 13, data, .. } if data == &vec![0, 0x48, 0x04, 0x03, 0x10])));
    assert!(plan.steps.iter().any(|s| matches!(s, Step::Alloc { lsm: 4, size: 40, mode: 1, fill: 0 })));
    assert_eq!(plan.total_bytes(), 40 + 4 + 6 + 6);
}

#[test]
fn download_to_simulated_device() {
    let (li, mask, dev, proj) = sample();
    let plan = build_plan(&li, &mask, &dev, &proj, DlKind::Full, None).unwrap();
    let img = build_images(&li, &dev, &proj).unwrap();
    let fake = FakeDevice { ia: 0x1114, state: HashMap::new(), ptr: HashMap::new(), next_free: 0x4000, mem: HashMap::new(), seq_out: 0, log: vec![], prefill: false };
    let (addr, h) = run_fake_interface(fake);
    let link = Link::open(Endpoint::Tunnel(addr));
    assert!(matches!(link.rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Connected { .. }));
    link.download(1, 0x1114, plan.steps.clone());
    let result = loop {
        match link.rx.recv_timeout(Duration::from_secs(20)).unwrap() {
            Event::Job { phase: crate::knx::Phase::Done, .. } => break Ok(()),
            Event::Job { phase: crate::knx::Phase::Failed, msg, .. } => break Err(msg),
            _ => {}
        }
    };
    link.close();
    let dev_after = h.join().unwrap();
    assert_eq!(result, Ok(()), "log: {:?}", dev_after.log);
    for lsm in 1..=4u8 {
        assert_eq!(dev_after.state[&lsm], 1, "lsm {lsm} loaded");
    }
    let base = dev_after.ptr[&4];
    let mem: Vec<u8> = (0..40).map(|i| dev_after.mem[&(base + i)]).collect();
    assert_eq!(mem, img.by_obj[&4]);
    let at = dev_after.ptr[&1];
    let table: Vec<u8> = (0..4).map(|i| dev_after.mem[&(at + i)]).collect();
    assert_eq!(table, vec![0, 1, 0x08, 0x05]);
    assert!(dev_after.log.iter().any(|l| l == "restart"));
    assert!(dev_after.log.iter().any(|l| l.starts_with("mcb 4")), "memory control table written");
    let _ = Ipv4Addr::LOCALHOST;
}

#[test]
fn wrong_device_type_is_refused() {
    let (li, mut mask, dev, proj) = sample();
    mask.number = 0x0705; // pretend the product is for another device type
    let plan = build_plan(&li, &MaskInfo { model: "SystemB".into(), ..mask }, &dev, &proj, DlKind::Full, None).unwrap();
    let fake = FakeDevice { ia: 0x1114, state: HashMap::new(), ptr: HashMap::new(), next_free: 0x4000, mem: HashMap::new(), seq_out: 0, log: vec![], prefill: false };
    let (addr, h) = run_fake_interface(fake);
    let link = Link::open(Endpoint::Tunnel(addr));
    let _ = link.rx.recv_timeout(Duration::from_secs(5));
    link.download(1, 0x1114, plan.steps);
    let msg = loop {
        if let Event::Job { phase: crate::knx::Phase::Failed, msg, .. } = link.rx.recv_timeout(Duration::from_secs(20)).unwrap() {
            break msg;
        }
    };
    link.close();
    let dev_after = h.join().unwrap();
    assert!(msg.contains("different type"), "{msg}");
    assert!(dev_after.log.is_empty(), "nothing may be written: {:?}", dev_after.log);
}

fn wait_done(link: &Link) -> (Result<String, String>, Vec<String>) {
    let mut report = vec![];
    loop {
        match link.rx.recv_timeout(Duration::from_secs(30)).unwrap() {
            Event::Report { lines, .. } => report = lines,
            Event::Job { phase: crate::knx::Phase::Done, msg, .. } => return (Ok(msg), report),
            Event::Job { phase: crate::knx::Phase::Failed, msg, .. } => return (Err(msg), report),
            _ => {}
        }
    }
}

/// A device that already holds a configuration (as after a download).
fn loaded_device(li: &LoadInfo, dev: &Device, proj: &Project, corrupt_app_byte: Option<usize>) -> FakeDevice {
    let img = build_images(li, dev, proj).unwrap();
    let mut f = FakeDevice { ia: 0x1114, state: HashMap::new(), ptr: HashMap::new(), next_free: 0x4000, mem: HashMap::new(), seq_out: 0, log: vec![], prefill: false };
    for obj in 1..=4u8 {
        let data = img.by_obj[&obj].clone();
        f.state.insert(obj, 1);
        f.ptr.insert(obj, f.next_free);
        for (i, b) in data.iter().enumerate() {
            f.mem.insert(f.next_free + i as u32, *b);
        }
        f.next_free += data.len() as u32 + 16;
    }
    if let Some(i) = corrupt_app_byte {
        let p = f.ptr[&4] + i as u32;
        let v = f.mem[&p];
        f.mem.insert(p, v ^ 0xFF);
    }
    f
}

#[test]
fn check_matching_device() {
    let (li, mask, dev, proj) = sample();
    let plan = build_check_plan(&li, &mask, &dev, &proj, None).unwrap();
    assert!(!plan.steps.iter().any(|s| matches!(s, Step::WriteMem { .. } | Step::WriteProp { .. } | Step::Alloc { .. } | Step::Unload { .. })), "a check never writes");
    let (addr, h) = run_fake_interface(loaded_device(&li, &dev, &proj, None));
    let link = Link::open(Endpoint::Tunnel(addr));
    let _ = link.rx.recv_timeout(Duration::from_secs(5));
    link.download(7, 0x1114, plan.steps);
    let (r, report) = wait_done(&link);
    link.close();
    let dev_after = h.join().unwrap();
    assert!(dev_after.log.iter().all(|l| !l.starts_with("lsm")), "nothing written: {:?}", dev_after.log);
    let text = report.join("\n");
    assert!(text.contains("1 group address(es): 1/0/5"), "{text}");
    assert!(text.contains("1 → object 1"), "{text}");
    assert_eq!(text.matches("identical to the project").count(), 4, "{text}");
    // App id property is not stored by the fake device, so that line reports a difference.
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn check_finds_parameter_difference() {
    let (li, mask, dev, proj) = sample();
    let plan = build_check_plan(&li, &mask, &dev, &proj, None).unwrap();
    let (addr, h) = run_fake_interface(loaded_device(&li, &dev, &proj, Some(3)));
    let link = Link::open(Endpoint::Tunnel(addr));
    let _ = link.rx.recv_timeout(Duration::from_secs(5));
    link.download(8, 0x1114, plan.steps);
    let (r, report) = wait_done(&link);
    link.close();
    let _ = h.join();
    let text = report.join("\n");
    assert!(text.contains("1 of 40 bytes differ"), "{text}");
    assert!(text.contains("offset 0003–0003"), "{text}");
    assert!(r.unwrap().contains("difference"));
}
/// The descriptor of group object n must be the n-th word after the count (object numbers start
/// at 1), so the device finds e.g. object 14's flags at word 14. A shift here silently disables objects.
#[test]
fn group_object_table_positions() {
    let (mut li, _mask, mut dev, proj) = sample();
    li.objects.clear();
    for n in 1..=20u16 {
        li.objects.insert(n, (Flags { c: true, w: true, ..Default::default() }, "2 bytes".into()));
    }
    dev.objects[0].number = 14;
    dev.objects[0].flags = Flags { c: true, w: true, t: true, u: true, r: false };
    dev.objects[0].size = "1 bit".into();
    let img = build_images(&li, &dev, &proj).unwrap();
    let t = &img.by_obj[&3];
    assert_eq!(u16::from_be_bytes([t[0], t[1]]), 20, "count = highest object number");
    assert_eq!(t.len(), 2 + 20 * 2);
    let word = |n: usize| [t[2 * n], t[2 * n + 1]];
    assert_eq!(word(14), [0xD7, 0x00], "object 14: update+transmit+write+communication, 1 bit");
    assert_eq!(word(13), [0x13, 0x08], "object 13 (not used): communication off, 2 bytes");
    // association: object 14 with the first group address
    assert_eq!(img.by_obj[&2], vec![0, 1, 0, 1, 0, 14]);
}
/// Diagnostics over a (fake) KNXnet/IP tunnel: device info of a System B device,
/// an address check of an unused address and a programming mode search.
#[test]
fn diagnostics_on_a_tunnel() {
    use crate::diag::{DiagReq, DiagUpdate};
    let mut fake = FakeDevice { ia: 0x1114, state: HashMap::new(), ptr: HashMap::new(), next_free: 0x4000, mem: HashMap::new(), seq_out: 0, log: vec![], prefill: false };
    fake.state.insert(4, 1);
    let (addr, h) = run_fake_interface(fake);
    let link = Link::open(Endpoint::Tunnel(addr));
    assert!(matches!(link.rx.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Connected { .. }));
    let run = |link: &Link, job: u64, req: DiagReq| -> (Result<String, String>, Vec<DiagUpdate>) {
        link.diag(job, req);
        let mut ups = vec![];
        loop {
            match link.rx.recv_timeout(Duration::from_secs(60)).unwrap() {
                Event::Diag(u) if u.job == job => {
                    if let Some(r) = u.done.clone() {
                        return (r, ups);
                    }
                    ups.push(u);
                }
                _ => {}
            }
        }
    };
    let (r, ups) = run(&link, 1, DiagReq::DeviceInfo(0x1114));
    assert_eq!(r.unwrap(), "Device info read from 1.1.20");
    let info: Vec<(String, String)> = ups.into_iter().flat_map(|u| u.info).collect();
    let get = |k: &str| info.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    assert!(get("Mask version").contains("System B"), "{info:?}");
    assert_eq!(get("Application"), "Loaded");
    assert_eq!(get("Group address table"), "Unloaded");
    assert_eq!(get("Serial number"), "not available");

    let (r, ups) = run(&link, 2, DiagReq::Check(0x1114));
    assert_eq!(r.unwrap(), "A device answers at 1.1.20");
    assert_eq!(ups.iter().flat_map(|u| u.hits.iter()).next().unwrap().mask, Some(0x07B0));
    let (r, _) = run(&link, 3, DiagReq::Check(0x1115));
    assert!(r.unwrap().contains("free"));
    let (r, _) = run(&link, 4, DiagReq::ProgScan);
    assert_eq!(r.unwrap(), "No device is in programming mode");
    link.close();
    let _ = h.join();
}
