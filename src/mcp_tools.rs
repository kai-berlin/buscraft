//! What the MCP server's tools do. They run in the window loop (see `App::mcp_poll`), so
//! they see and change exactly what the person sees, and edits of the open project can be undone.

use crate::app::{App, Screen};
use crate::bus::Service;
use crate::model::{Device, GaStyle, GroupAddress, MainGroup, MiddleGroup, ParamKind, Product, Project, Space, SpaceKind};
use serde_json::{json, Value};

type R = Result<Value, String>;

fn arg_str<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    match a.get(k) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim()),
        _ => None,
    }
}
fn arg_text(a: &Value, k: &str) -> Option<String> {
    match a.get(k) {
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}
fn arg_int(a: &Value, k: &str) -> Option<i64> {
    match a.get(k) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}
fn arg_bool(a: &Value, k: &str) -> Option<bool> {
    match a.get(k) {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => match s.to_lowercase().as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}
fn need<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    arg_str(a, k).ok_or_else(|| format!("'{k}' is missing"))
}

/// "1/2/3", "1/515" or "2563" → (main, middle, sub).
pub fn parse_ga(t: &str) -> Result<(u8, u8, u8), String> {
    let bad = || format!("\"{t}\" is not a group address (use main/middle/sub, e.g. 1/2/3)");
    let p: Vec<u32> = t.trim().split('/').map(|x| x.trim().parse::<u32>()).collect::<Result<_, _>>().map_err(|_| bad())?;
    let raw = match p.as_slice() {
        [m, mi, s] if *m <= 31 && *mi <= 7 && *s <= 255 => (m << 11) | (mi << 8) | s,
        [m, s] if *m <= 31 && *s <= 2047 => (m << 11) | s,
        [r] if *r <= 65535 => *r,
        _ => return Err(bad()),
    };
    if raw == 0 {
        return Err("Group address 0/0/0 cannot be used".into());
    }
    Ok(((raw >> 11) as u8, ((raw >> 8) & 7) as u8, (raw & 0xff) as u8))
}

fn parse_ia(t: &str) -> Result<(u8, u8, u8), String> {
    let bad = || format!("\"{t}\" is not an individual address (use area.line.device, e.g. 1.1.5)");
    let p: Vec<u32> = t.trim().split('.').map(|x| x.trim().parse::<u32>()).collect::<Result<_, _>>().map_err(|_| bad())?;
    match p.as_slice() {
        [a, l, d] if *a <= 15 && *l <= 15 && *d <= 255 => Ok((*a as u8, *l as u8, *d as u8)),
        _ => Err(bad()),
    }
}

fn find_ga(p: &Project, t: &str) -> Result<u32, String> {
    let (m, mi, s) = parse_ga(t)?;
    p.addresses.iter().find(|g| g.main == m && g.middle == mi && g.sub == s).map(|g| g.id).ok_or_else(|| format!("Group address {t} does not exist in \"{}\"", p.name))
}

fn find_device(p: &Project, t: &str) -> Result<u32, String> {
    let t = t.trim();
    if let Ok((a, l, d)) = parse_ia(t) {
        if let Some(x) = p.devices.iter().find(|x| x.has_app() && x.area == a && x.line == l && x.number == d) {
            return Ok(x.id);
        }
    }
    if let Some(x) = p.devices.iter().find(|x| x.name.eq_ignore_ascii_case(t)) {
        return Ok(x.id);
    }
    if let Ok(id) = t.parse::<u32>() {
        if p.device(id).is_some() {
            return Ok(id);
        }
    }
    let hits: Vec<&Device> = p.devices.iter().filter(|x| x.name.to_lowercase().contains(&t.to_lowercase())).collect();
    match hits.as_slice() {
        [one] => Ok(one.id),
        [] => Err(format!("No device \"{t}\" in \"{}\"", p.name)),
        many => Err(format!("\"{t}\" matches several devices: {}", many.iter().map(|d| format!("{} {}", d.address(), d.name)).collect::<Vec<_>>().join(", "))),
    }
}

fn find_space_mut<'a>(spaces: &'a mut [Space], name: &str) -> Option<&'a mut Space> {
    for s in spaces.iter_mut() {
        if s.name.eq_ignore_ascii_case(name) {
            return Some(s);
        }
        if let Some(x) = find_space_mut(&mut s.children, name) {
            return Some(x);
        }
    }
    None
}

fn space_kind(t: &str) -> Result<SpaceKind, String> {
    use SpaceKind::*;
    Ok(match t.to_lowercase().replace(['_', '-'], " ").as_str() {
        "building" => Building,
        "building part" | "part" => BuildingPart,
        "floor" | "storey" => Floor,
        "room" => Room,
        "corridor" => Corridor,
        "stairway" | "stairs" => Stairway,
        "distribution board" | "cabinet" | "board" => DistributionBoard,
        _ => return Err(format!("Unknown kind \"{t}\"")),
    })
}

fn flags_text(o: &crate::model::GroupObject) -> String {
    let f = o.flags;
    [(f.c, 'C'), (f.r, 'R'), (f.w, 'W'), (f.t, 'T'), (f.u, 'U')].iter().filter(|x| x.0).map(|x| x.1).collect()
}

fn device_json(p: &Project, d: &Device) -> Value {
    json!({
        "id": d.id,
        "address": d.address(),
        "name": d.name,
        "description": d.description,
        "product": d.product,
        "manufacturer": d.manufacturer,
        "order_no": d.order_no,
        "room": p.room_name(d.id),
        "status": d.status_text(),
    })
}

fn ga_json(p: &Project, g: &GroupAddress, last: Option<&(String, String)>) -> Value {
    let links: Vec<Value> = p
        .links_of(g.id)
        .into_iter()
        .filter_map(|(dev, oi)| {
            let d = p.device(dev)?;
            let o = &d.objects[oi];
            Some(json!({"device": d.address(), "device_name": d.name, "object": o.number, "object_name": format!("{} – {}", o.name, o.function), "sending": o.links.first() == Some(&g.id)}))
        })
        .collect();
    let mut v = json!({
        "address": g.fmt(p.ga_style),
        "name": g.name,
        "dpt": g.dpt,
        "central": g.central,
        "comment": g.comment,
        "links": links,
    });
    if let Some((val, time)) = last {
        v["last_value"] = json!(val);
        v["last_value_time"] = json!(time);
    }
    v
}

fn space_json(s: &Space, p: &Project) -> Value {
    json!({
        "name": s.name,
        "kind": s.kind.label(),
        "devices": s.devices.iter().filter_map(|d| p.device(*d)).map(|d| format!("{} {}", d.address(), d.name)).collect::<Vec<_>>(),
        "children": s.children.iter().map(|c| space_json(c, p)).collect::<Vec<_>>(),
    })
}

fn style_name(s: GaStyle) -> &'static str {
    match s {
        GaStyle::ThreeLevel => "3-level",
        GaStyle::TwoLevel => "2-level",
        GaStyle::Free => "free",
    }
}

/// Make sure main group `m` and (for 3-level) middle group `mi` exist.
fn ensure_range(p: &mut Project, m: u8, mi: u8) {
    if !p.main_groups.iter().any(|g| g.number == m) {
        p.main_groups.push(MainGroup { number: m, name: format!("Main group {m}"), middles: vec![] });
        p.main_groups.sort_by_key(|g| g.number);
    }
    if p.ga_style == GaStyle::ThreeLevel {
        let mg = p.main_groups.iter_mut().find(|g| g.number == m).unwrap();
        if !mg.middles.iter().any(|x| x.number == mi) {
            mg.middles.push(MiddleGroup { number: mi, name: format!("Middle group {mi}") });
            mg.middles.sort_by_key(|x| x.number);
        }
    }
}

fn find_product<'a>(cat: &'a [Product], t: &str) -> Result<&'a Product, String> {
    let low = t.to_lowercase();
    if let Some(p) = cat.iter().find(|p| p.order_no.eq_ignore_ascii_case(t) || p.name.eq_ignore_ascii_case(t)) {
        return Ok(p);
    }
    let hits: Vec<&Product> = cat.iter().filter(|p| p.name.to_lowercase().contains(&low) || p.order_no.to_lowercase().contains(&low)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("No product \"{t}\" in the catalog (see list_catalog)")),
        many => Err(format!("\"{t}\" matches several products: {}", many.iter().take(8).map(|p| format!("{} ({})", p.name, p.order_no)).collect::<Vec<_>>().join(", "))),
    }
}

impl App {
    /// Carry out the tool calls that arrived since the last frame.
    pub fn mcp_poll(&mut self, ctx: &egui::Context) {
        self.mcp.set_ctx(ctx);
        for call in self.mcp.take_calls() {
            let out = self.mcp_call(&call.name, &call.args);
            let _ = call.reply.send(out);
        }
    }

    fn open_id(&self) -> Option<u64> {
        match self.screen {
            Screen::Project(id) => Some(id),
            Screen::Overview => None,
        }
    }

    fn mcp_project(&self, a: &Value) -> Result<usize, String> {
        if let Some(t) = arg_str(a, "project") {
            return self
                .store
                .projects
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(t) || p.id.to_string() == t)
                .ok_or_else(|| format!("No project \"{t}\". Projects: {}", self.store.projects.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")));
        }
        if let Some(id) = self.open_id() {
            if let Some(i) = self.store.projects.iter().position(|p| p.id == id) {
                return Ok(i);
            }
        }
        if self.store.projects.len() == 1 {
            return Ok(0);
        }
        Err(format!("No project is open in the app. Give 'project' or call open_project. Projects: {}", self.store.projects.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")))
    }

    /// Change a project; the open project gets an undo step.
    fn mcp_change(&mut self, a: &Value, what: &str, f: impl FnOnce(&mut Project, &[Product]) -> R) -> R {
        let idx = self.mcp_project(a)?;
        let before = self.store.projects[idx].clone();
        match f(&mut self.store.projects[idx], &self.catalog) {
            Ok(v) => {
                if self.open_id() == Some(before.id) {
                    self.undo.push(before);
                    if self.undo.len() > 100 {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                }
                self.store.projects[idx].touch();
                self.dirty = true;
                self.say(format!("Assistant: {what}"));
                Ok(v)
            }
            Err(e) => {
                self.store.projects[idx] = before;
                Err(e)
            }
        }
    }

    pub fn mcp_call(&mut self, name: &str, a: &Value) -> R {
        match name {
            "list_projects" => {
                let open = self.open_id();
                Ok(json!({"projects": self.store.projects.iter().map(|p| json!({
                    "id": p.id.to_string(), "name": p.name, "number": p.number, "status": p.status.label(),
                    "devices": p.devices.len(), "group_addresses": p.addresses.len(), "open": open == Some(p.id),
                })).collect::<Vec<_>>()}))
            }
            "open_project" => {
                let idx = self.mcp_project(a)?;
                let id = self.store.projects[idx].id;
                if self.open_id() != Some(id) {
                    self.open_project(id);
                }
                Ok(json!({"opened": self.store.projects[idx].name}))
            }
            "get_project" => {
                let p = &self.store.projects[self.mcp_project(a)?];
                Ok(json!({
                    "id": p.id.to_string(), "name": p.name, "number": p.number, "status": p.status.label(), "comment": p.comment,
                    "group_address_style": style_name(p.ga_style),
                    "topology": p.areas.iter().map(|ar| json!({"area": ar.number, "name": ar.name, "lines": ar.lines.iter().map(|l| json!({
                        "line": format!("{}.{}", ar.number, l.number), "name": l.name, "medium": l.medium.label(),
                        "devices": p.devices.iter().filter(|d| d.area == ar.number && d.line == l.number).count(),
                    })).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                    "buildings": p.buildings.iter().map(|s| space_json(s, p)).collect::<Vec<_>>(),
                    "group_ranges": p.main_groups.iter().map(|m| json!({"main": m.number, "name": m.name, "middles": m.middles.iter().map(|x| json!({"middle": format!("{}/{}", m.number, x.number), "name": x.name})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                    "devices": p.devices.len(),
                    "group_addresses": p.addresses.len(),
                    "devices_needing_download": p.devices.iter().filter(|d| d.needs_download()).count(),
                }))
            }
            "list_devices" => {
                let p = &self.store.projects[self.mcp_project(a)?];
                let f = arg_str(a, "filter").unwrap_or("").to_lowercase();
                let mut list: Vec<&Device> = p.devices.iter().collect();
                list.sort_by_key(|d| (d.area, d.line, d.number));
                Ok(json!({"devices": list.into_iter().map(|d| device_json(p, d)).filter(|v| f.is_empty() || v.to_string().to_lowercase().contains(&f)).collect::<Vec<_>>()}))
            }
            "get_device" => {
                let p = &self.store.projects[self.mcp_project(a)?];
                let d = p.device(find_device(p, need(a, "device")?)?).unwrap();
                let mut v = device_json(p, d);
                v["comment"] = json!(d.comment);
                v["serial"] = json!(d.serial);
                v["application"] = json!(d.app_program);
                v["commissioning"] = json!({"individual_address": d.adr_ok, "application_program": d.prg_ok, "parameters": d.par_ok, "group_addresses": d.grp_ok, "configuration": d.cfg_ok});
                if d.knx_app.is_some() {
                    v["parameters_note"] = json!("Imported product: its parameters can be viewed and changed in the app's Parameters tab.");
                } else {
                    v["parameters"] = json!(d.params.iter().map(|x| {
                        let mut pv = json!({"page": x.page, "name": x.name, "value": x.value, "default": x.default});
                        match &x.kind {
                            ParamKind::Choice(o) => pv["options"] = json!(o),
                            ParamKind::Number { min, max, unit } => { pv["min"] = json!(min); pv["max"] = json!(max); pv["unit"] = json!(unit); }
                            ParamKind::Flag => pv["options"] = json!(["0", "1"]),
                            ParamKind::Text => pv["type"] = json!("text"),
                        }
                        pv
                    }).collect::<Vec<_>>());
                }
                v["group_objects"] = json!(d.objects.iter().map(|o| json!({
                    "number": o.number, "name": o.name, "function": o.function, "dpt": o.dpt, "flags": flags_text(o),
                    "links": o.links.iter().filter_map(|l| p.ga(*l)).map(|g| g.fmt(p.ga_style)).collect::<Vec<_>>(),
                })).collect::<Vec<_>>());
                Ok(v)
            }
            "list_group_addresses" => {
                let p = &self.store.projects[self.mcp_project(a)?];
                let f = arg_str(a, "filter").unwrap_or("").to_lowercase();
                let mut list: Vec<&GroupAddress> = p.addresses.iter().filter(|g| f.is_empty() || g.fmt(p.ga_style).contains(&f) || g.name.to_lowercase().contains(&f) || g.dpt.to_lowercase().contains(&f)).collect();
                list.sort_by_key(|g| g.raw());
                Ok(json!({"style": style_name(p.ga_style), "group_addresses": list.into_iter().map(|g| ga_json(p, g, self.bus.last_values.get(&g.fmt(p.ga_style)))).collect::<Vec<_>>()}))
            }
            "list_catalog" => {
                let f = arg_str(a, "filter").unwrap_or("").to_lowercase();
                let limit = arg_int(a, "limit").unwrap_or(50).clamp(1, 500) as usize;
                let hits: Vec<&Product> = self.catalog.iter().filter(|p| f.is_empty() || [&p.manufacturer, &p.name, &p.order_no, &p.category].iter().any(|x| x.to_lowercase().contains(&f))).collect();
                Ok(json!({"total": hits.len(), "products": hits.iter().take(limit).map(|p| json!({
                    "name": p.name, "manufacturer": p.manufacturer, "order_no": p.order_no, "category": p.category, "medium": p.medium.label(), "imported": p.imported,
                })).collect::<Vec<_>>()}))
            }
            "create_group_address" => {
                let name = need(a, "name")?.to_string();
                let dpt = arg_text(a, "dpt").unwrap_or_default();
                let addr = arg_str(a, "address").map(|s| s.to_string());
                let (main, middle) = (arg_int(a, "main"), arg_int(a, "middle"));
                let central = arg_bool(a, "central").unwrap_or(false);
                let comment = arg_text(a, "comment").unwrap_or_default();
                self.mcp_change(a, &format!("created group address \"{name}\""), |p, _| {
                    let (m, mi, s) = match addr {
                        Some(t) => {
                            let x = parse_ga(&t)?;
                            if p.addresses.iter().any(|g| (g.main, g.middle, g.sub) == x) {
                                return Err(format!("Group address {t} already exists"));
                            }
                            x
                        }
                        None => {
                            let m = main.ok_or("Give 'address', or 'main' and 'middle'")?;
                            let mi = middle.unwrap_or(0);
                            if !(0..=31).contains(&m) || !(0..=7).contains(&mi) {
                                return Err("Main group must be 0…31 and middle group 0…7".into());
                            }
                            let s = p.free_sub(m as u8, mi as u8);
                            (m as u8, mi as u8, s)
                        }
                    };
                    ensure_range(p, m, mi);
                    let id = p.alloc_id();
                    p.addresses.push(GroupAddress { id, main: m, middle: mi, sub: s, name, dpt, central, pass_through: false, comment });
                    let g = p.ga(id).unwrap();
                    Ok(ga_json(p, g, None))
                })
            }
            "update_group_address" => {
                let t = need(a, "address")?.to_string();
                let a2 = a.clone();
                self.mcp_change(a, &format!("changed group address {t}"), |p, _| {
                    let id = find_ga(p, &t)?;
                    let new = match arg_str(&a2, "new_address") {
                        Some(n) => {
                            let x = parse_ga(n)?;
                            if p.addresses.iter().any(|g| g.id != id && (g.main, g.middle, g.sub) == x) {
                                return Err(format!("Group address {n} already exists"));
                            }
                            Some(x)
                        }
                        None => None,
                    };
                    let structural;
                    {
                        let g = p.ga_mut(id).unwrap();
                        let old = (g.main, g.middle, g.sub, g.dpt.clone(), g.central);
                        if let Some(n) = arg_text(&a2, "name") {
                            g.name = n;
                        }
                        if let Some(d) = arg_text(&a2, "dpt") {
                            g.dpt = d;
                        }
                        if let Some(c) = arg_bool(&a2, "central") {
                            g.central = c;
                        }
                        if let Some(c) = arg_text(&a2, "comment") {
                            g.comment = c;
                        }
                        if let Some((m, mi, s)) = new {
                            g.main = m;
                            g.middle = mi;
                            g.sub = s;
                        }
                        structural = old != (g.main, g.middle, g.sub, g.dpt.clone(), g.central);
                    }
                    if let Some((m, mi, _)) = new {
                        ensure_range(p, m, mi);
                    }
                    if structural {
                        for (dev, _) in p.links_of(id) {
                            p.device_mut(dev).unwrap().grp_ok = false;
                        }
                    }
                    Ok(ga_json(p, p.ga(id).unwrap(), None))
                })
            }
            "delete_group_address" => {
                let t = need(a, "address")?.to_string();
                self.mcp_change(a, &format!("deleted group address {t}"), |p, _| {
                    let id = find_ga(p, &t)?;
                    let links = p.links_of(id).len();
                    p.remove_ga(id);
                    Ok(json!({"deleted": t, "links_removed": links}))
                })
            }
            "name_group_range" => {
                let r = need(a, "range")?.to_string();
                let name = need(a, "name")?.to_string();
                self.mcp_change(a, &format!("named group range {r}"), |p, _| {
                    let parts: Vec<u8> = r.split('/').map(|x| x.trim().parse::<u8>()).collect::<Result<_, _>>().map_err(|_| format!("\"{r}\" is not a range (use 1 or 1/2)"))?;
                    match parts.as_slice() {
                        [m] if *m <= 31 => {
                            if !p.main_groups.iter().any(|g| g.number == *m) {
                                p.main_groups.push(MainGroup { number: *m, name: String::new(), middles: vec![] });
                                p.main_groups.sort_by_key(|g| g.number);
                            }
                            p.main_groups.iter_mut().find(|g| g.number == *m).unwrap().name = name.clone();
                        }
                        [m, mi] if *m <= 31 && *mi <= 7 => {
                            let style = p.ga_style;
                            p.ga_style = GaStyle::ThreeLevel;
                            ensure_range(p, *m, *mi);
                            p.ga_style = style;
                            let mg = p.main_groups.iter_mut().find(|g| g.number == *m).unwrap();
                            mg.middles.iter_mut().find(|x| x.number == *mi).unwrap().name = name.clone();
                        }
                        _ => return Err(format!("\"{r}\" is not a range (main 0…31, middle 0…7)")),
                    }
                    Ok(json!({"range": r, "name": name}))
                })
            }
            "link_group_address" | "unlink_group_address" => {
                let link = name == "link_group_address";
                let dev_t = need(a, "device")?.to_string();
                let obj = arg_int(a, "object").ok_or("'object' is missing")?;
                let ga_t = need(a, "address")?.to_string();
                let sending = arg_bool(a, "sending").unwrap_or(false);
                let what = if link { format!("linked {ga_t} to object {obj} of {dev_t}") } else { format!("unlinked {ga_t} from object {obj} of {dev_t}") };
                self.mcp_change(a, &what, |p, _| {
                    let did = find_device(p, &dev_t)?;
                    let gid = find_ga(p, &ga_t)?;
                    let gdpt = p.ga(gid).unwrap().dpt.clone();
                    let d = p.device_mut(did).unwrap();
                    let dname = d.name.clone();
                    let o = d.objects.iter_mut().find(|o| o.number as i64 == obj).ok_or_else(|| format!("{dname} has no group object {obj}"))?;
                    let mut warning = None;
                    if link {
                        if o.links.contains(&gid) {
                            if sending {
                                o.links.retain(|x| *x != gid);
                                o.links.insert(0, gid);
                            }
                        } else if sending {
                            o.links.insert(0, gid);
                        } else {
                            o.links.push(gid);
                        }
                        let main = |s: &str| s.split(['.', ' ']).next().unwrap_or("").to_string();
                        if !gdpt.is_empty() && !o.dpt.is_empty() && main(&gdpt) != main(&o.dpt) {
                            warning = Some(format!("Data types differ: object {} has {}, address has {}", o.number, o.dpt, gdpt));
                        }
                    } else {
                        if !o.links.contains(&gid) {
                            return Err(format!("Object {obj} of {dname} is not linked to {ga_t}"));
                        }
                        o.links.retain(|x| *x != gid);
                    }
                    let links: Vec<u32> = o.links.clone();
                    d.grp_ok = false;
                    let links: Vec<String> = links.iter().filter_map(|l| p.ga(*l)).map(|g| g.fmt(p.ga_style)).collect();
                    let mut v = json!({"device": dev_t, "object": obj, "links": links});
                    if let Some(w) = warning {
                        v["warning"] = json!(w);
                    }
                    Ok(v)
                })
            }
            "add_device" => {
                let prod_t = need(a, "product")?.to_string();
                let area = arg_int(a, "area").unwrap_or(1);
                let line = arg_int(a, "line").unwrap_or(1);
                let number = arg_int(a, "number");
                let dname = arg_text(a, "name");
                let space = arg_str(a, "space").map(|s| s.to_string());
                self.mcp_change(a, &format!("added \"{prod_t}\""), |p, cat| {
                    let prod = find_product(cat, &prod_t)?.clone();
                    if !(0..=15).contains(&area) || !(0..=15).contains(&line) || p.line(area as u8, line as u8).is_none() {
                        let lines: Vec<String> = p.areas.iter().flat_map(|a| a.lines.iter().map(move |l| format!("{}.{}", a.number, l.number))).collect();
                        return Err(format!("Line {area}.{line} does not exist. Lines: {}", lines.join(", ")));
                    }
                    let (area, line) = (area as u8, line as u8);
                    let id = p.add_device_from(&prod, area, line);
                    if let Some(n) = number {
                        if !(0..=255).contains(&n) {
                            return Err("Device number must be 0…255".into());
                        }
                        if p.address_taken(area, line, n as u8, id) {
                            return Err(format!("Address {area}.{line}.{n} is already used"));
                        }
                        p.device_mut(id).unwrap().number = n as u8;
                    }
                    if let Some(n) = dname {
                        p.device_mut(id).unwrap().name = n;
                    }
                    if let Some(s) = space {
                        let sp = p.buildings.iter_mut().find_map(|b| if b.name.eq_ignore_ascii_case(&s) { Some(b) } else { find_space_mut(&mut b.children, &s) }).ok_or_else(|| format!("No room \"{s}\""))?;
                        if !sp.kind.holds_devices() {
                            return Err(format!("\"{s}\" is a {}; devices go in rooms, corridors, stairways or distribution boards", sp.kind.label()));
                        }
                        sp.devices.push(id);
                    }
                    Ok(device_json(p, p.device(id).unwrap()))
                })
            }
            "update_device" => {
                let t = need(a, "device")?.to_string();
                let a2 = a.clone();
                self.mcp_change(a, &format!("changed device {t}"), |p, _| {
                    let id = find_device(p, &t)?;
                    if let Some(na) = arg_str(&a2, "new_address") {
                        let (ar, l, n) = parse_ia(na)?;
                        if p.line(ar, l).is_none() {
                            return Err(format!("Line {ar}.{l} does not exist"));
                        }
                        if p.address_taken(ar, l, n, id) {
                            return Err(format!("Address {na} is already used by another device"));
                        }
                        let d = p.device_mut(id).unwrap();
                        if !d.has_app() {
                            return Err(format!("{} has no individual address (no application)", d.name));
                        }
                        if (d.area, d.line, d.number) != (ar, l, n) {
                            d.area = ar;
                            d.line = l;
                            d.number = n;
                            d.adr_ok = false;
                        }
                    }
                    let d = p.device_mut(id).unwrap();
                    if let Some(x) = arg_text(&a2, "name") {
                        d.name = x;
                    }
                    if let Some(x) = arg_text(&a2, "description") {
                        d.description = x;
                    }
                    if let Some(x) = arg_text(&a2, "comment") {
                        d.comment = x;
                    }
                    Ok(device_json(p, p.device(id).unwrap()))
                })
            }
            "delete_device" => {
                let t = need(a, "device")?.to_string();
                let open = self.open_id();
                let idx = self.mcp_project(a)?;
                let id = find_device(&self.store.projects[idx], &t)?;
                // Close the device's editor in the open views so nothing points at it.
                if open == Some(self.store.projects[idx].id) {
                    for pane in std::iter::once(&mut self.ws.p).chain(self.ws.panes.iter_mut()) {
                        if pane.open_device == Some(id) {
                            pane.open_device = None;
                        }
                        if pane.sel == crate::app::Sel::Device(id) {
                            pane.sel = crate::app::Sel::None;
                        }
                    }
                }
                self.mcp_change(a, &format!("deleted device {t}"), |p, _| {
                    let name = format!("{} {}", p.device(id).unwrap().address(), p.device(id).unwrap().name);
                    p.remove_device(id);
                    Ok(json!({"deleted": name}))
                })
            }
            "set_parameter" => {
                let t = need(a, "device")?.to_string();
                let pname = need(a, "parameter")?.to_string();
                let page = arg_str(a, "page").map(|s| s.to_lowercase());
                let value = arg_text(a, "value").ok_or("'value' is missing")?;
                self.mcp_change(a, &format!("set \"{pname}\" of {t}"), |p, _| {
                    let id = find_device(p, &t)?;
                    let d = p.device_mut(id).unwrap();
                    if d.knx_app.is_some() {
                        return Err("This device comes from an imported product; change its parameters in the app's Parameters tab.".into());
                    }
                    let hits: Vec<usize> = d.params.iter().enumerate().filter(|(_, x)| x.name.eq_ignore_ascii_case(&pname) && page.as_ref().map(|pg| x.page.to_lowercase() == *pg).unwrap_or(true)).map(|(i, _)| i).collect();
                    let i = match hits.as_slice() {
                        [i] => *i,
                        [] => return Err(format!("{} has no parameter \"{pname}\"", d.name)),
                        _ => return Err(format!("\"{pname}\" is on several pages ({}); give 'page'", hits.iter().map(|i| d.params[*i].page.clone()).collect::<Vec<_>>().join(", "))),
                    };
                    let par = &mut d.params[i];
                    let v = value.trim();
                    let new = match &par.kind {
                        ParamKind::Choice(opts) => opts.iter().find(|o| o.eq_ignore_ascii_case(v)).cloned().ok_or_else(|| format!("\"{v}\" is not allowed; options: {}", opts.join(", ")))?,
                        ParamKind::Number { min, max, .. } => {
                            let n: i64 = v.parse().map_err(|_| format!("\"{v}\" is not a whole number"))?;
                            if n < *min || n > *max {
                                return Err(format!("Value must be {min}…{max}"));
                            }
                            n.to_string()
                        }
                        ParamKind::Flag => match v.to_lowercase().as_str() {
                            "1" | "true" | "yes" | "on" => "1".into(),
                            "0" | "false" | "no" | "off" => "0".into(),
                            _ => return Err("Use 1 or 0".into()),
                        },
                        ParamKind::Text => v.to_string(),
                    };
                    par.value = new.clone();
                    let page = par.page.clone();
                    d.par_ok = false;
                    Ok(json!({"device": t, "page": page, "parameter": pname, "value": new}))
                })
            }
            "create_space" => {
                let sname = need(a, "name")?.to_string();
                let kind = space_kind(need(a, "kind")?)?;
                let parent = arg_str(a, "parent").map(|s| s.to_string());
                self.mcp_change(a, &format!("added {} \"{sname}\"", kind.label().to_lowercase()), |p, _| {
                    let id = p.alloc_id();
                    let s = Space::new(id, &sname, kind);
                    if kind == SpaceKind::Building && parent.is_none() {
                        p.buildings.push(s);
                        return Ok(json!({"created": sname, "kind": kind.label()}));
                    }
                    let par = match &parent {
                        Some(n) => find_space_mut(&mut p.buildings, n).ok_or_else(|| format!("No building part or room \"{n}\""))?,
                        None => p.buildings.first_mut().ok_or("The project has no building; create one with kind 'building'")?,
                    };
                    if !par.kind.child_kinds().contains(&kind) {
                        return Err(format!("A {} cannot be added to a {}", kind.label(), par.kind.label()));
                    }
                    let pn = par.name.clone();
                    par.children.push(s);
                    Ok(json!({"created": sname, "kind": kind.label(), "parent": pn}))
                })
            }
            "assign_device" => {
                let t = need(a, "device")?.to_string();
                let sname = need(a, "space")?.to_string();
                self.mcp_change(a, &format!("put {t} in \"{sname}\""), |p, _| {
                    let id = find_device(p, &t)?;
                    if find_space_mut(&mut p.buildings, &sname).is_none() {
                        return Err(format!("No room \"{sname}\""));
                    }
                    for b in &mut p.buildings {
                        b.unassign(id);
                    }
                    let sp = find_space_mut(&mut p.buildings, &sname).unwrap();
                    if !sp.kind.holds_devices() {
                        return Err(format!("\"{sname}\" is a {}; devices go in rooms, corridors, stairways or distribution boards", sp.kind.label()));
                    }
                    sp.devices.push(id);
                    Ok(json!({"device": t, "space": sname}))
                })
            }
            "bus_status" => {
                let i = self.bus.iface();
                Ok(json!({
                    "interface": i.name, "kind": i.kind, "address": i.address, "simulated": !i.is_real(),
                    "connected": self.bus.connected, "state": self.bus.state_text(), "monitor_recording": self.bus.monitor_running,
                    "telegrams_recorded": self.bus.log.len(), "last_seq": self.bus.log.last().map(|t| t.seq).unwrap_or(0),
                }))
            }
            "read_telegrams" => {
                let after = arg_int(a, "after").unwrap_or(-1);
                let limit = arg_int(a, "limit").unwrap_or(100).clamp(1, 2000) as usize;
                let only = match arg_str(a, "address") {
                    Some(t) => {
                        let (m, mi, s) = parse_ga(t)?;
                        let style = self.mcp_project(a).map(|i| self.store.projects[i].ga_style).unwrap_or(GaStyle::ThreeLevel);
                        Some(GroupAddress { id: 0, main: m, middle: mi, sub: s, name: String::new(), dpt: String::new(), central: false, pass_through: false, comment: String::new() }.fmt(style))
                    }
                    None => None,
                };
                let list: Vec<&crate::bus::Telegram> = self.bus.log.iter().filter(|t| t.seq as i64 > after && only.as_ref().map(|o| &t.dest == o).unwrap_or(true)).collect();
                let skip = list.len().saturating_sub(limit);
                Ok(json!({"monitor_recording": self.bus.monitor_running, "telegrams": list[skip..].iter().map(|t| json!({
                    "seq": t.seq, "time": t.time, "service": t.service, "source": t.source, "source_name": t.source_name,
                    "destination": t.dest, "destination_name": t.dest_name, "dpt": t.dpt, "value": t.info, "priority": t.prio, "raw": t.raw,
                    "direction": if t.from_bus { "from bus" } else { "to bus" },
                })).collect::<Vec<_>>()}))
            }
            "send_group_value" | "read_group_value" => {
                let t = need(a, "address")?.to_string();
                let idx = self.mcp_project(a)?;
                let id = find_ga(&self.store.projects[idx], &t)?;
                let (service, value) = if name == "send_group_value" { (Service::Write, arg_text(a, "value").ok_or("'value' is missing")?) } else { (Service::Read, String::new()) };
                let proj = self.store.projects[idx].clone();
                self.bus.send_group(&proj, id, service, &value, None)?;
                let last = self.bus.log.last().filter(|x| x.dest == proj.ga(id).unwrap().fmt(proj.ga_style)).map(|x| (x.info.clone(), x.seq));
                self.say(format!("Assistant: {} {t}", if service == Service::Write { "wrote to" } else { "read" }));
                Ok(json!({"sent": service.label(), "address": t, "value": last.as_ref().map(|x| x.0.clone()).unwrap_or(value), "seq": last.map(|x| x.1)}))
            }
            _ => Err(format!("Unknown tool: {name}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut a = App::new(Some(std::path::PathBuf::from("unused.png")));
        let id = a.store.projects[0].id;
        a.open_project(id);
        a
    }

    #[test]
    fn every_listed_tool_is_handled() {
        let mut a = app();
        for t in crate::mcp::tool_list() {
            let n = t["name"].as_str().unwrap();
            let r = a.mcp_call(n, &json!({}));
            if let Err(e) = r {
                assert!(!e.starts_with("Unknown tool"), "{n} not handled");
            }
        }
    }

    #[test]
    fn create_link_write_and_undo() {
        let mut a = app();
        let undo0 = a.undo.len();
        let g = a.mcp_call("create_group_address", &json!({"address": "4/1/7", "name": "Clock time", "dpt": "10.001 time of day"})).unwrap();
        assert_eq!(g["address"], "4/1/7");
        assert_eq!(a.undo.len(), undo0 + 1);
        let p = &a.store.projects[0];
        assert!(p.main_groups.iter().any(|m| m.number == 4 && m.middles.iter().any(|x| x.number == 1)));

        // Next free address in a middle group
        let g2 = a.mcp_call("create_group_address", &json!({"main": 4, "middle": 1, "name": "Next"})).unwrap();
        assert_eq!(g2["address"], "4/1/1");
        assert!(a.mcp_call("create_group_address", &json!({"address": "4/1/7", "name": "dup"})).is_err());

        // Link an object of a device and see it in the device
        let devs = a.mcp_call("list_devices", &json!({})).unwrap();
        let addr = devs["devices"].as_array().unwrap().iter().find(|d| d["product"].as_str().unwrap().contains("Switch actuator")).unwrap()["address"].as_str().unwrap().to_string();
        let r = a.mcp_call("link_group_address", &json!({"device": addr, "object": 0, "address": "4/1/1"})).unwrap();
        assert!(r["links"].as_array().unwrap().iter().any(|x| x == "4/1/1"));
        let d = a.mcp_call("get_device", &json!({"device": addr})).unwrap();
        assert_eq!(d["commissioning"]["group_addresses"], false);

        // Write a value and read it back from the monitor
        let w = a.mcp_call("send_group_value", &json!({"address": "4/1/7", "value": "Tue 14:23:05"})).unwrap();
        assert_eq!(w["value"], "Tue 14:23:05");
        let tl = a.mcp_call("read_telegrams", &json!({"address": "4/1/7"})).unwrap();
        assert_eq!(tl["telegrams"].as_array().unwrap().last().unwrap()["value"], "Tue 14:23:05");

        // Parameters are checked
        let dd = a.mcp_call("get_device", &json!({"device": addr})).unwrap();
        let par = dd["parameters"].as_array().unwrap().iter().find(|x| x.get("options").is_some()).unwrap().clone();
        let opt = par["options"][0].as_str().unwrap();
        let mut args = json!({"device": addr, "parameter": par["name"], "page": par["page"], "value": opt});
        a.mcp_call("set_parameter", &args).unwrap();
        args["value"] = json!("no such option");
        assert!(a.mcp_call("set_parameter", &args).is_err());

        // Delete: links go away
        let del = a.mcp_call("delete_group_address", &json!({"address": "4/1/1"})).unwrap();
        assert_eq!(del["links_removed"], 1);
        assert!(a.store.projects[0].addresses.iter().all(|g| (g.main, g.middle, g.sub) != (4, 1, 1)));
    }

    #[test]
    fn rooms_and_devices() {
        let mut a = app();
        a.mcp_call("create_space", &json!({"name": "Attic", "kind": "floor"})).unwrap();
        a.mcp_call("create_space", &json!({"name": "Studio", "kind": "room", "parent": "Attic"})).unwrap();
        let d = a.mcp_call("add_device", &json!({"product": "Push button", "area": 1, "line": 1, "name": "Studio buttons", "space": "Studio"})).unwrap();
        assert_eq!(d["room"], "Studio");
        let addr = d["address"].as_str().unwrap().to_string();
        let free = a.mcp_call("update_device", &json!({"device": addr, "new_address": "1.1.200"})).unwrap();
        assert_eq!(free["address"], "1.1.200");
        assert!(a.mcp_call("add_device", &json!({"product": "Push button", "area": 9, "line": 9})).is_err());
        a.mcp_call("delete_device", &json!({"device": "Studio buttons"})).unwrap();
        assert!(a.mcp_call("get_device", &json!({"device": "Studio buttons"})).is_err());
    }

    #[test]
    fn address_parsing() {
        assert_eq!(parse_ga("1/2/3").unwrap(), (1, 2, 3));
        assert_eq!(parse_ga("1/515").unwrap(), (1, 2, 3));
        assert_eq!(parse_ga("2563").unwrap(), (1, 2, 3));
        assert!(parse_ga("32/0/0").is_err());
        assert!(parse_ga("0/0/0").is_err());
    }

    #[test]
    fn http_round_trip() {
        // The HTTP side answers initialize and tools/list without the window loop.
        let mut s = crate::mcp::McpServer::default();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        s.start(port);
        assert!(s.running, "{}", s.status);
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
        let body = crate::mcp::test_post(port, init).unwrap();
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "buscraft");
        let body = crate::mcp::test_post(port, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let v: Value = serde_json::from_str(&body).unwrap();
        assert!(v["result"]["tools"].as_array().unwrap().len() > 20);
        assert_eq!(crate::mcp::test_post(port, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap(), "");
        s.stop();
    }
}
