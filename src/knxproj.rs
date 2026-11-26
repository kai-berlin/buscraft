//! Turning the project part of a .knxproj (0.xml / project.xml) into a BusCraft project.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek};
use std::path::Path;

use roxmltree::Node as XNode;

use crate::knxfile::{dpt_text, medium_of, parse, project_xml, ImportErr, MfrData};
use crate::model::*;

pub(crate) fn read_project<R: Read + Seek>(z: &mut zip::ZipArchive<R>, path: &Path, password: Option<&str>, mfr: &MfrData, warnings: &mut Vec<String>) -> Result<Project, ImportErr> {
    let (xml, info, name_hint) = project_xml(z, path, password)?;
    project_from_xml(&xml, &info, &name_hint, mfr, warnings).map_err(ImportErr::Failed)
}

pub(crate) fn rtf_to_text(s: &str) -> String {
    if !s.trim_start().starts_with("{\\rtf") {
        return s.to_string();
    }
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut depth = 0i32;
    let mut skip_depth: Option<i32> = None;
    while let Some(c) = chars.next() {
        match c {
            '{' => depth += 1,
            '}' => {
                if skip_depth == Some(depth) {
                    skip_depth = None;
                }
                depth -= 1;
            }
            '\\' => {
                let mut word = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphabetic() {
                        word.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_digit() || n == '-' {
                        chars.next();
                    } else {
                        break;
                    }
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                match word.as_str() {
                    "par" | "line" => {
                        if skip_depth.is_none() {
                            out.push('\n')
                        }
                    }
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "generator" => skip_depth = Some(depth),
                    "" => {
                        if let Some(&n) = chars.peek() {
                            if n == '\'' {
                                chars.next();
                                let h: String = chars.by_ref().take(2).collect();
                                if let (Ok(b), None) = (u8::from_str_radix(&h, 16), skip_depth) {
                                    out.push(b as char);
                                }
                            } else {
                                if skip_depth.is_none() && n != '*' {
                                    out.push(n);
                                }
                                if n == '*' {
                                    skip_depth = Some(depth);
                                }
                                chars.next();
                            }
                        }
                    }
                    _ => {}
                }
            }
            '\r' | '\n' => {}
            _ => {
                if skip_depth.is_none() {
                    out.push(c);
                }
            }
        }
    }
    out.trim().to_string()
}

fn space_kind(t: &str) -> SpaceKind {
    match t {
        "Building" => SpaceKind::Building,
        "BuildingPart" => SpaceKind::BuildingPart,
        "Floor" => SpaceKind::Floor,
        "Corridor" => SpaceKind::Corridor,
        "Stairway" => SpaceKind::Stairway,
        "DistributionBoard" | "Cabinet" => SpaceKind::DistributionBoard,
        _ => SpaceKind::Room,
    }
}

fn info_attr(info: &str, attr: &str) -> Option<String> {
    let d = parse(info).ok()?;
    d.descendants().find(|n| n.has_tag_name("ProjectInformation")).and_then(|n| n.attribute(attr).map(|s| s.to_string()))
}

pub fn project_from_xml(xml: &str, info_xml: &str, name_hint: &str, mfr: &MfrData, warnings: &mut Vec<String>) -> Result<Project, String> {
    let doc = parse(xml)?;
    let style = match info_attr(info_xml, "GroupAddressStyle").as_deref() {
        Some("TwoLevel") => GaStyle::TwoLevel,
        Some("Free") => GaStyle::Free,
        _ => GaStyle::ThreeLevel,
    };
    let name = info_attr(info_xml, "Name").filter(|s| !s.is_empty()).unwrap_or_else(|| name_hint.to_string());

    let mut p = Project::new(&name, Medium::IP, Medium::TP, style);
    p.areas.clear();
    p.buildings.clear();
    p.main_groups.clear();
    p.status = ProjectStatus::Commissioning;
    p.comment = "Imported from a KNX project file".into();
    if let Some(n) = info_attr(info_xml, "ProjectNumber") {
        p.number = n;
    }

    // Group addresses and ranges
    let mut range_names: HashMap<(u8, Option<u8>), String> = HashMap::new();
    for r in doc.descendants().filter(|n| n.has_tag_name("GroupRange")) {
        let start: u16 = r.attribute("RangeStart").and_then(|v| v.parse().ok()).unwrap_or(0);
        let end: u16 = r.attribute("RangeEnd").and_then(|v| v.parse().ok()).unwrap_or(0);
        let nm = r.attribute("Name").unwrap_or("").to_string();
        let size = end.saturating_sub(start);
        if size >= 2047 {
            range_names.insert(((start >> 11) as u8, None), nm);
        } else if size >= 255 {
            range_names.insert(((start >> 11) as u8, Some(((start >> 8) & 7) as u8)), nm);
        }
    }
    let mut ga_ids: HashMap<String, u32> = HashMap::new();
    for g in doc.descendants().filter(|n| n.has_tag_name("GroupAddress")) {
        let Some(gid) = g.attribute("Id") else { continue };
        let raw: u16 = g.attribute("Address").and_then(|v| v.parse().ok()).unwrap_or(0);
        let id = p.alloc_id();
        let (main, middle, sub) = ((raw >> 11) as u8, ((raw >> 8) & 7) as u8, (raw & 0xff) as u8);
        p.addresses.push(GroupAddress {
            id,
            main,
            middle,
            sub,
            name: g.attribute("Name").unwrap_or("").to_string(),
            dpt: dpt_text(g.attribute("DatapointType").unwrap_or(""), &mfr.dpt_names),
            central: g.attribute("Central") == Some("true"),
            pass_through: g.attribute("Unfiltered") == Some("true"),
            comment: rtf_to_text(g.attribute("Comment").or(g.attribute("Description")).unwrap_or("")),
        });
        ga_ids.insert(gid.to_string(), id);
        if let Some(short) = gid.rsplit('_').next() {
            ga_ids.insert(short.to_string(), id);
        }
        if !p.main_groups.iter().any(|m| m.number == main) {
            let n = range_names.get(&(main, None)).cloned().unwrap_or_else(|| format!("Main group {main}"));
            p.main_groups.push(MainGroup { number: main, name: n, middles: vec![] });
        }
        let mg = p.main_groups.iter_mut().find(|m| m.number == main).unwrap();
        if !mg.middles.iter().any(|m| m.number == middle) {
            let n = range_names.get(&(main, Some(middle))).cloned().unwrap_or_else(|| format!("Middle group {middle}"));
            mg.middles.push(MiddleGroup { number: middle, name: n });
        }
    }
    // Ranges without addresses still show up as groups.
    for ((m, mi), n) in &range_names {
        if !p.main_groups.iter().any(|x| x.number == *m) {
            let mn = range_names.get(&(*m, None)).cloned().unwrap_or_else(|| format!("Main group {m}"));
            p.main_groups.push(MainGroup { number: *m, name: mn, middles: vec![] });
        }
        if let Some(mi) = mi {
            let mg = p.main_groups.iter_mut().find(|x| x.number == *m).unwrap();
            if !mg.middles.iter().any(|x| x.number == *mi) {
                mg.middles.push(MiddleGroup { number: *mi, name: n.clone() });
            }
        }
    }
    p.main_groups.sort_by_key(|m| m.number);
    for m in &mut p.main_groups {
        m.middles.sort_by_key(|x| x.number);
    }

    // Topology and devices
    let mut dev_ids: HashMap<String, u32> = HashMap::new();
    let mut missing_apps = 0;
    for area in doc.descendants().filter(|n| n.has_tag_name("Area")) {
        let an: u8 = area.attribute("Address").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut a = Area { number: an, name: area.attribute("Name").unwrap_or("").to_string(), comment: String::new(), lines: vec![] };
        for line in area.children().filter(|n| n.has_tag_name("Line")) {
            let ln: u8 = line.attribute("Address").and_then(|v| v.parse().ok()).unwrap_or(0);
            let medium = line
                .attribute("MediumTypeRefId")
                .or_else(|| line.descendants().find(|n| n.has_tag_name("Segment")).and_then(|s| s.attribute("MediumTypeRefId")))
                .unwrap_or("");
            a.lines.push(Line { number: ln, name: line.attribute("Name").unwrap_or("").to_string(), medium: medium_of(medium), comment: String::new() });
            for di in line.descendants().filter(|n| n.has_tag_name("DeviceInstance")) {
                let dev = device(di, an, ln, &mut p, mfr, &ga_ids, &mut missing_apps);
                if let Some(did) = di.attribute("Id") {
                    dev_ids.insert(did.to_string(), dev.id);
                }
                p.devices.push(dev);
            }
        }
        a.lines.sort_by_key(|l| l.number);
        if let Some(x) = p.areas.iter_mut().find(|x| x.number == an) {
            x.lines.extend(a.lines);
        } else {
            p.areas.push(a);
        }
    }
    p.areas.sort_by_key(|a| a.number);
    if missing_apps > 0 {
        warnings.push(format!("{missing_apps} device(s) use an application that is not included in the file"));
    }

    // Buildings / locations
    fn space(n: XNode, p: &mut Project, dev_ids: &HashMap<String, u32>) -> Space {
        let id = p.alloc_id();
        let mut s = Space::new(id, n.attribute("Name").unwrap_or(""), space_kind(n.attribute("Type").unwrap_or("Room")));
        s.number = n.attribute("Number").unwrap_or("").to_string();
        for c in n.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "Space" | "BuildingPart" => {
                    let child = space(c, p, dev_ids);
                    s.children.push(child);
                }
                "DeviceInstanceRef" => {
                    if let Some(d) = c.attribute("RefId").and_then(|r| dev_ids.get(r)) {
                        s.devices.push(*d);
                    }
                }
                _ => {}
            }
        }
        s
    }
    let roots: Vec<XNode> = doc
        .descendants()
        .filter(|n| (n.has_tag_name("Space") || n.has_tag_name("BuildingPart")) && n.parent().map(|x| x.has_tag_name("Locations") || x.has_tag_name("Buildings")).unwrap_or(false))
        .collect();
    for r in roots {
        let s = space(r, &mut p, &dev_ids);
        p.buildings.push(s);
    }
    if p.buildings.is_empty() {
        let id = p.alloc_id();
        p.buildings.push(Space::new(id, &name, SpaceKind::Building));
    }
    if p.areas.is_empty() {
        warnings.push("The project has no topology".into());
    }
    Ok(p)
}

fn device(di: XNode, an: u8, ln: u8, p: &mut Project, mfr: &MfrData, ga_ids: &HashMap<String, u32>, missing_apps: &mut usize) -> Device {
    let pid = di.attribute("ProductRefId").unwrap_or("");
    let h2p = di.attribute("Hardware2ProgramRefId").unwrap_or("");
    let (ptext, order, _) = mfr.products.get(pid).cloned().unwrap_or_default();
    let app_id = mfr.hw2prog.get(h2p).map(|x| x.0.clone()).filter(|a| !a.is_empty());
    let app = app_id.as_ref().and_then(|a| mfr.apps.get(a));
    if app_id.is_some() && app.is_none() {
        *missing_apps += 1;
    }
    let id = p.alloc_id();
    let name = di.attribute("Name").filter(|s| !s.is_empty()).map(|s| s.to_string()).unwrap_or_else(|| ptext.clone());
    let serial = di
        .attribute("SerialNumber")
        .and_then(|s| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(s).ok()
        })
        .filter(|b| b.len() == 6)
        .map(|b| format!("{:02X}{:02X}:{:02X}{:02X}{:02X}{:02X}", b[0], b[1], b[2], b[3], b[4], b[5]))
        .unwrap_or_default();
    let loaded = |a: &str| di.attribute(a) == Some("true");
    let mut dev = Device {
        id,
        area: an,
        line: ln,
        number: di.attribute("Address").and_then(|v| v.parse().ok()).unwrap_or(0),
        name,
        description: di.attribute("Description").unwrap_or("").to_string(),
        comment: rtf_to_text(di.attribute("Comment").unwrap_or("")),
        manufacturer: mfr.mfr_name(pid),
        product: ptext,
        order_no: order,
        app_program: app.map(|a| format!("{} {}", a.name, a.version)).or(app_id.clone()).unwrap_or_default(),
        serial,
        params: vec![],
        objects: vec![],
        adr_ok: loaded("IndividualAddressLoaded"),
        prg_ok: loaded("ApplicationProgramLoaded"),
        par_ok: loaded("ParametersLoaded"),
        grp_ok: loaded("CommunicationPartLoaded"),
        cfg_ok: di.attribute("MediumConfigLoaded") != Some("false"),
        knx_app: app_id.clone(),
        knx_values: BTreeMap::new(),
    };
    let Some(app) = app else { return dev };
    for pr in di.descendants().filter(|n| n.has_tag_name("ParameterInstanceRef")) {
        if let (Some(r), Some(v)) = (pr.attribute("RefId"), pr.attribute("Value")) {
            if let Some(full) = app.resolve(r, false) {
                dev.knx_values.insert(full, v.to_string());
            }
        }
    }
    let mut links: HashMap<String, Vec<u32>> = HashMap::new();
    for cr in di.descendants().filter(|n| n.has_tag_name("ComObjectInstanceRef")) {
        let Some(r) = cr.attribute("RefId") else { continue };
        let Some(full) = app.resolve(r, true) else { continue };
        let mut l: Vec<u32> = vec![];
        let lookup = |t: &str| ga_ids.get(t).or_else(|| ga_ids.get(t.rsplit('_').next().unwrap_or(t))).copied();
        if let Some(s) = cr.attribute("Links") {
            for t in s.split_whitespace() {
                if let Some(g) = lookup(t) {
                    l.push(g);
                }
            }
        }
        for c in cr.descendants().filter(|n| n.has_tag_name("Send") || n.has_tag_name("Receive")) {
            if let Some(g) = c.attribute("GroupAddressRefId").and_then(lookup) {
                if !l.contains(&g) {
                    l.push(g);
                }
            }
        }
        if !l.is_empty() {
            links.insert(full, l);
        }
    }
    app.sync_objects(&mut dev);
    for (r, l) in links {
        if let Some(o) = dev.objects.iter_mut().find(|o| o.ref_id == r) {
            o.links = l;
        } else if let Some(o) = app.object_from(&r, l) {
            dev.objects.push(o);
        }
    }
    dev.objects.sort_by_key(|o| o.number);
    dev
}
