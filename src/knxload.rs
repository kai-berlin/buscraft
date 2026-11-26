//! Load (download) data of application programs and the download plan builder.
//!
//! The product files describe how a device is loaded: the device type's generic load
//! procedure (in the master data, per mask version), the application's own steps (merged
//! into it), the memory segment with default data, where each parameter lives, and the
//! group address / association / group object tables. From that and the device's settings
//! a list of steps is built, which the bus thread executes (see `knx::Conn::download`).

use std::collections::{BTreeMap, HashMap};

use roxmltree::{Document, Node as XNode};
use serde::{Deserialize, Serialize};

use crate::model::*;

// ------------------------------------------------------------------ stored definitions

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Ld {
    Connect,
    Disconnect,
    Restart,
    Unload { lsm: u8 },
    Load { lsm: u8 },
    LoadCompleted { lsm: u8 },
    RelSegment { lsm: u8, size: u32, mode: u8, fill: u8, applies: String },
    WriteProp { obj: u8, pid: u8, start: u16, data: String, verify: bool, applies: String },
    CompareProp { obj: u8, pid: u8, start: u16, data: String },
    LoadImageProp { obj: u8, pid: u8, count: u8 },
    WriteRelMem { obj: u8, offset: u32, size: u32, verify: bool, applies: String },
    Merge { id: u32 },
    MapError { mapped_to_ok: bool },
    Other(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskProc {
    pub ptype: String,
    pub subtype: String,
    pub steps: Vec<Ld>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskInfo {
    pub id: String,
    pub number: u16,
    pub model: String,
    pub procedures: Vec<MaskProc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Enc {
    Uint,
    Int,
    F16,
    F32,
    F64,
    Text,
    Binary,
    Ip,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemLoc {
    pub seg: String,
    pub bit: u32,
    pub bits: u32,
    pub enc: Enc,
    /// Parameters sharing memory (a union) have the same id; only one of them is written.
    #[serde(default)]
    pub union: Option<u64>,
    #[serde(default)]
    pub union_default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PropLoc {
    pub obj: u8,
    pub pid: u8,
    pub bit: u32,
    pub bits: u32,
    pub enc: Enc,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub id: String,
    pub lsm: u8,
    pub size: u32,
    /// Default content (base64); empty when the product gives none.
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LoadInfo {
    pub mask: String,
    pub style: String,
    pub manufacturer: u16,
    pub app_number: u16,
    pub app_version: u8,
    pub segments: Vec<Segment>,
    /// Base parameter id → memory location
    pub mem: HashMap<String, MemLoc>,
    /// Base parameter id → property location
    pub prop: HashMap<String, PropLoc>,
    /// ParameterRef id → base parameter id
    pub ref_param: HashMap<String, String>,
    /// Base parameter id → default value
    pub defaults: HashMap<String, String>,
    /// ParameterRef id → default value given by the reference (overrides the parameter's)
    #[serde(default)]
    pub ref_value: HashMap<String, String>,
    pub procedures: BTreeMap<u32, Vec<Ld>>,
    pub addr_max: u32,
    pub assoc_max: u32,
    /// Group object number → (flags, size text) for all objects of the application.
    pub objects: BTreeMap<u16, (Flags, String)>,
}

// ------------------------------------------------------------------ parsing

fn a<'a>(n: XNode<'a, 'a>, k: &str) -> &'a str {
    n.attribute(k).unwrap_or("")
}
fn au(n: XNode, k: &str) -> u32 {
    a(n, k).parse().unwrap_or(0)
}

pub fn parse_ld(n: XNode) -> Option<Ld> {
    let applies = a(n, "AppliesTo").to_string();
    Some(match n.tag_name().name() {
        "LdCtrlConnect" => Ld::Connect,
        "LdCtrlDisconnect" => Ld::Disconnect,
        "LdCtrlRestart" => Ld::Restart,
        "LdCtrlUnload" => Ld::Unload { lsm: au(n, "LsmIdx") as u8 },
        "LdCtrlLoad" => Ld::Load { lsm: au(n, "LsmIdx") as u8 },
        "LdCtrlLoadCompleted" => Ld::LoadCompleted { lsm: au(n, "LsmIdx") as u8 },
        "LdCtrlRelSegment" => Ld::RelSegment { lsm: au(n, "LsmIdx") as u8, size: au(n, "Size"), mode: au(n, "Mode") as u8, fill: au(n, "Fill") as u8, applies },
        "LdCtrlWriteProp" => Ld::WriteProp {
            obj: au(n, "ObjIdx") as u8,
            pid: au(n, "PropId") as u8,
            start: n.attribute("StartElement").and_then(|s| s.parse().ok()).unwrap_or(1),
            data: a(n, "InlineData").to_string(),
            verify: a(n, "Verify") == "true",
            applies,
        },
        "LdCtrlCompareProp" => Ld::CompareProp {
            obj: au(n, "ObjIdx") as u8,
            pid: au(n, "PropId") as u8,
            start: n.attribute("StartElement").and_then(|s| s.parse().ok()).unwrap_or(1),
            data: a(n, "InlineData").to_string(),
        },
        "LdCtrlLoadImageProp" => Ld::LoadImageProp { obj: au(n, "ObjIdx") as u8, pid: au(n, "PropId") as u8, count: n.attribute("Count").and_then(|s| s.parse().ok()).unwrap_or(1) },
        "LdCtrlWriteRelMem" => Ld::WriteRelMem { obj: au(n, "ObjIdx") as u8, offset: au(n, "Offset"), size: au(n, "Size"), verify: a(n, "Verify") == "true", applies },
        "LdCtrlMerge" => Ld::Merge { id: au(n, "MergeId") },
        "LdCtrlMapError" => Ld::MapError { mapped_to_ok: a(n, "MappedError") == "0" },
        other if other.starts_with("LdCtrl") => Ld::Other(other.to_string()),
        _ => return None,
    })
}

/// Device type information (mask versions) from knx_master.xml.
pub fn parse_masks(doc: &Document) -> Vec<MaskInfo> {
    let mut out = vec![];
    for mv in doc.descendants().filter(|n| n.has_tag_name("MaskVersion")) {
        let id = a(mv, "Id").to_string();
        let number: u16 = a(mv, "MaskVersion").parse().unwrap_or(0);
        // The current configuration data (not the legacy one)
        let hawk = mv.children().find(|c| c.has_tag_name("HawkConfigurationData") && c.attribute("LegacyVersion").is_none()).or_else(|| mv.children().find(|c| c.has_tag_name("HawkConfigurationData")));
        let mut procedures = vec![];
        if let Some(h) = hawk {
            if let Some(ps) = h.children().find(|c| c.has_tag_name("Procedures")) {
                for p in ps.children().filter(|c| c.has_tag_name("Procedure")) {
                    procedures.push(MaskProc {
                        ptype: a(p, "ProcedureType").to_string(),
                        subtype: a(p, "ProcedureSubType").to_string(),
                        steps: p.children().filter(|c| c.is_element()).filter_map(parse_ld).collect(),
                    });
                }
            }
        }
        out.push(MaskInfo { id, number, model: a(mv, "ManagementModel").to_string(), procedures });
    }
    out
}

fn type_enc(pt: XNode) -> Option<(u32, Enc)> {
    let def = pt.children().find(|c| c.is_element())?;
    let size = def.attribute("SizeInBit").and_then(|s| s.parse().ok());
    Some(match def.tag_name().name() {
        "TypeNumber" => {
            let t = a(def, "Type");
            let enc = if t.contains("signedInt") && !t.contains("unsigned") { Enc::Int } else if t.contains("float") { Enc::F16 } else { Enc::Uint };
            (size?, enc)
        }
        "TypeRestriction" => (size?, if a(def, "Base") == "BinaryValue" { Enc::Binary } else { Enc::Uint }),
        "TypeTime" | "TypeDate" => (size.unwrap_or(16), Enc::Uint),
        "TypeFloat" => match a(def, "Encoding") {
            "IEEE-754 Single" => (32, Enc::F32),
            "IEEE-754 Double" => (64, Enc::F64),
            _ => (16, Enc::F16),
        },
        "TypeText" => (size?, Enc::Text),
        "TypeIPAddress" => (32, Enc::Ip),
        _ => return None,
    })
}

/// Read the load data of an application program.
pub fn parse_load(doc: &Document, app: XNode) -> LoadInfo {
    use base64::Engine;
    let mut li = LoadInfo {
        mask: a(app, "MaskVersion").to_string(),
        style: a(app, "LoadProcedureStyle").to_string(),
        app_number: a(app, "ApplicationNumber").parse().unwrap_or(0),
        app_version: a(app, "ApplicationVersion").parse().unwrap_or(0),
        manufacturer: a(app, "Id").strip_prefix("M-").and_then(|s| u16::from_str_radix(&s[..s.find('_').unwrap_or(s.len())], 16).ok()).unwrap_or(0),
        ..Default::default()
    };
    for s in doc.descendants().filter(|n| n.has_tag_name("RelativeSegment")) {
        let data = s.children().find(|c| c.has_tag_name("Data")).and_then(|d| d.text()).map(|t| t.trim().to_string()).unwrap_or_default();
        // Keep only valid base64
        let data = if base64::engine::general_purpose::STANDARD.decode(&data).is_ok() { data } else { String::new() };
        li.segments.push(Segment { id: a(s, "Id").to_string(), lsm: au(s, "LoadStateMachine") as u8, size: au(s, "Size"), data });
    }
    let types: HashMap<&str, (u32, Enc)> = doc.descendants().filter(|n| n.has_tag_name("ParameterType")).filter_map(|t| Some((t.attribute("Id")?, type_enc(t)?))).collect();
    for p in doc.descendants().filter(|n| n.has_tag_name("Parameter")) {
        let Some(pid) = p.attribute("Id") else { continue };
        li.defaults.insert(pid.to_string(), a(p, "Value").to_string());
        let Some(&(bits, enc)) = types.get(a(p, "ParameterType")) else { continue };
        let own_bit = au(p, "Offset") * 8 + au(p, "BitOffset");
        let (mem, prop) = match p.parent().filter(|x| x.has_tag_name("Union")) {
            Some(u) => (u.children().find(|c| c.has_tag_name("Memory")), u.children().find(|c| c.has_tag_name("Property"))),
            None => (p.children().find(|c| c.has_tag_name("Memory")), p.children().find(|c| c.has_tag_name("Property"))),
        };
        let in_union = p.parent().map(|x| x.has_tag_name("Union")).unwrap_or(false);
        if let Some(m) = mem {
            if m.attribute("BaseOffset").is_some() {
                continue; // module parameters are not supported
            }
            let base = au(m, "Offset") * 8 + au(m, "BitOffset");
            let bit = if in_union { base + own_bit } else { base };
            let union = if in_union { p.parent().map(|u| u.id().get() as u64) } else { None };
            li.mem.insert(pid.to_string(), MemLoc { seg: a(m, "CodeSegment").to_string(), bit, bits, enc, union, union_default: p.attribute("DefaultUnionParameter") == Some("true") });
        } else if let Some(pr) = prop {
            let base = au(pr, "Offset") * 8 + au(pr, "BitOffset");
            let bit = if in_union { base + own_bit } else { base };
            li.prop.insert(pid.to_string(), PropLoc { obj: au(pr, "ObjectIndex") as u8, pid: au(pr, "PropertyId") as u8, bit, bits, enc });
        }
    }
    for r in doc.descendants().filter(|n| n.has_tag_name("ParameterRef")) {
        if let (Some(id), Some(rf)) = (r.attribute("Id"), r.attribute("RefId")) {
            li.ref_param.insert(id.to_string(), rf.to_string());
            if let Some(v) = r.attribute("Value") {
                li.ref_value.insert(id.to_string(), v.to_string());
            }
        }
    }
    for lp in doc.descendants().filter(|n| n.has_tag_name("LoadProcedure")) {
        let id: u32 = lp.attribute("MergeId").and_then(|s| s.parse().ok()).unwrap_or(0);
        let steps: Vec<Ld> = lp.children().filter(|c| c.is_element()).filter_map(parse_ld).collect();
        li.procedures.entry(id).or_default().extend(steps);
    }
    if let Some(t) = doc.descendants().find(|n| n.has_tag_name("AddressTable")) {
        li.addr_max = au(t, "MaxEntries");
    }
    if let Some(t) = doc.descendants().find(|n| n.has_tag_name("AssociationTable")) {
        li.assoc_max = au(t, "MaxEntries");
    }
    // Objects inside module definitions have numbers relative to the module and are not
    // real object numbers (modules are not supported yet).
    for c in doc.descendants().filter(|n| n.has_tag_name("ComObject") && !n.ancestors().any(|a| a.has_tag_name("ModuleDefs"))) {
        let f = |k: &str| c.attribute(k) == Some("Enabled");
        let n: u16 = a(c, "Number").parse().unwrap_or(0);
        li.objects.entry(n).or_insert((
            Flags { c: f("CommunicationFlag"), r: f("ReadFlag"), w: f("WriteFlag"), t: f("TransmitFlag"), u: f("UpdateFlag") },
            a(c, "ObjectSize").to_lowercase(),
        ));
    }
    li
}

// ------------------------------------------------------------------ images

fn put_bits(buf: &mut [u8], bitpos: u32, bits: u32, bytes_be: &[u8]) -> Result<(), String> {
    // `bytes_be` holds the value right-aligned in big-endian order.
    let total = bytes_be.len() as u32 * 8;
    if bits > total {
        return Err("value too short".into());
    }
    for i in 0..bits {
        let src = total - bits + i;
        let bit = (bytes_be[(src / 8) as usize] >> (7 - src % 8)) & 1;
        let pos = bitpos + i;
        let byte = (pos / 8) as usize;
        if byte >= buf.len() {
            return Err(format!("parameter outside the memory segment (byte {byte})"));
        }
        let m = 1u8 << (7 - pos % 8);
        if bit == 1 {
            buf[byte] |= m;
        } else {
            buf[byte] &= !m;
        }
    }
    Ok(())
}

fn encode(value: &str, bits: u32, enc: Enc) -> Result<Vec<u8>, String> {
    let v = value.trim();
    let num = || v.replace(',', ".").parse::<f64>().map_err(|_| format!("\"{v}\" is not a number"));
    Ok(match enc {
        Enc::Uint | Enc::Int => {
            let x = if let Some(h) = v.strip_prefix("0x") { i64::from_str_radix(h, 16).map_err(|e| e.to_string())? } else { num()? as i64 };
            (x as u64).to_be_bytes().to_vec()
        }
        Enc::F16 => crate::knx::encode_f16(num()?).to_vec(),
        Enc::F32 => (num()? as f32).to_be_bytes().to_vec(),
        Enc::F64 => num()?.to_be_bytes().to_vec(),
        Enc::Text => {
            let mut b: Vec<u8> = v.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
            b.resize((bits / 8) as usize, 0);
            b
        }
        Enc::Binary => {
            let h: String = v.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            let mut b: Vec<u8> = (0..h.len() / 2).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap_or(0)).collect();
            let n = (bits as usize).div_ceil(8);
            while b.len() < n {
                b.insert(0, 0);
            }
            b
        }
        Enc::Ip => {
            let p: Vec<u8> = v.split('.').map(|x| x.parse().unwrap_or(0)).collect();
            if p.len() != 4 {
                return Err(format!("\"{v}\" is not an IP address"));
            }
            p
        }
    })
}

/// Size code used in group object tables.
fn size_code(s: &str) -> u8 {
    let s = s.to_lowercase();
    let n: u32 = s.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(1);
    if s.contains("bit") {
        (n.clamp(1, 7) - 1) as u8
    } else {
        match n {
            1 => 7,
            2 => 8,
            3 => 9,
            4 => 10,
            6 => 11,
            8 => 12,
            10 => 13,
            14 => 14,
            5 => 15,
            7 => 16,
            9 => 17,
            11 => 18,
            12 => 19,
            13 => 20,
            _ => 14,
        }
    }
}

fn descriptor(f: Flags, size: &str) -> [u8; 2] {
    let mut c = 0u8;
    if f.u {
        c |= 0x80;
    }
    if f.t {
        c |= 0x40;
    }
    if f.w {
        c |= 0x10;
    }
    if f.r {
        c |= 0x08;
    }
    if f.c {
        c |= 0x04;
    }
    c |= 0x03; // low priority
    [c, size_code(size)]
}

pub struct Images {
    /// Interface object index → content
    pub by_obj: HashMap<u8, Vec<u8>>,
    /// Parameters stored in properties: (obj, pid, bit, bits, value bytes)
    pub props: Vec<(u8, u8, u32, u32, Vec<u8>)>,
    pub warnings: Vec<String>,
}

/// What is active in the device with its current settings (from evaluating the dynamic part).
#[derive(Default, Clone)]
pub struct Active {
    /// Parameter references reached on active branches (also hidden ones)
    pub reached: std::collections::HashSet<String>,
    /// Active (visible) group objects by number: flags and size as given by the object reference
    pub objects: HashMap<u16, (Flags, String)>,
}

/// Build the memory images: application segment(s) with parameter values, and the tables.
#[allow(dead_code)]
pub fn build_images(li: &LoadInfo, dev: &Device, proj: &Project) -> Result<Images, String> {
    build_images_with(li, dev, proj, None)
}

/// Like `build_images`, matching the original tool: every parameter is written (the device's
/// value, else the reference's default, else the parameter's default) – the product's memory
/// template does not contain the defaults. Of parameters sharing memory (a union) only the
/// active one is written. Only active group objects get a descriptor.
pub fn build_images_with(li: &LoadInfo, dev: &Device, proj: &Project, active: Option<&Active>) -> Result<Images, String> {
    use base64::Engine;
    let mut by_obj = HashMap::new();
    let mut warnings = vec![];
    // Parameter values by base parameter id
    let mut changed: HashMap<String, String> = HashMap::new();
    for (r, v) in &dev.knx_values {
        if let Some(p) = li.ref_param.get(r) {
            changed.insert(p.clone(), v.clone());
        }
    }
    // References per parameter (to find reached ones and reference defaults)
    let mut refs_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for (r, p) in &li.ref_param {
        refs_of.entry(p.as_str()).or_default().push(r.as_str());
    }
    let is_reached = |pid: &str| -> bool { active.map(|a| refs_of.get(pid).map(|rs| rs.iter().any(|r| a.reached.contains(*r))).unwrap_or(false)).unwrap_or(false) };
    let value_for = |pid: &str| -> String {
        if let Some(v) = changed.get(pid) {
            return v.clone();
        }
        let rs = refs_of.get(pid).cloned().unwrap_or_default();
        let reached_ref = rs.iter().find(|r| active.map(|a| a.reached.contains(**r)).unwrap_or(false));
        reached_ref
            .and_then(|r| li.ref_value.get(*r))
            .or_else(|| rs.iter().find_map(|r| li.ref_value.get(*r)))
            .or_else(|| li.defaults.get(pid))
            .cloned()
            .unwrap_or_default()
    };
    for seg in &li.segments {
        let data = base64::engine::general_purpose::STANDARD.decode(&seg.data).unwrap_or_default();
        let has_data = data.len() == seg.size as usize;
        let mut img = if has_data { data } else { vec![0u8; seg.size as usize] };
        if active.is_some() {
            // Choose one member per union: a reached one, else one with a set value, else the default member.
            let mut unions: BTreeMap<u64, Vec<&str>> = BTreeMap::new();
            let mut plain: Vec<&str> = vec![];
            for (pid, m) in li.mem.iter().filter(|(_, m)| m.seg == seg.id) {
                match m.union {
                    Some(u) => unions.entry(u).or_default().push(pid.as_str()),
                    None => plain.push(pid.as_str()),
                }
            }
            plain.sort();
            let mut chosen: Vec<&str> = vec![];
            for (_, mut members) in unions {
                members.sort();
                let pick = members
                    .iter()
                    .find(|p| is_reached(p))
                    .or_else(|| members.iter().find(|p| changed.contains_key(**p)))
                    .or_else(|| members.iter().find(|p| li.mem[**p].union_default))
                    .copied();
                if let Some(p) = pick {
                    chosen.push(p);
                }
            }
            // Only parameters on active branches are written (with their value or default);
            // everything else keeps the product's memory template.
            for pid in plain.into_iter().chain(chosen) {
                if !is_reached(pid) && !changed.contains_key(pid) {
                    continue;
                }
                let m = &li.mem[pid];
                let value = value_for(pid);
                if value.is_empty() {
                    continue;
                }
                let bytes = encode(&value, m.bits, m.enc).map_err(|e| format!("Parameter {pid}: {e}"))?;
                put_bits(&mut img, m.bit, m.bits, &bytes).map_err(|e| format!("Parameter {pid}: {e}"))?;
            }
        } else {
            for (pid, m) in li.mem.iter().filter(|(_, m)| m.seg == seg.id) {
                let value = match changed.get(pid) {
                    Some(v) => v.clone(),
                    None if !has_data => li.defaults.get(pid).cloned().unwrap_or_default(),
                    None => continue,
                };
                let bytes = encode(&value, m.bits, m.enc).map_err(|e| format!("Parameter {pid}: {e}"))?;
                put_bits(&mut img, m.bit, m.bits, &bytes).map_err(|e| format!("Parameter {pid}: {e}"))?;
            }
        }
        by_obj.insert(seg.lsm, img);
    }
    let mut props = vec![];
    for (pid, p) in &li.prop {
        let value = changed.get(pid).cloned().or_else(|| li.defaults.get(pid).cloned()).unwrap_or_default();
        match encode(&value, p.bits, p.enc) {
            Ok(b) => props.push((p.obj, p.pid, p.bit, p.bits, b)),
            Err(e) => warnings.push(format!("Property parameter {pid} skipped: {e}")),
        }
    }

    // Group address table: count, then sorted addresses
    let mut gas: Vec<u16> = dev.objects.iter().flat_map(|o| o.links.iter()).filter_map(|g| proj.ga(*g)).map(|g| g.raw()).collect();
    gas.sort();
    gas.dedup();
    if li.addr_max > 0 && gas.len() as u32 > li.addr_max {
        return Err(format!("The device supports {} group addresses, but {} are linked", li.addr_max, gas.len()));
    }
    let mut at = (gas.len() as u16).to_be_bytes().to_vec();
    for g in &gas {
        at.extend(g.to_be_bytes());
    }
    by_obj.insert(1, at);

    // Association table: count, then (address index, object number)
    let mut assoc: Vec<(u16, u16)> = vec![];
    for o in &dev.objects {
        for l in &o.links {
            if let Some(i) = proj.ga(*l).and_then(|g| gas.iter().position(|x| *x == g.raw())) {
                assoc.push((i as u16 + 1, o.number));
            }
        }
    }
    assoc.sort();
    assoc.dedup();
    if li.assoc_max > 0 && assoc.len() as u32 > li.assoc_max {
        return Err(format!("The device supports {} associations, but {} are needed", li.assoc_max, assoc.len()));
    }
    let mut asc = (assoc.len() as u16).to_be_bytes().to_vec();
    for (t, o) in &assoc {
        asc.extend(t.to_be_bytes());
        asc.extend(o.to_be_bytes());
    }
    by_obj.insert(2, asc);

    // Group object table (System B): the first word is the number of objects, then one
    // descriptor per object. Objects are addressed by their number (ASAP) starting at 1, so
    // the descriptor of object n is the n-th word after the count – object numbers are 1-based.
    let max = li.objects.keys().max().copied().into_iter().chain(dev.objects.iter().map(|o| o.number)).max().unwrap_or(0);
    if li.objects.contains_key(&0) || dev.objects.iter().any(|o| o.number == 0) {
        warnings.push("The application has a group object with number 0, which this device type cannot address; it is left out".into());
    }
    let mut got = vec![];
    if max > 0 {
        got.extend(max.to_be_bytes());
        for n in 1..=max {
            if let Some(a) = active {
                // As the original tool: only active objects get a descriptor, and the
                // communication flag only when the object has group addresses.
                let d = if !a.objects.contains_key(&n) {
                    [0, 0]
                } else {
                    match dev.objects.iter().find(|o| o.number == n) {
                        Some(o) => {
                            let size = if o.size.is_empty() { li.objects.get(&n).map(|x| x.1.clone()).unwrap_or_default() } else { o.size.clone() };
                            descriptor(Flags { c: o.flags.c && !o.links.is_empty(), ..o.flags }, &size)
                        }
                        // Active but without links: the reference's flags, communication off
                        None => {
                            let (f, s) = &a.objects[&n];
                            descriptor(Flags { c: false, ..*f }, s)
                        }
                    }
                };
                got.extend(d);
                continue;
            }
            let d = match dev.objects.iter().find(|o| o.number == n) {
                Some(o) => descriptor(o.flags, &if o.size.is_empty() { li.objects.get(&n).map(|x| x.1.clone()).unwrap_or_default() } else { o.size.clone() }),
                None => match li.objects.get(&n) {
                    Some((f, s)) => descriptor(Flags { c: false, ..*f }, s),
                    None => [0, 0],
                },
            };
            got.extend(d);
        }
    } else {
        got.extend(0u16.to_be_bytes());
    }
    by_obj.insert(3, got);
    Ok(Images { by_obj, props, warnings })
}

// ------------------------------------------------------------------ plan

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Connect,
    CheckMask(u16),
    Authorize,
    Unload { lsm: u8, ignore_errors: bool },
    Load { lsm: u8 },
    LoadCompleted { lsm: u8 },
    Alloc { lsm: u8, size: u32, mode: u8, fill: u8 },
    WriteProp { obj: u8, pid: u8, start: u16, data: Vec<u8>, verify: bool },
    PatchProp { obj: u8, pid: u8, bit: u32, bits: u32, value: Vec<u8> },
    CompareProp { obj: u8, pid: u8, start: u16, data: Vec<u8> },
    ImageProp { obj: u8, pid: u8, count: u8 },
    WriteMem { obj: u8, offset: u32, data: Vec<u8>, verify: bool, skip_fill: Option<u8> },
    /// Read-only checks (device check):
    ReadState { lsm: u8 },
    CheckProp { obj: u8, pid: u8, data: Vec<u8> },
    CheckMem { obj: u8, data: Vec<u8> },
    Restart,
    Disconnect,
}

fn obj_name(o: u8) -> &'static str {
    match o {
        1 => "group address table",
        2 => "association table",
        3 => "group object table",
        4 => "application",
        5 => "PEI program",
        _ => "object",
    }
}

impl Step {
    pub fn describe(&self) -> String {
        match self {
            Step::Connect => "Connect to the device".into(),
            Step::CheckMask(m) => format!("Check device type (mask {m:04X})"),
            Step::Authorize => "Authorize".into(),
            Step::Unload { lsm, .. } => format!("Unload {}", obj_name(*lsm)),
            Step::Load { lsm } => format!("Start loading {}", obj_name(*lsm)),
            Step::LoadCompleted { lsm } => format!("Finish loading {}", obj_name(*lsm)),
            Step::Alloc { lsm, size, .. } => format!("Reserve {size} bytes for the {}", obj_name(*lsm)),
            Step::WriteProp { obj, pid, start, data, .. } => format!("Write property {pid} of object {obj} (element {start}, {} bytes)", data.len()),
            Step::PatchProp { obj, pid, .. } => format!("Write parameter in property {pid} of object {obj}"),
            Step::CompareProp { obj, pid, .. } => format!("Check property {pid} of object {obj}"),
            Step::ImageProp { obj, pid, .. } => format!("Read property {pid} of object {obj}"),
            Step::WriteMem { obj, data, .. } => format!("Write {} ({} bytes)", obj_name(*obj), data.len()),
            Step::ReadState { lsm } => format!("Read load state of the {}", obj_name(*lsm)),
            Step::CheckProp { obj, pid, .. } => format!("Compare property {pid} of object {obj}"),
            Step::CheckMem { obj, data } => format!("Read back {} ({} bytes)", obj_name(*obj), data.len()),
            Step::Restart => "Restart the device".into(),
            Step::Disconnect => "Disconnect".into(),
        }
    }
    /// Bytes this step writes (for progress).
    pub fn weight(&self) -> u64 {
        match self {
            Step::WriteMem { data, .. } | Step::CheckMem { data, .. } => data.len() as u64 + 20,
            _ => 20,
        }
    }
}

pub struct Plan {
    pub kind: String,
    pub steps: Vec<Step>,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn total_bytes(&self) -> usize {
        self.steps.iter().map(|s| if let Step::WriteMem { data, .. } = s { data.len() } else { 0 }).sum()
    }
}

fn hex(s: &str) -> Vec<u8> {
    let h: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (0..h.len() / 2).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap_or(0)).collect()
}

/// What kind of download: "full" (application) or "par" (parameters and tables).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DlKind {
    Full,
    Partial,
    Unload,
}

pub fn supported(li: &LoadInfo, mask: Option<&MaskInfo>) -> Result<(), String> {
    let Some(m) = mask else { return Err(format!("The device type {} is not known – import the product file again", li.mask)) };
    if m.model != "SystemB" || li.style != "MergedProcedure" {
        return Err(format!(
            "Downloading to this device type ({}, {}) is not supported yet. Supported are System B devices (mask 07B0), which most current devices use.",
            li.mask.trim_start_matches("MV-"),
            if m.model.is_empty() { "unknown" } else { &m.model }
        ));
    }
    if li.segments.iter().all(|s| s.lsm != 4) {
        return Err("The product file contains no application memory for this device".into());
    }
    Ok(())
}

/// Build the list of download steps for a device.
pub fn build_plan(li: &LoadInfo, mask: &MaskInfo, dev: &Device, proj: &Project, kind: DlKind, active: Option<&Active>) -> Result<Plan, String> {
    supported(li, Some(mask))?;
    let has_pei = li.procedures.contains_key(&3) || li.segments.iter().any(|s| s.lsm == 5);
    let (ptype, wanted): (&str, Vec<&str>) = match kind {
        DlKind::Full => ("Load", if has_pei { vec!["all", "ap1"] } else { vec!["ap1", "all"] }),
        DlKind::Partial => ("Load", vec!["par,grp", "par"]),
        DlKind::Unload => ("Unload", vec!["all"]),
    };
    let proc_ = wanted
        .iter()
        .find_map(|w| mask.procedures.iter().find(|p| p.ptype == ptype && p.subtype == *w))
        .ok_or_else(|| format!("The device type has no \"{}\" procedure", wanted[0]))?;
    let phase = if kind == DlKind::Full { "full" } else { "par" };
    let applies = |a: &str| a.is_empty() || a.split(',').any(|x| x.trim() == phase);

    // Expand merges
    let mut steps_ld: Vec<Ld> = vec![];
    for s in &proc_.steps {
        match s {
            Ld::Merge { id } => {
                if let Some(v) = li.procedures.get(id) {
                    steps_ld.extend(v.iter().cloned());
                }
            }
            x => steps_ld.push(x.clone()),
        }
    }

    let images = if kind == DlKind::Unload { None } else { Some(build_images_with(li, dev, proj, active)?) };
    let mut warnings = images.as_ref().map(|i| i.warnings.clone()).unwrap_or_default();
    let mut out = vec![];
    let mut fill_of: HashMap<u8, u8> = HashMap::new();
    let mut ignore_next = false;
    let mut props_done = false;
    for s in steps_ld {
        match s {
            Ld::Connect => {
                out.push(Step::Connect);
                out.push(Step::CheckMask(mask.number));
                out.push(Step::Authorize);
            }
            Ld::Disconnect => out.push(Step::Disconnect),
            Ld::Restart => out.push(Step::Restart),
            Ld::MapError { mapped_to_ok } => ignore_next = mapped_to_ok,
            Ld::Unload { lsm } => {
                out.push(Step::Unload { lsm, ignore_errors: ignore_next });
                ignore_next = false;
            }
            Ld::Load { lsm } => out.push(Step::Load { lsm }),
            Ld::LoadCompleted { lsm } => out.push(Step::LoadCompleted { lsm }),
            Ld::RelSegment { lsm, size, mode, fill, applies: ap } => {
                if !applies(&ap) {
                    continue;
                }
                let size = if size <= 2 && lsm <= 3 {
                    // Table segments: size of the table we write
                    images.as_ref().and_then(|i| i.by_obj.get(&lsm)).map(|d| d.len() as u32).unwrap_or(size).max(2)
                } else {
                    size
                };
                if mode == 1 {
                    fill_of.insert(lsm, fill);
                }
                out.push(Step::Alloc { lsm, size, mode, fill });
            }
            Ld::WriteProp { obj, pid, start, data, verify, applies: ap } => {
                if !applies(&ap) {
                    continue;
                }
                let mut d = hex(&data);
                if pid == 13 && d.iter().all(|b| *b == 0) && d.len() == 5 {
                    if obj == 4 {
                        let [m1, m2] = li.manufacturer.to_be_bytes();
                        let [n1, n2] = li.app_number.to_be_bytes();
                        d = vec![m1, m2, n1, n2, li.app_version];
                    } else {
                        continue; // no PEI program id to write
                    }
                }
                out.push(Step::WriteProp { obj, pid, start, data: d, verify });
            }
            Ld::CompareProp { obj, pid, start, data } => out.push(Step::CompareProp { obj, pid, start, data: hex(&data) }),
            Ld::LoadImageProp { obj, pid, count } => out.push(Step::ImageProp { obj, pid, count }),
            Ld::WriteRelMem { obj, offset, size, verify, applies: ap } => {
                if !applies(&ap) {
                    continue;
                }
                let Some(img) = images.as_ref().and_then(|i| i.by_obj.get(&obj)) else {
                    if obj == 5 {
                        continue;
                    }
                    return Err(format!("No data for the {}", obj_name(obj)));
                };
                let start = (offset as usize).min(img.len());
                let end = (start + size as usize).min(img.len());
                // Blocks equal to the fill value are only skipped for very large segments: not every
                // device really pre-fills reserved memory, and leftover data breaks parameters.
                let skip_fill = if end - start > 65536 { fill_of.get(&obj).copied() } else { None };
                out.push(Step::WriteMem { obj, offset, data: img[start..end].to_vec(), verify, skip_fill });
                // Parameters stored in properties go right after the application memory.
                if obj == 4 && !props_done {
                    props_done = true;
                    for (o, p, bit, bits, v) in images.as_ref().map(|i| i.props.clone()).unwrap_or_default() {
                        out.push(Step::PatchProp { obj: o, pid: p, bit, bits, value: v });
                    }
                }
            }
            Ld::Merge { .. } => {}
            Ld::Other(name) => return Err(format!("The load procedure uses \"{name}\", which is not supported yet")),
        }
    }
    if kind != DlKind::Unload && !out.iter().any(|s| matches!(s, Step::Restart)) {
        out.push(Step::Restart);
    }
    if !out.iter().any(|s| matches!(s, Step::Disconnect | Step::Restart)) {
        out.push(Step::Disconnect);
    }
    if li.mem.is_empty() && kind != DlKind::Unload {
        warnings.push("No parameter memory locations were found; parameters keep their default values".into());
    }
    let kind_name = match kind {
        DlKind::Full => "Full download",
        DlKind::Partial => "Partial download",
        DlKind::Unload => "Unload",
    };
    Ok(Plan { kind: kind_name.into(), steps: out, warnings })
}

/// Read-only plan: read the device's load states, application id, tables and application
/// memory and compare them with what the project expects. Nothing is written.
pub fn build_check_plan(li: &LoadInfo, mask: &MaskInfo, dev: &Device, proj: &Project, active: Option<&Active>) -> Result<Plan, String> {
    supported(li, Some(mask))?;
    let images = build_images_with(li, dev, proj, active)?;
    let mut steps = vec![Step::Connect, Step::CheckMask(mask.number), Step::Authorize];
    for lsm in [4u8, 3, 2, 1] {
        steps.push(Step::ReadState { lsm });
    }
    let [m1, m2] = li.manufacturer.to_be_bytes();
    let [n1, n2] = li.app_number.to_be_bytes();
    steps.push(Step::CheckProp { obj: 4, pid: 13, data: vec![m1, m2, n1, n2, li.app_version] });
    for obj in [1u8, 2, 3, 4] {
        if let Some(d) = images.by_obj.get(&obj) {
            steps.push(Step::CheckMem { obj, data: d.clone() });
        }
    }
    steps.push(Step::Disconnect);
    Ok(Plan { kind: "Device check".into(), steps, warnings: images.warnings })
}

/// Apply bits into a property value (used for parameters stored in properties).
pub fn patch_prop(current: &mut Vec<u8>, bit: u32, bits: u32, value: &[u8]) -> Result<(), String> {
    let need = ((bit + bits) as usize).div_ceil(8);
    if current.len() < need {
        current.resize(need, 0);
    }
    put_bits(current, bit, bits, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits() {
        let mut b = vec![0u8; 2];
        put_bits(&mut b, 3, 2, &[0b11]).unwrap();
        assert_eq!(b[0], 0b0001_1000);
        put_bits(&mut b, 4, 8, &[0xAB]).unwrap();
        assert_eq!(b, vec![0b0001_1010, 0b1011_0000]);
        assert_eq!(encode("300", 16, Enc::Uint).unwrap()[6..], [0x01, 0x2C]);
        assert_eq!(size_code("1 bit"), 0);
        assert_eq!(size_code("2 bytes"), 8);
        assert_eq!(descriptor(Flags { c: true, r: true, w: false, t: true, u: false }, "1 bit"), [0x4F, 0]);
    }
}
