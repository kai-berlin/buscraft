//! Properties panel editors (right side of the workspace).

use egui::{RichText, Ui};

use crate::app::*;
use crate::bus::JobKind;
use crate::model::*;
use crate::theme;

#[derive(Default)]
pub struct Changed {
    pub any: bool,
    pub typing: bool,
    pub msg: Option<String>,
}

pub fn flatten(s: &Space, path: &str, out: &mut Vec<(u32, String, SpaceKind)>) {
    let p = if path.is_empty() { s.name.clone() } else { format!("{path} › {}", s.name) };
    out.push((s.id, p.clone(), s.kind));
    for c in &s.children {
        flatten(c, &p, out);
    }
}

impl App {
    pub fn ws_properties(&mut self, ui: &mut Ui, proj: &mut Project) {
        ui.horizontal(|ui| {
            for (t, n) in [(PropTab::Settings, "Settings"), (PropTab::Comments, "Comments"), (PropTab::Information, "Information")] {
                if theme::tab(ui, self.ws.prop_tab == t, n).clicked() {
                    self.ws.prop_tab = t;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("props_scroll").auto_shrink([false; 2]).show(ui, |ui| {
            let mut ch = Changed::default();
            let tab = self.ws.prop_tab;
            match self.ws.p.sel {
                Sel::Device(d) => self.props_device(ui, proj, d, &mut ch),
                Sel::Space(s) => props_space(ui, proj, s, tab, &mut ch),
                Sel::Area(a) => props_area(ui, proj, a, None, tab, &mut ch),
                Sel::Line(a, l) => props_area(ui, proj, a, Some(l), tab, &mut ch),
                Sel::Main(m) => props_group(ui, proj, m, None, tab, &mut ch),
                Sel::Middle(m, mi) => props_group(ui, proj, m, Some(mi), tab, &mut ch),
                Sel::Ga(g) => props_ga(ui, proj, g, tab, &mut ch),
                Sel::None => props_project(ui, proj, tab, &mut ch),
            }
            if ch.typing {
                self.mark_typing();
            } else if ch.any {
                self.mark();
            }
            if let Some(m) = ch.msg {
                self.say(m);
            }
        });
    }

    fn props_device(&mut self, ui: &mut Ui, proj: &mut Project, id: u32, ch: &mut Changed) {
        let Some(dev) = proj.device(id).cloned() else { return };
        match self.ws.prop_tab {
            PropTab::Settings => {
                ui.label(RichText::new(format!("{} {}", dev.address(), dev.name)).strong());
                ui.label(RichText::new(dev.status_text()).color(if dev.needs_download() { theme::WARN } else { theme::OK }));
                ui.add_space(6.0);
                let mut name = dev.name.clone();
                let mut desc = dev.description.clone();
                let mut serial = dev.serial.clone();
                let mut number = dev.number;
                let mut line = (dev.area, dev.line);
                let mut rooms = vec![];
                for b in &proj.buildings {
                    flatten(b, "", &mut rooms);
                }
                let rooms: Vec<(u32, String)> = rooms.into_iter().filter(|(_, _, k)| k.holds_devices()).map(|(i, p, _)| (i, p)).collect();
                let cur_room = proj.room_of(id).map(|s| s.id);
                let mut room = cur_room;
                egui::Grid::new("pd").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("Name");
                    if ui.add(egui::TextEdit::singleline(&mut name).desired_width(180.0)).changed() {
                        ch.typing = true;
                    }
                    ui.end_row();
                    ui.label("Line");
                    let lt = proj.line(line.0, line.1).map(|l| format!("{}.{} {}", line.0, line.1, l.name)).unwrap_or_else(|| format!("{}.{}", line.0, line.1));
                    egui::ComboBox::from_id_salt("dline").selected_text(lt).width(170.0).show_ui(ui, |ui| {
                        for a in &proj.areas {
                            for l in &a.lines {
                                ui.selectable_value(&mut line, (a.number, l.number), format!("{}.{} {}", a.number, l.number, l.name));
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Individual address");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}.{}.", line.0, line.1)).monospace());
                        ui.add(egui::DragValue::new(&mut number).range(0..=255));
                    });
                    ui.end_row();
                    ui.label("Description");
                    if ui.add(egui::TextEdit::singleline(&mut desc).desired_width(180.0)).changed() {
                        ch.typing = true;
                    }
                    ui.end_row();
                    ui.label("Room");
                    let rt = room.and_then(|r| rooms.iter().find(|x| x.0 == r)).map(|x| x.1.rsplit(" › ").next().unwrap_or("").to_string()).unwrap_or_else(|| "– not placed –".into());
                    egui::ComboBox::from_id_salt("droom").selected_text(rt).width(170.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut room, None, "– not placed –");
                        for (rid, p) in &rooms {
                            ui.selectable_value(&mut room, Some(*rid), p);
                        }
                    });
                    ui.end_row();
                    ui.label("Serial number");
                    if ui.add(egui::TextEdit::singleline(&mut serial).desired_width(180.0)).changed() {
                        ch.typing = true;
                    }
                    ui.end_row();
                });

                let line_changed = line != (dev.area, dev.line);
                let number_changed = number != dev.number;
                if line_changed || number_changed {
                    let num = if line_changed && !number_changed { proj.free_device_number(line.0, line.1) } else { number };
                    if proj.address_taken(line.0, line.1, num, id) {
                        ch.msg = Some(format!("Address {}.{}.{} is already used by another device", line.0, line.1, num));
                    }
                    let d = proj.device_mut(id).unwrap();
                    d.area = line.0;
                    d.line = line.1;
                    d.number = num;
                    d.adr_ok = false;
                    ch.any = true;
                }
                {
                    let d = proj.device_mut(id).unwrap();
                    if name != d.name || desc != d.description || serial != d.serial {
                        d.name = name;
                        d.description = desc;
                        d.serial = serial;
                    }
                }
                if room != cur_room {
                    for b in &mut proj.buildings {
                        b.unassign(id);
                    }
                    if let Some(r) = room {
                        if let Some(s) = proj.space_mut(r) {
                            s.devices.push(id);
                        }
                    }
                    ch.any = true;
                }
                ui.add_space(10.0);
                theme::section(ui, "Commissioning");
                egui::Grid::new("pdst").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    let app = dev.has_app();
                    for (n, ok) in [("Individual address", dev.adr_ok), ("Application program", dev.prg_ok), ("Parameters", dev.par_ok), ("Group addresses", dev.grp_ok), ("Configuration", dev.cfg_ok)] {
                        ui.label(n);
                        theme::status_mark(ui, ok, app);
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(dev.has_app(), egui::Button::new("⬇ Download")).clicked() {
                        let kind = if dev.adr_ok { JobKind::Partial } else { JobKind::Full };
                        self.enqueue(proj, id, kind);
                    }
                    if ui.button("📍 Program address").clicked() {
                        self.enqueue(proj, id, JobKind::Address);
                    }
                    if ui.add_enabled(dev.knx_app.is_some(), egui::Button::new("🔍 Check device")).on_hover_text("Read the configuration back from the device and compare it with the project (writes nothing)").clicked() {
                        self.enqueue(proj, id, JobKind::Check);
                    }
                });
            }
            PropTab::Comments => {
                let mut c = dev.comment.clone();
                ui.label("Comment");
                if ui.add(egui::TextEdit::multiline(&mut c).desired_rows(10).desired_width(f32::INFINITY)).changed() {
                    proj.device_mut(id).unwrap().comment = c;
                    ch.typing = true;
                }
            }
            PropTab::Information => {
                egui::Grid::new("pdi").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                    for (k, v) in [
                        ("Manufacturer", dev.manufacturer.clone()),
                        ("Product", dev.product.clone()),
                        ("Order number", dev.order_no.clone()),
                        ("Application program", if dev.has_app() { dev.app_program.clone() } else { "–".into() }),
                        ("Medium", proj.line(dev.area, dev.line).map(|l| l.medium.short().to_string()).unwrap_or_default()),
                        ("Group objects", dev.objects.len().to_string()),
                        ("Linked objects", dev.objects.iter().filter(|o| !o.links.is_empty()).count().to_string()),
                        ("Parameters", dev.params.len().to_string()),
                        ("Changed parameters", dev.params.iter().filter(|p| p.value != p.default).count().to_string()),
                    ] {
                        ui.label(RichText::new(k).color(theme::MUTED));
                        ui.label(v);
                        ui.end_row();
                    }
                });
            }
        }
    }
}

fn comment_tab(ui: &mut Ui, text: &mut String, ch: &mut Changed) {
    ui.label("Comment");
    if ui.add(egui::TextEdit::multiline(text).desired_rows(10).desired_width(f32::INFINITY)).changed() {
        ch.typing = true;
    }
}

fn props_project(ui: &mut Ui, proj: &mut Project, tab: PropTab, ch: &mut Changed) {
    match tab {
        PropTab::Settings => {
            ui.label(RichText::new("Project").strong());
            ui.add_space(4.0);
            egui::Grid::new("pp").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ch.typing |= ui.add(egui::TextEdit::singleline(&mut proj.name).desired_width(180.0)).changed();
                ui.end_row();
                ui.label("Project number");
                ch.typing |= ui.add(egui::TextEdit::singleline(&mut proj.number).desired_width(180.0)).changed();
                ui.end_row();
                ui.label("Status");
                egui::ComboBox::from_id_salt("ppst").selected_text(proj.status.label()).show_ui(ui, |ui| {
                    for s in ProjectStatus::ALL {
                        ch.any |= ui.selectable_value(&mut proj.status, s, s.label()).changed();
                    }
                });
                ui.end_row();
                ui.label("Group address style");
                egui::ComboBox::from_id_salt("ppga").selected_text(proj.ga_style.label()).width(180.0).show_ui(ui, |ui| {
                    for s in [GaStyle::ThreeLevel, GaStyle::TwoLevel, GaStyle::Free] {
                        ch.any |= ui.selectable_value(&mut proj.ga_style, s, s.label()).changed();
                    }
                });
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Select an item in the tree to see and edit its properties.").color(theme::MUTED));
        }
        PropTab::Comments => comment_tab(ui, &mut proj.comment, ch),
        PropTab::Information => {
            egui::Grid::new("ppi").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                for (k, v) in [
                    ("Created", fmt_time(proj.created)),
                    ("Last modified", fmt_time(proj.modified)),
                    ("Devices", proj.devices.len().to_string()),
                    ("Group addresses", proj.addresses.len().to_string()),
                    ("Need download", proj.devices.iter().filter(|d| d.needs_download()).count().to_string()),
                ] {
                    ui.label(RichText::new(k).color(theme::MUTED));
                    ui.label(v);
                    ui.end_row();
                }
            });
        }
    }
}

fn props_space(ui: &mut Ui, proj: &mut Project, id: u32, tab: PropTab, ch: &mut Changed) {
    let mut devs = vec![];
    if let Some(s) = proj.space(id) {
        s.all_devices(&mut devs);
    }
    let Some(s) = proj.space_mut(id) else { return };
    match tab {
        PropTab::Settings => {
            ui.label(RichText::new(format!("{} {}", s.kind.icon(), s.kind.label())).strong());
            ui.add_space(4.0);
            egui::Grid::new("ps").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ch.typing |= ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(180.0)).changed();
                ui.end_row();
                ui.label("Number");
                ch.typing |= ui.add(egui::TextEdit::singleline(&mut s.number).desired_width(180.0)).changed();
                ui.end_row();
                ui.label("Type");
                let opts: &[SpaceKind] = match s.kind {
                    SpaceKind::Building => &[SpaceKind::Building],
                    SpaceKind::BuildingPart | SpaceKind::Floor => &[SpaceKind::BuildingPart, SpaceKind::Floor],
                    _ => &[SpaceKind::Room, SpaceKind::Corridor, SpaceKind::Stairway, SpaceKind::DistributionBoard],
                };
                egui::ComboBox::from_id_salt("pstype").selected_text(s.kind.label()).show_ui(ui, |ui| {
                    for k in opts {
                        ch.any |= ui.selectable_value(&mut s.kind, *k, k.label()).changed();
                    }
                });
                ui.end_row();
            });
        }
        PropTab::Comments => comment_tab(ui, &mut s.comment, ch),
        PropTab::Information => {
            egui::Grid::new("psi").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                ui.label(RichText::new("Contains").color(theme::MUTED));
                ui.label(format!("{} parts / rooms", s.children.len()));
                ui.end_row();
                ui.label(RichText::new("Devices (incl. below)").color(theme::MUTED));
                ui.label(devs.len().to_string());
                ui.end_row();
            });
        }
    }
}

fn props_area(ui: &mut Ui, proj: &mut Project, a: u8, l: Option<u8>, tab: PropTab, ch: &mut Changed) {
    let devs: Vec<String> = proj.devices.iter().filter(|d| d.area == a && l.map(|l| d.line == l).unwrap_or(true)).map(|d| d.product.clone()).collect();
    let has_power = devs.iter().any(|p| p.to_lowercase().contains("power supply"));
    match l {
        None => {
            let Some(area) = proj.areas.iter_mut().find(|x| x.number == a) else { return };
            match tab {
                PropTab::Settings => {
                    ui.label(RichText::new(format!("Area {a}")).strong());
                    egui::Grid::new("pa").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label("Name");
                        ch.typing |= ui.add(egui::TextEdit::singleline(&mut area.name).desired_width(180.0)).changed();
                        ui.end_row();
                        ui.label("Address");
                        ui.label(a.to_string());
                        ui.end_row();
                    });
                }
                PropTab::Comments => comment_tab(ui, &mut area.comment, ch),
                PropTab::Information => {
                    ui.label(format!("{} lines, {} devices", area.lines.len(), devs.len()));
                }
            }
        }
        Some(ln) => {
            let Some(line) = proj.line_mut(a, ln) else { return };
            match tab {
                PropTab::Settings => {
                    ui.label(RichText::new(format!("Line {a}.{ln}")).strong());
                    egui::Grid::new("pl").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label("Name");
                        ch.typing |= ui.add(egui::TextEdit::singleline(&mut line.name).desired_width(180.0)).changed();
                        ui.end_row();
                        ui.label("Medium");
                        egui::ComboBox::from_id_salt("plm").selected_text(line.medium.label()).show_ui(ui, |ui| {
                            for m in [Medium::TP, Medium::IP, Medium::RF] {
                                ch.any |= ui.selectable_value(&mut line.medium, m, m.label()).changed();
                            }
                        });
                        ui.end_row();
                        ui.label("Address");
                        ui.label(format!("{a}.{ln}"));
                        ui.end_row();
                    });
                    if line.medium == Medium::TP && ln != 0 && !has_power {
                        ui.add_space(6.0);
                        ui.label(RichText::new("⚠ This TP line has no power supply.").color(theme::WARN));
                    }
                    if devs.len() > 64 {
                        ui.label(RichText::new("⚠ More than 64 devices on one line segment.").color(theme::WARN));
                    }
                }
                PropTab::Comments => comment_tab(ui, &mut line.comment, ch),
                PropTab::Information => {
                    ui.label(format!("{} devices on this line", devs.len()));
                }
            }
        }
    }
}

fn props_group(ui: &mut Ui, proj: &mut Project, m: u8, mi: Option<u8>, tab: PropTab, ch: &mut Changed) {
    let count = proj.addresses.iter().filter(|g| g.main == m && mi.map(|x| g.middle == x).unwrap_or(true)).count();
    let Some(mg) = proj.main_groups.iter_mut().find(|x| x.number == m) else { return };
    let (label, name): (String, &mut String) = match mi {
        None => (format!("Main group {m}"), &mut mg.name),
        Some(x) => match mg.middles.iter_mut().find(|y| y.number == x) {
            Some(y) => (format!("Middle group {m}/{x}"), &mut y.name),
            None => return,
        },
    };
    match tab {
        PropTab::Settings => {
            ui.label(RichText::new(label).strong());
            egui::Grid::new("pg").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ch.typing |= ui.add(egui::TextEdit::singleline(name).desired_width(180.0)).changed();
                ui.end_row();
            });
        }
        PropTab::Comments => {
            ui.label(RichText::new("Comments are not available for groups.").color(theme::MUTED));
        }
        PropTab::Information => {
            ui.label(format!("{count} group addresses"));
        }
    }
}

fn props_ga(ui: &mut Ui, proj: &mut Project, id: u32, tab: PropTab, ch: &mut Changed) {
    let links: Vec<String> = proj
        .links_of(id)
        .iter()
        .map(|(d, oi)| {
            let dev = proj.device(*d).unwrap();
            format!("{} {} – {} {}", dev.address(), dev.name, dev.objects[*oi].name, dev.objects[*oi].function)
        })
        .collect();
    let mains: Vec<(u8, Vec<u8>)> = proj.main_groups.iter().map(|m| (m.number, m.middles.iter().map(|x| x.number).collect())).collect();
    let style = proj.ga_style;
    let Some(g) = proj.addresses.iter().find(|g| g.id == id).cloned() else { return };
    let mut ng = g.clone();
    match tab {
        PropTab::Settings => {
            ui.label(RichText::new(format!("Group address {}", g.fmt(style))).strong());
            egui::Grid::new("pga").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ch.typing |= ui.add(egui::TextEdit::singleline(&mut ng.name).desired_width(180.0)).changed();
                ui.end_row();
                ui.label("Address");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("pgam").selected_text(ng.main.to_string()).width(40.0).show_ui(ui, |ui| {
                        for (m, _) in &mains {
                            ui.selectable_value(&mut ng.main, *m, m.to_string());
                        }
                    });
                    ui.label("/");
                    let middles = mains.iter().find(|(m, _)| *m == ng.main).map(|x| x.1.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("pgami").selected_text(ng.middle.to_string()).width(40.0).show_ui(ui, |ui| {
                        for mi in &middles {
                            ui.selectable_value(&mut ng.middle, *mi, mi.to_string());
                        }
                    });
                    ui.label("/");
                    ui.add(egui::DragValue::new(&mut ng.sub).range(0..=255));
                });
                ui.end_row();
                ui.label("Data type");
                let dt = if ng.dpt.is_empty() { "– not set –".to_string() } else { ng.dpt.clone() };
                egui::ComboBox::from_id_salt("pgdpt").selected_text(dt).width(180.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut ng.dpt, String::new(), "– not set –");
                    for d in DPTS {
                        ui.selectable_value(&mut ng.dpt, d.to_string(), *d);
                    }
                });
                ui.end_row();
                ui.label("Central");
                ui.checkbox(&mut ng.central, "");
                ui.end_row();
                ui.label("Pass through line coupler");
                ui.checkbox(&mut ng.pass_through, "");
                ui.end_row();
            });
            ui.add_space(8.0);
            theme::section(ui, &format!("Linked objects ({})", links.len()));
            if links.is_empty() {
                ui.label(RichText::new("None").color(theme::MUTED));
            }
            for l in &links {
                ui.label(RichText::new(l).small());
            }
        }
        PropTab::Comments => {
            ui.label("Comment");
            if ui.add(egui::TextEdit::multiline(&mut ng.comment).desired_rows(10).desired_width(f32::INFINITY)).changed() {
                ch.typing = true;
            }
        }
        PropTab::Information => {
            ui.label(format!("Raw value: {}", g.raw()));
            ui.label(format!("Length: {}", dpt_size(&g.dpt)));
            ui.label(format!("{} linked objects", links.len()));
        }
    }
    let addr_new = (ng.main, ng.middle, ng.sub);
    let addr_old = (g.main, g.middle, g.sub);
    if addr_new != addr_old && proj.addresses.iter().any(|x| x.id != id && (x.main, x.middle, x.sub) == addr_new) {
        ch.msg = Some(format!("{}/{}/{} is already used", ng.main, ng.middle, ng.sub));
        ng.main = g.main;
        ng.middle = g.middle;
        ng.sub = g.sub;
    }
    let structural = (ng.main, ng.middle, ng.sub) != addr_old || ng.dpt != g.dpt || ng.central != g.central || ng.pass_through != g.pass_through;
    let text_changed = ng.name != g.name || ng.comment != g.comment;
    if structural {
        ch.any = true;
        for d in &mut proj.devices {
            if d.objects.iter().any(|o| o.links.contains(&id)) {
                d.grp_ok = false;
            }
        }
    }
    if structural || text_changed {
        *proj.ga_mut(id).unwrap() = ng;
    }
}
