//! The project workspace: header, menus, toolbar, tree, center tables and the device editor.

use egui::collapsing_header::CollapsingState;
use egui::{Align, Color32, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::*;
use crate::bus::JobKind;
use crate::model::*;
use crate::theme;

#[allow(deprecated)]
pub(crate) fn nid(salt: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    egui::Id::new(salt)
}

fn tree_node(ui: &mut Ui, id: egui::Id, leaf: bool, selected: bool, text: RichText, add_body: impl FnOnce(&mut Ui)) -> egui::Response {
    if leaf {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().icon_width + 2.0);
            ui.selectable_label(selected, text)
        })
        .inner
    } else {
        // Salted with the surrounding panel so two panels can show the same tree.
        let id = ui.scope_id().with(id);
        let state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
        let (_, hdr, _) = state.show_header(ui, |ui| ui.selectable_label(selected, text)).body(add_body);
        hdr.inner
    }
}

fn remove_space(list: &mut Vec<Space>, id: u32) -> bool {
    if let Some(i) = list.iter().position(|s| s.id == id) {
        list.remove(i);
        return true;
    }
    list.iter_mut().any(|s| remove_space(&mut s.children, id))
}

pub fn matches(filter: &str, texts: &[&str]) -> bool {
    filter.is_empty() || texts.iter().any(|t| t.to_lowercase().contains(filter))
}

fn flags_text(f: Flags) -> String {
    let mut s = String::new();
    for (b, c) in [(f.c, 'C'), (f.r, 'R'), (f.w, 'W'), (f.t, 'T'), (f.u, 'U')] {
        s.push(if b { c } else { '-' });
    }
    s
}

fn duplicates(proj: &Project) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for d in &proj.devices {
        if !d.has_app() {
            continue;
        }
        let a = d.address();
        if !seen.insert(a.clone()) && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}


impl App {
    pub fn workspace(&mut self, ui: &mut Ui, proj: &mut Project) {
        let ctx = ui.ctx().clone();

        if ctx.memory(|m| m.focused().is_none()) {
            if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z))) {
                self.do_undo(proj);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Y))) {
                self.do_redo(proj);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) && self.ws.p.sel != Sel::None {
                self.delete_selection(proj);
            }
        }

        // Remember navigation for back/forward.
        if (self.ws.p.view, self.ws.p.sel) != self.ws.p.last {
            self.ws.p.back.push(self.ws.p.last);
            if self.ws.p.back.len() > 50 {
                self.ws.p.back.remove(0);
            }
            self.ws.p.fwd.clear();
            self.ws.p.last = (self.ws.p.view, self.ws.p.sel);
        }

        // Continue a download that waited for re-imported product data.
        if self.retry_download && self.import_report.is_none() {
            self.retry_download = false;
            if let Some((pid, dev, kind)) = self.pending_download.take() {
                if pid == proj.id {
                    self.enqueue(proj, dev, kind);
                }
            }
        }

        self.ws_titlebar(ui, proj);
        self.ws_actionbar(ui, proj);
        let right = format!("{} · {} devices · {} group addresses", proj.name, proj.devices.len(), proj.addresses.len());
        self.status_bar(ui, &right);
        self.ws_sidebar(ui, proj);
        self.ws_panes(ui, proj);

        if let Some(id) = self.confirm_delete_device {
            let name = proj.device(id).map(|d| format!("{} {}", d.address(), d.name)).unwrap_or_default();
            let mut close = false;
            egui::Window::new("Delete device").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(&ctx, |ui| {
                ui.label(format!("Delete {name} from the project?"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(theme::BAD)).clicked() {
                        proj.remove_device(id);
                        self.ws.p.sel = Sel::None;
                        if self.ws.p.open_device == Some(id) {
                            self.ws.p.open_device = None;
                        }
                        self.mark();
                        self.say(format!("Deleted {name}"));
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                self.confirm_delete_device = None;
            }
        }
        self.download_confirm_dialog(&ctx, proj);
    }

    pub(crate) fn check_addresses(&mut self, proj: &Project) {
        let dups = duplicates(proj);
        if dups.is_empty() {
            self.say("Individual address check: no duplicate addresses found");
        } else {
            self.say(format!("Duplicate individual addresses: {}", dups.join(", ")));
        }
    }

    fn selected_device(&self) -> Option<u32> {
        match self.ws.p.sel {
            Sel::Device(d) => Some(d),
            _ => self.ws.p.open_device,
        }
    }

    pub(crate) fn download_menu(&mut self, ui: &mut Ui, proj: &mut Project) {
        let dev = self.selected_device();
        for kind in [JobKind::Full, JobKind::Partial, JobKind::Address, JobKind::Application, JobKind::Unload] {
            if ui.add_enabled(dev.is_some(), egui::Button::new(format!("{} (selected device)", kind.label()))).clicked() {
                if let Some(d) = dev {
                    self.enqueue(proj, d, kind);
                }
                ui.close();
            }
        }
        ui.separator();
        if ui.add_enabled(dev.is_some(), egui::Button::new("🔍 Check device (read back and compare)")).on_hover_text("Reads the group addresses, links, group objects and parameters from the device and compares them with the project. Nothing is written.").clicked() {
            if let Some(d) = dev {
                self.enqueue(proj, d, JobKind::Check);
            }
            ui.close();
        }
        ui.separator();
        if ui.button("Download all devices that need it").clicked() {
            self.download_all(proj);
            ui.close();
        }
    }

    pub(crate) fn enqueue(&mut self, proj: &Project, dev: u32, kind: JobKind) {
        let real = self.bus.is_real();
        let plan = match self.plan_for(proj, dev, kind) {
            Ok(p) => p,
            Err(e) if real => {
                let name = proj.device(dev).map(|d| format!("{} {}", d.address(), d.name)).unwrap_or_default();
                let reimport = e.contains("import its product file") || e.contains("Import its product file") || e.contains("not in the library");
                self.say(e.clone());
                self.problem = Some(crate::dl_ui::Problem { title: format!("{} – {name}", kind.label()), text: e, reimport });
                if reimport {
                    self.pending_download = Some((proj.id, dev, kind));
                }
                return;
            }
            Err(e) => {
                self.say(format!("Simulation only – a real download would not work yet: {e}"));
                None
            }
        };
        if real && kind != JobKind::Address && kind != JobKind::Check {
            let Some(plan) = plan else {
                let text = "This is a built-in sample product without real product data – it cannot be downloaded to a real device. Use a product imported from a .knxprod file.";
                self.say(text);
                self.problem = Some(crate::dl_ui::Problem { title: kind.label().into(), text: text.into(), reimport: false });
                return;
            };
            let d = proj.device(dev).unwrap();
            self.dl_confirm = Some(crate::dl_ui::DlConfirm { device: dev, kind, title: format!("{} to {} {}", plan.kind, d.address(), d.name), plan });
            return;
        }
        match self.bus.enqueue_plan(proj, dev, kind, plan.map(|p| p.steps)) {
            Ok(()) => {
                let n = proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
                self.say(format!("{}: {n}", kind.label()));
                self.open_dock(Dock::Jobs);
            }
            Err(e) => self.say(e),
        }
    }

    /// Show the catalog / a monitor in a panel (an existing one, or a new one).
    pub(crate) fn open_dock(&mut self, d: Dock) {
        self.ws.pending_show = Some(match d {
            Dock::Catalog => (View::Catalog, None),
            Dock::GroupMonitor => (View::Monitors, Some(MonView::GroupMonitor)),
            Dock::BusMonitor => (View::Monitors, Some(MonView::BusMonitor)),
            Dock::Jobs => (View::Monitors, Some(MonView::Jobs)),
        });
    }

    pub(crate) fn set_view(&mut self, v: View) {
        if self.ws.p.view != v {
            self.ws.p.view = v;
            self.ws.p.open_device = None;
            if !matches!(self.ws.p.sel, Sel::Device(_)) {
                self.ws.p.sel = Sel::None;
            }
        }
    }

    // ------------------------------------------------------------ tree

    pub(crate) fn ws_tree(&mut self, ui: &mut Ui, proj: &mut Project) {
        ui.add(egui::TextEdit::singleline(&mut self.ws.p.filter).hint_text("🔍 Filter").desired_width(f32::INFINITY));
        theme::divider(ui);
        let filter = self.ws.p.filter.to_lowercase();
        let mut new_sel: Option<Sel> = None;
        let mut open: Option<u32> = None;
        let mut folder: Option<u8> = None;
        let sel = self.ws.p.sel;
        let dev_folder = self.ws.p.dev_folder;
        let view = self.ws.p.view;
        egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
            // Search folders
            if view != View::Devices {
                let need = proj.devices.iter().filter(|d| d.needs_download()).count();
                let mut items: Vec<(u8, String)> = vec![];
                match view {
                    View::Buildings => {
                        let unplaced = proj.devices.iter().filter(|d| proj.room_of(d.id).is_none()).count();
                        items.push((2, format!("{unplaced}  Not assigned to a room")));
                        items.push((1, format!("{need}  Programming required")));
                    }
                    View::Topology => items.push((1, format!("{need}  Programming required"))),
                    _ => {
                        let unlinked = proj.addresses.iter().filter(|g| proj.links_of(g.id).is_empty()).count();
                        items.push((3, format!("{unlinked}  Addresses not linked")));
                    }
                }
                tree_node(ui, nid(("sf", view as u8)), false, false, RichText::new("📂 Search folders").strong(), |ui| {
                    for (n, t) in items {
                        if tree_node(ui, nid(("sfi", n)), true, sel == Sel::None && dev_folder == n, RichText::new(format!("📁 {t}")), |_| {}).clicked() {
                            folder = Some(n);
                            new_sel = Some(Sel::None);
                        }
                    }
                });
            }
            match view {
            View::Catalog | View::Monitors => {}
            View::Buildings => {
                if ui.selectable_label(sel == Sel::None && dev_folder == 0, RichText::new("🗂 All buildings").strong()).clicked() {
                    new_sel = Some(Sel::None);
                    folder = Some(0);
                }
                fn walk(ui: &mut Ui, s: &Space, proj: &Project, sel: Sel, filter: &str, new_sel: &mut Option<Sel>, open: &mut Option<u32>) {
                    let leaf = s.children.is_empty() && s.devices.is_empty();
                    let label = if s.number.is_empty() { format!("{} {}", s.kind.icon(), s.name) } else { format!("{} {} {}", s.kind.icon(), s.number, s.name) };
                    let r = tree_node(ui, nid(("sp", s.id)), leaf, sel == Sel::Space(s.id), RichText::new(label), |ui| {
                        for c in &s.children {
                            walk(ui, c, proj, sel, filter, new_sel, open);
                        }
                        for d in &s.devices {
                            if let Some(dev) = proj.device(*d) {
                                if !matches(filter, &[&dev.name, &dev.address(), &dev.product]) {
                                    continue;
                                }
                                let r = tree_node(ui, nid(("spd", s.id, *d)), true, sel == Sel::Device(*d), RichText::new(format!("🔌 {} {}", dev.address(), dev.name)), |_| {});
                                if r.clicked() {
                                    *new_sel = Some(Sel::Device(*d));
                                }
                                if r.clicked() {
                                    *open = Some(*d);
                                }
                            }
                        }
                    });
                    if r.clicked() {
                        *new_sel = Some(Sel::Space(s.id));
                    }
                }
                for b in &proj.buildings {
                    walk(ui, b, proj, sel, &filter, &mut new_sel, &mut open);
                }
            }
            View::Topology => {
                if ui.selectable_label(sel == Sel::None && dev_folder == 0, RichText::new("🗂 Topology").strong()).clicked() {
                    new_sel = Some(Sel::None);
                    folder = Some(0);
                }
                for a in &proj.areas {
                    let r = tree_node(ui, nid(("ar", a.number)), a.lines.is_empty(), sel == Sel::Area(a.number), RichText::new(format!("▦ {} {}", a.number, a.name)), |ui| {
                        for l in &a.lines {
                            let mut devs: Vec<&Device> = proj.devices.iter().filter(|d| d.area == a.number && d.line == l.number).collect();
                            devs.sort_by_key(|d| d.number);
                            let r = tree_node(
                                ui,
                                nid(("ln", a.number, l.number)),
                                devs.is_empty(),
                                sel == Sel::Line(a.number, l.number),
                                RichText::new(format!("═ {}.{} {}  [{}]", a.number, l.number, l.name, l.medium.short())),
                                |ui| {
                                    for d in devs {
                                        if !matches(&filter, &[&d.name, &d.address(), &d.product]) {
                                            continue;
                                        }
                                        let mut t = RichText::new(format!("🔌 {} {}", d.address(), d.name));
                                        if d.needs_download() {
                                            t = t.color(theme::WARN);
                                        }
                                        let r = tree_node(ui, nid(("dv", d.id)), true, sel == Sel::Device(d.id), t, |_| {});
                                        if r.clicked() {
                                            new_sel = Some(Sel::Device(d.id));
                                        }
                                        if r.clicked() {
                                            open = Some(d.id);
                                        }
                                    }
                                },
                            );
                            if r.clicked() {
                                new_sel = Some(Sel::Line(a.number, l.number));
                            }
                        }
                    });
                    if r.clicked() {
                        new_sel = Some(Sel::Area(a.number));
                    }
                }
            }
            View::GroupAddresses => {
                if ui.selectable_label(sel == Sel::None && dev_folder == 0, RichText::new("🗂 Group Addresses").strong()).clicked() {
                    new_sel = Some(Sel::None);
                    folder = Some(0);
                }
                for m in &proj.main_groups {
                    let r = tree_node(ui, nid(("mg", m.number)), m.middles.is_empty(), sel == Sel::Main(m.number), RichText::new(format!("▤ {} {}", m.number, m.name)), |ui| {
                        for mi in &m.middles {
                            let mut gas: Vec<&GroupAddress> = proj.addresses.iter().filter(|g| g.main == m.number && g.middle == mi.number).collect();
                            gas.sort_by_key(|g| g.sub);
                            let r = tree_node(
                                ui,
                                nid(("mi", m.number, mi.number)),
                                gas.is_empty(),
                                sel == Sel::Middle(m.number, mi.number),
                                RichText::new(format!("▥ {}/{} {}", m.number, mi.number, mi.name)),
                                |ui| {
                                    for g in gas {
                                        if !matches(&filter, &[&g.name, &g.fmt(proj.ga_style)]) {
                                            continue;
                                        }
                                        let linked = !proj.links_of(g.id).is_empty();
                                        let mut t = RichText::new(format!("{} {} {}", if linked { "◉" } else { "○" }, g.fmt(proj.ga_style), g.name));
                                        if !linked {
                                            t = t.color(theme::MUTED);
                                        }
                                        let r = tree_node(ui, nid(("ga", g.id)), true, sel == Sel::Ga(g.id), t, |_| {});
                                        if r.clicked() {
                                            new_sel = Some(Sel::Ga(g.id));
                                        }
                                        // Drag onto a group object to link – or drop one here
                                        self.dnd_link(&r, crate::panes::DragItem::Ga { id: g.id, label: format!("{} {}", g.fmt(proj.ga_style), g.name) });
                                    }
                                },
                            );
                            if r.clicked() {
                                new_sel = Some(Sel::Middle(m.number, mi.number));
                            }
                        }
                    });
                    if r.clicked() {
                        new_sel = Some(Sel::Main(m.number));
                    }
                }
            }
            View::Devices => {
                let all = proj.devices.len();
                let need = proj.devices.iter().filter(|d| d.needs_download()).count();
                let unplaced = proj.devices.iter().filter(|d| proj.room_of(d.id).is_none()).count();
                for (i, (n, c)) in [("🗂 All devices", all), ("⬇ Download required", need), ("◻ Not assigned to a room", unplaced)].into_iter().enumerate() {
                    if ui.selectable_label(dev_folder == i as u8 && sel == Sel::None, format!("{n} ({c})")).clicked() {
                        folder = Some(i as u8);
                        new_sel = Some(Sel::None);
                    }
                }
            }
            }
        });
        if let Some(f) = folder {
            self.ws.p.dev_folder = f;
        }
        if let Some(s) = new_sel {
            self.ws.p.sel = s;
            self.ws.p.open_device = None;
        }
        if let Some(d) = open {
            self.ws.p.sel = Sel::Device(d);
            self.ws.p.open_device = Some(d);
        }
    }

    pub(crate) fn add_menu(&mut self, ui: &mut Ui, proj: &mut Project) {
        match self.ws.p.view {
            View::Catalog | View::Monitors => {}
            View::Buildings => {
                let parent = match self.ws.p.sel {
                    Sel::Space(id) => proj.space(id).map(|s| (id, s.kind)),
                    _ => None,
                };
                ui.menu_button("+ Add ▾", |ui| {
                    let kinds: Vec<SpaceKind> = match parent {
                        Some((_, k)) => k.child_kinds().to_vec(),
                        None => vec![SpaceKind::Building],
                    };
                    if kinds.is_empty() {
                        ui.label("Nothing can be added here");
                    }
                    for k in kinds {
                        if ui.button(format!("{} {}", k.icon(), k.label())).clicked() {
                            let id = proj.alloc_id();
                            let s = Space::new(id, &format!("New {}", k.label().to_lowercase()), k);
                            match parent {
                                Some((pid, _)) => {
                                    if let Some(p) = proj.space_mut(pid) {
                                        p.children.push(s);
                                    }
                                }
                                None => proj.buildings.push(s),
                            }
                            self.ws.p.sel = Sel::Space(id);
                            self.mark();
                            ui.close();
                        }
                    }
                    if matches!(parent, Some((_, k)) if k.holds_devices()) && ui.button("🔌 Device from catalog…").clicked() {
                        self.open_dock(Dock::Catalog);
                        ui.close();
                    }
                });
            }
            View::Topology => {
                ui.menu_button("+ Add ▾", |ui| {
                    if ui.button("▦ Area").clicked() {
                        if let Some(n) = (1..=15u8).find(|n| !proj.areas.iter().any(|a| a.number == *n)) {
                            proj.areas.push(Area {
                                number: n,
                                name: "New area".into(),
                                comment: String::new(),
                                lines: vec![Line { number: 0, name: "Main line".into(), medium: Medium::TP, comment: String::new() }],
                            });
                            proj.areas.sort_by_key(|a| a.number);
                            self.ws.p.sel = Sel::Area(n);
                            self.mark();
                        } else {
                            self.say("All 15 areas are in use");
                        }
                        ui.close();
                    }
                    let area = match self.ws.p.sel {
                        Sel::Area(a) | Sel::Line(a, _) => Some(a),
                        Sel::Device(d) => proj.device(d).map(|d| d.area),
                        _ => proj.areas.first().map(|a| a.number),
                    };
                    if ui.add_enabled(area.is_some(), egui::Button::new("═ Line")).clicked() {
                        if let Some(a) = area.and_then(|a| proj.areas.iter_mut().find(|x| x.number == a)) {
                            if let Some(n) = (1..=15u8).find(|n| !a.lines.iter().any(|l| l.number == *n)) {
                                a.lines.push(Line { number: n, name: "New line".into(), medium: Medium::TP, comment: String::new() });
                                a.lines.sort_by_key(|l| l.number);
                                self.ws.p.sel = Sel::Line(a.number, n);
                                self.mark();
                            } else {
                                self.say("All 15 lines of this area are in use");
                            }
                        }
                        ui.close();
                    }
                    if ui.button("🔌 Device from catalog…").clicked() {
                        self.open_dock(Dock::Catalog);
                        ui.close();
                    }
                });
            }
            View::GroupAddresses => {
                ui.menu_button("+ Add ▾", |ui| {
                    if ui.button("▤ Main group").clicked() {
                        if let Some(n) = (0..=31u8).find(|n| !proj.main_groups.iter().any(|m| m.number == *n)) {
                            proj.main_groups.push(MainGroup { number: n, name: "New main group".into(), middles: vec![] });
                            proj.main_groups.sort_by_key(|m| m.number);
                            self.ws.p.sel = Sel::Main(n);
                            self.mark();
                        }
                        ui.close();
                    }
                    let main = match self.ws.p.sel {
                        Sel::Main(m) | Sel::Middle(m, _) => Some(m),
                        Sel::Ga(g) => proj.ga(g).map(|g| g.main),
                        _ => None,
                    };
                    if ui.add_enabled(main.is_some(), egui::Button::new("▥ Middle group")).clicked() {
                        if let Some(m) = main.and_then(|m| proj.main_groups.iter_mut().find(|x| x.number == m)) {
                            if let Some(n) = (0..=7u8).find(|n| !m.middles.iter().any(|x| x.number == *n)) {
                                m.middles.push(MiddleGroup { number: n, name: "New middle group".into() });
                                m.middles.sort_by_key(|x| x.number);
                                self.ws.p.sel = Sel::Middle(m.number, n);
                                self.mark();
                            }
                        }
                        ui.close();
                    }
                    let middle = match self.ws.p.sel {
                        Sel::Middle(m, mi) => Some((m, mi)),
                        Sel::Ga(g) => proj.ga(g).map(|g| (g.main, g.middle)),
                        _ => None,
                    };
                    if ui.add_enabled(middle.is_some(), egui::Button::new("◉ Group address")).clicked() {
                        if let Some((m, mi)) = middle {
                            let id = new_ga(proj, m, mi, "New group address", "");
                            self.ws.p.sel = Sel::Ga(id);
                            self.mark();
                        }
                        ui.close();
                    }
                });
            }
            View::Devices => {
                if ui.button("+ Add").clicked() {
                    self.open_dock(Dock::Catalog);
                }
            }
        }
    }

    pub(crate) fn delete_selection(&mut self, proj: &mut Project) {
        match self.ws.p.sel {
            Sel::None => return,
            Sel::Device(d) => {
                self.confirm_delete_device = Some(d);
                return;
            }
            Sel::Space(id) => {
                remove_space(&mut proj.buildings, id);
                self.say("Deleted from buildings (its devices stay in the topology)");
            }
            Sel::Area(a) => {
                let ids: Vec<u32> = proj.devices.iter().filter(|d| d.area == a).map(|d| d.id).collect();
                for d in ids {
                    proj.remove_device(d);
                }
                proj.areas.retain(|x| x.number != a);
                self.say(format!("Deleted area {a} and its devices"));
            }
            Sel::Line(a, l) => {
                let ids: Vec<u32> = proj.devices.iter().filter(|d| d.area == a && d.line == l).map(|d| d.id).collect();
                for d in ids {
                    proj.remove_device(d);
                }
                if let Some(ar) = proj.areas.iter_mut().find(|x| x.number == a) {
                    ar.lines.retain(|x| x.number != l);
                }
                self.say(format!("Deleted line {a}.{l} and its devices"));
            }
            Sel::Main(m) => {
                let ids: Vec<u32> = proj.addresses.iter().filter(|g| g.main == m).map(|g| g.id).collect();
                for g in ids {
                    proj.remove_ga(g);
                }
                proj.main_groups.retain(|x| x.number != m);
                self.say(format!("Deleted main group {m}"));
            }
            Sel::Middle(m, mi) => {
                let ids: Vec<u32> = proj.addresses.iter().filter(|g| g.main == m && g.middle == mi).map(|g| g.id).collect();
                for g in ids {
                    proj.remove_ga(g);
                }
                if let Some(mg) = proj.main_groups.iter_mut().find(|x| x.number == m) {
                    mg.middles.retain(|x| x.number != mi);
                }
                self.say(format!("Deleted middle group {m}/{mi}"));
            }
            Sel::Ga(g) => {
                let t = proj.ga_text(g);
                proj.remove_ga(g);
                self.say(format!("Deleted group address {t}"));
            }
        }
        self.ws.p.sel = Sel::None;
        self.mark();
    }

    // ------------------------------------------------------------ center

    pub(crate) fn ws_center(&mut self, ui: &mut Ui, proj: &mut Project) {
        let filter = self.ws.p.filter.to_lowercase();
        match (self.ws.p.view, self.ws.p.sel) {
            (View::Catalog | View::Monitors, _) => {}
            (_, Sel::Device(d)) => {
                let (t, s) = proj.device(d).map(|x| (format!("🔌 {} {}", x.address(), x.name), x.product.clone())).unwrap_or_default();
                center_title(ui, &t, &s);
                self.device_actions(ui, proj);
                self.device_table(ui, proj, &[d], "one");
            }
            (_, Sel::None) if self.ws.p.view != View::Devices && matches!(self.ws.p.dev_folder, 1 | 2) => {
                let folder = self.ws.p.dev_folder;
                self.device_list(ui, proj, folder, &filter);
            }
            (_, Sel::None) if self.ws.p.view == View::GroupAddresses && self.ws.p.dev_folder == 3 => {
                center_title(ui, "Addresses not linked", "Group addresses without any linked group object");
                let mut list: Vec<&GroupAddress> = proj.addresses.iter().filter(|g| proj.links_of(g.id).is_empty() && matches(&filter, &[&g.name, &g.fmt(proj.ga_style)])).collect();
                list.sort_by_key(|g| g.raw());
                let ids: Vec<u32> = list.iter().map(|g| g.id).collect();
                self.ga_table(ui, proj, &ids, f32::INFINITY);
            }
            (View::Buildings, Sel::Space(id)) => {
                let Some(s) = proj.space(id).cloned() else { return };
                center_title(ui, &format!("{} {}", s.kind.icon(), s.name), s.kind.label());
                if !s.children.is_empty() {
                    theme::section(ui, "Contains");
                    let mut pick = None;
                    theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
                        TableBuilder::new(ui)
                            .id_salt("children")
                            .striped(true)
                            .sense(Sense::click())
                            .max_scroll_height(170.0)
                            .cell_layout(Layout::left_to_right(Align::Center))
                            .column(Column::initial(240.0))
                            .column(Column::initial(150.0))
                            .column(Column::initial(80.0))
                            .column(Column::remainder())
                            .header(22.0, |mut h| {
                                for t in ["Name", "Type", "Number", "Devices"] {
                                    h.col(|ui| {
                                        ui.strong(t);
                                    });
                                }
                            })
                            .body(|mut body| {
                                for c in &s.children {
                                    body.row(24.0, |mut row| {
                                        row.col(|ui| {
                                            ui.label(format!("{} {}", c.kind.icon(), c.name));
                                        });
                                        row.col(|ui| {
                                            ui.label(c.kind.label());
                                        });
                                        row.col(|ui| {
                                            ui.label(&c.number);
                                        });
                                        row.col(|ui| {
                                            let mut v = vec![];
                                            c.all_devices(&mut v);
                                            ui.label(v.len().to_string());
                                        });
                                        if row.response().clicked() {
                                            pick = Some(c.id);
                                        }
                                    });
                                }
                            });
                    });
                    if let Some(p) = pick {
                        self.ws.p.sel = Sel::Space(p);
                    }
                    ui.add_space(6.0);
                }
                let mut ids = vec![];
                s.all_devices(&mut ids);
                let ids: Vec<u32> = ids.into_iter().filter(|d| proj.device(*d).map(|x| matches(&filter, &[&x.name, &x.address(), &x.product])).unwrap_or(false)).collect();
                theme::section(ui, &format!("Devices ({})", ids.len()));
                self.device_actions(ui, proj);
                if s.kind.holds_devices() {
                    self.assign_bar(ui, proj, id);
                }
                self.device_table(ui, proj, &ids, "space");
            }
            (View::Buildings, _) => {
                center_title(ui, "Buildings", "All buildings of the project");
                let mut ids = vec![];
                for b in &proj.buildings {
                    b.all_devices(&mut ids);
                }
                let ids: Vec<u32> = ids.into_iter().filter(|d| proj.device(*d).map(|x| matches(&filter, &[&x.name, &x.address(), &x.product])).unwrap_or(false)).collect();
                theme::section(ui, &format!("Devices placed in rooms ({})", ids.len()));
                self.device_actions(ui, proj);
                self.device_table(ui, proj, &ids, "bld");
            }
            (View::Topology, Sel::Line(a, l)) => {
                let name = proj.line(a, l).map(|x| format!("{a}.{l} {}", x.name)).unwrap_or_default();
                let medium = proj.line(a, l).map(|x| x.medium.label()).unwrap_or("");
                center_title(ui, &format!("═ {name}"), medium);
                let mut ds: Vec<&Device> = proj.devices.iter().filter(|d| d.area == a && d.line == l && matches(&filter, &[&d.name, &d.address(), &d.product])).collect();
                ds.sort_by_key(|d| d.number);
                let ids: Vec<u32> = ds.iter().map(|d| d.id).collect();
                self.device_actions(ui, proj);
                self.device_table(ui, proj, &ids, "line");
            }
            (View::Topology, Sel::Area(a)) => {
                let name = proj.areas.iter().find(|x| x.number == a).map(|x| x.name.clone()).unwrap_or_default();
                center_title(ui, &format!("▦ {a} {name}"), "Area");
                let lines: Vec<Line> = proj.areas.iter().find(|x| x.number == a).map(|x| x.lines.clone()).unwrap_or_default();
                let mut pick = None;
                theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
                    TableBuilder::new(ui)
                        .id_salt("lines")
                        .striped(true)
                        .sense(Sense::click())
                        .cell_layout(Layout::left_to_right(Align::Center))
                        .column(Column::initial(90.0))
                        .column(Column::initial(240.0))
                        .column(Column::initial(150.0))
                        .column(Column::remainder())
                        .header(22.0, |mut h| {
                            for t in ["Address", "Name", "Medium", "Devices"] {
                                h.col(|ui| {
                                    ui.strong(t);
                                });
                            }
                        })
                        .body(|mut body| {
                            for l in &lines {
                                body.row(24.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(format!("{a}.{}", l.number));
                                    });
                                    row.col(|ui| {
                                        ui.label(&l.name);
                                    });
                                    row.col(|ui| {
                                        ui.label(l.medium.label());
                                    });
                                    row.col(|ui| {
                                        ui.label(proj.devices.iter().filter(|d| d.area == a && d.line == l.number).count().to_string());
                                    });
                                    if row.response().clicked() {
                                        pick = Some(l.number);
                                    }
                                });
                            }
                        });
                });
                if let Some(l) = pick {
                    self.ws.p.sel = Sel::Line(a, l);
                }
            }
            (View::Topology, _) | (View::Devices, _) => {
                let folder = if self.ws.p.view == View::Devices { self.ws.p.dev_folder } else { 0 };
                self.device_list(ui, proj, folder, &filter);
            }
            (View::GroupAddresses, sel) => self.ga_center(ui, proj, sel, &filter),
        }
    }

    fn device_list(&mut self, ui: &mut Ui, proj: &mut Project, folder: u8, filter: &str) {
        let (title, sub) = match (self.ws.p.view, folder) {
            (_, 1) => ("Programming required", "Devices whose configuration differs from the device"),
            (_, 2) => ("Not assigned to a room", "Devices not placed in the building structure"),
            (View::Topology, _) => ("Topology", "All devices by individual address"),
            _ => ("All devices", "Every device in the project"),
        };
        center_title(ui, title, sub);
        let mut ds: Vec<&Device> = proj
            .devices
            .iter()
            .filter(|d| match folder {
                1 => d.needs_download(),
                2 => proj.room_of(d.id).is_none(),
                _ => true,
            })
            .filter(|d| matches(filter, &[&d.name, &d.address(), &d.product, &d.description]))
            .collect();
        ds.sort_by_key(|d| (d.area, d.line, d.number));
        let ids: Vec<u32> = ds.iter().map(|d| d.id).collect();
        self.device_actions(ui, proj);
        self.device_table(ui, proj, &ids, "all");
    }

    fn ga_center(&mut self, ui: &mut Ui, proj: &mut Project, sel: Sel, filter: &str) {
        let (title, sub, gas): (String, &str, Vec<u32>) = match sel {
            Sel::Main(m) => {
                let n = proj.main_groups.iter().find(|x| x.number == m).map(|x| x.name.clone()).unwrap_or_default();
                (format!("▤ {m} {n}"), "Main group", proj.addresses.iter().filter(|g| g.main == m).map(|g| g.id).collect())
            }
            Sel::Middle(m, mi) => {
                let n = proj.main_groups.iter().find(|x| x.number == m).and_then(|x| x.middles.iter().find(|y| y.number == mi)).map(|y| y.name.clone()).unwrap_or_default();
                (format!("▥ {m}/{mi} {n}"), "Middle group", proj.addresses.iter().filter(|g| g.main == m && g.middle == mi).map(|g| g.id).collect())
            }
            Sel::Ga(g) => {
                let (m, mi) = proj.ga(g).map(|x| (x.main, x.middle)).unwrap_or((0, 0));
                let name = proj.ga(g).map(|x| x.name.clone()).unwrap_or_default();
                (format!("◉ {} {}", proj.ga_text(g), name), "Group address", proj.addresses.iter().filter(|x| x.main == m && x.middle == mi).map(|x| x.id).collect())
            }
            _ => ("Group Addresses".into(), "All group addresses", proj.addresses.iter().map(|g| g.id).collect()),
        };
        center_title(ui, &title, sub);
        let mut list: Vec<&GroupAddress> = gas.iter().filter_map(|id| proj.ga(*id)).filter(|g| matches(filter, &[&g.name, &g.fmt(proj.ga_style), &g.dpt])).collect();
        list.sort_by_key(|g| g.raw());
        let gas: Vec<u32> = list.iter().map(|g| g.id).collect();
        let Sel::Ga(g) = sel else {
            self.ga_table(ui, proj, &gas, f32::INFINITY);
            return;
        };
        let _ = gas;
        self.ga_value_bar(ui, proj, g);
        ui.add_space(6.0);
        let links = proj.links_of(g);
        theme::section(ui, &format!("Linked group objects ({})", links.len()));
        let mut open = None;
        let mut pick = None;
        let sel_key = nid(("glsel", self.ws.p.id, g));
        let sel_obj: Option<(u32, usize)> = ui.data(|d| d.get_temp(sel_key));
        theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
            ui.set_min_height(ui.available_height().max(60.0));
            egui::ScrollArea::horizontal().id_salt("glinks_h").show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt("glinks")
                .striped(true)
                .resizable(true)
                .sense(Sense::click_and_drag())
                .max_scroll_height(f32::INFINITY)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::initial(80.0))
                .column(Column::initial(220.0).clip(true))
                .column(Column::initial(60.0))
                .column(Column::initial(220.0).clip(true))
                .column(Column::initial(70.0))
                .column(Column::remainder())
                .header(22.0, |mut h| {
                    for t in ["Address", "Device", "Number", "Object", "Flags", "Sending"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|mut body| {
                    for (d, oi) in &links {
                        let dev = proj.device(*d).unwrap();
                        let o = &dev.objects[*oi];
                        body.row(24.0, |mut row| {
                            row.col(|ui| {
                                ui.label(dev.address());
                            });
                            row.col(|ui| {
                                ui.label(&dev.name);
                            });
                            row.col(|ui| {
                                ui.label(o.number.to_string());
                            });
                            row.col(|ui| {
                                ui.label(format!("{} – {}", o.name, o.function));
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(flags_text(o.flags)).monospace());
                            });
                            row.col(|ui| {
                                ui.label(if o.links.first() == Some(&g) { "Yes" } else { "" });
                            });
                            row.set_selected(sel_obj == Some((*d, *oi)));
                            let r = row.response();
                            if r.clicked() {
                                pick = Some((*d, *oi));
                            }
                            if r.double_clicked() {
                                open = Some((*d, *oi));
                            }
                            self.dnd_link(&r, crate::panes::DragItem::Object { dev: *d, idx: *oi, label: format!("{} {} – {}", dev.address(), o.number, o.name) });
                        });
                    }
                });
            });
            if links.is_empty() {
                ui.label(RichText::new("Not linked yet. Drag a group object from a device onto this address, or open a device's group objects to link it.").color(theme::MUTED));
            }
        });
        if let Some(x) = pick {
            ui.data_mut(|d| d.insert_temp(sel_key, x));
        }
        if let Some((d, oi)) = open {
            self.ws.p.sel = Sel::Device(d);
            self.ws.p.open_device = Some(d);
            self.ws.p.dev_tab = DevTab::GroupObjects;
            self.ws.p.sel_object = Some(oi);
        }
    }

    /// Last value of a group address plus Write / Read on the bus.
    fn ga_value_bar(&mut self, ui: &mut Ui, proj: &Project, g: u32) {
        let Some(ga) = proj.ga(g) else { return };
        let last = self.bus.last_values.get(&ga.fmt(proj.ga_style)).cloned();
        let connected = self.bus.connected;
        let switch = ga.dpt.starts_with("1.") || ga.dpt == "1";
        let mut send: Option<(crate::bus::Service, String)> = None;
        theme::card_frame(ui).inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Last value").color(theme::MUTED));
                match &last {
                    Some((v, t)) => {
                        ui.label(RichText::new(if v.is_empty() { "–" } else { v.as_str() }).strong().size(15.0));
                        ui.label(RichText::new(format!("at {t}")).small().color(theme::MUTED));
                    }
                    None => {
                        ui.label(RichText::new("not seen yet").color(theme::MUTED));
                    }
                }
                ui.separator();
                ui.label("Value");
                let te = ui.add(egui::TextEdit::singleline(&mut self.ws.p.send_value).desired_width(80.0).hint_text(dpt_hint(&ga.dpt)));
                let enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if ui.add_enabled(connected, egui::Button::new("✎ Write")).on_hover_text("Send a GroupValueWrite with this value").clicked() || (enter && connected) {
                    send = Some((crate::bus::Service::Write, self.ws.p.send_value.trim().to_string()));
                }
                if switch {
                    if ui.add_enabled(connected, egui::Button::new("Off (0)")).clicked() {
                        send = Some((crate::bus::Service::Write, "0".into()));
                    }
                    if ui.add_enabled(connected, egui::Button::new("On (1)")).clicked() {
                        send = Some((crate::bus::Service::Write, "1".into()));
                    }
                }
                ui.separator();
                if ui.add_enabled(connected, egui::Button::new("⟳ Read")).on_hover_text("Ask the bus for the current value (GroupValueRead)").clicked() {
                    send = Some((crate::bus::Service::Read, String::new()));
                }
            });
            if !connected {
                ui.label(RichText::new("Not connected to the bus. Connect an interface to write or read values.").small().color(theme::MUTED));
            }
        });
        if let Some((s, v)) = send {
            if let Err(e) = self.bus.send_group(proj, g, s, &v, None) {
                self.say(e);
            }
        }
    }

    fn device_actions(&mut self, ui: &mut Ui, proj: &mut Project) {
        let sel = match self.ws.p.sel {
            Sel::Device(d) => Some(d),
            _ => None,
        };
        ui.horizontal(|ui| {
            if ui.add_enabled(sel.is_some(), egui::Button::new("✏ Parameters")).clicked() {
                self.ws.p.open_device = sel;
                self.ws.p.dev_tab = DevTab::Parameters;
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("🔗 Group objects")).clicked() {
                self.ws.p.open_device = sel;
                self.ws.p.dev_tab = DevTab::GroupObjects;
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("⬇ Download")).clicked() {
                if let Some(d) = sel {
                    let kind = if proj.device(d).map(|x| x.adr_ok).unwrap_or(false) { JobKind::Partial } else { JobKind::Full };
                    self.enqueue(proj, d, kind);
                }
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("🗑 Delete")).clicked() {
                self.confirm_delete_device = sel;
            }
            if ui.button("+ Add from catalog").clicked() {
                self.open_dock(Dock::Catalog);
            }
        });
        ui.add_space(4.0);
    }

    fn assign_bar(&mut self, ui: &mut Ui, proj: &mut Project, room: u32) {
        let unplaced: Vec<(u32, String)> = proj.devices.iter().filter(|d| proj.room_of(d.id).is_none()).map(|d| (d.id, format!("{} {}", d.address(), d.name))).collect();
        if unplaced.is_empty() {
            return;
        }
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Place a device here:").color(theme::MUTED));
            egui::ComboBox::from_id_salt("assign").selected_text("Choose an unplaced device…").width(280.0).show_ui(ui, |ui| {
                for (id, n) in &unplaced {
                    if ui.selectable_label(false, n).clicked() {
                        pick = Some(*id);
                    }
                }
            });
        });
        if let Some(id) = pick {
            if let Some(s) = proj.space_mut(room) {
                s.devices.push(id);
            }
            self.mark();
        }
        ui.add_space(4.0);
    }

    fn device_table(&mut self, ui: &mut Ui, proj: &Project, ids: &[u32], salt: &str) {
        let mut click = None;
        let mut dbl = None;
        let sel = self.ws.p.sel;
        theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
            ui.set_min_height(ui.available_height().clamp(80.0, 400.0));
            egui::ScrollArea::horizontal().id_salt(("devtable_h", salt)).show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt(("devtable", salt))
                .striped(true)
                .resizable(true)
                .sense(Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::initial(60.0))
                .column(Column::initial(210.0).clip(true))
                .columns(Column::exact(30.0), 5)
                .column(Column::initial(120.0).clip(true))
                .column(Column::initial(140.0).clip(true))
                .column(Column::initial(120.0).clip(true))
                .column(Column::initial(125.0).clip(true))
                .column(Column::initial(90.0).clip(true))
                .header(24.0, |mut h| {
                    for t in ["Address", "Name", "Adr", "Prg", "Par", "Grp", "Cfg", "Room", "Description", "Application", "Manufacturer", "Order number"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|mut body| {
                    for id in ids {
                        let Some(d) = proj.device(*id) else { continue };
                        body.row(24.0, |mut row| {
                            row.set_selected(sel == Sel::Device(*id));
                            row.col(|ui| {
                                ui.label(RichText::new(d.address()).monospace());
                            });
                            row.col(|ui| {
                                ui.label(&d.name);
                            });
                            let app = d.has_app();
                            for ok in [d.adr_ok, d.prg_ok, d.par_ok, d.grp_ok, d.cfg_ok] {
                                row.col(|ui| theme::status_mark(ui, ok, app));
                            }
                            row.col(|ui| {
                                ui.label(proj.room_name(d.id));
                            });
                            row.col(|ui| {
                                ui.label(&d.description);
                            });
                            row.col(|ui| {
                                ui.label(if d.has_app() { d.app_program.as_str() } else { "–" });
                            });
                            row.col(|ui| {
                                ui.label(&d.manufacturer);
                            });
                            row.col(|ui| {
                                ui.label(&d.order_no);
                            });
                            let r = row.response();
                            if r.clicked() {
                                click = Some(*id);
                            }
                            if r.double_clicked() {
                                dbl = Some(*id);
                            }
                        });
                    }
                });
            });
            if ids.is_empty() {
                ui.label(RichText::new("No devices here yet. Add one from the catalog.").color(theme::MUTED));
            }
        });
        if let Some(d) = click {
            self.ws.p.sel = Sel::Device(d);
        }
        if let Some(d) = dbl {
            self.ws.p.sel = Sel::Device(d);
            self.ws.p.open_device = Some(d);
        }
    }

    fn ga_table(&mut self, ui: &mut Ui, proj: &Project, ids: &[u32], max_h: f32) {
        let mut click = None;
        let last_values = self.bus.last_values.clone();
        let sel = self.ws.p.sel;
        theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt("gatable_h").show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt("gatable")
                .striped(true)
                .resizable(true)
                .sense(Sense::click_and_drag())
                .max_scroll_height(max_h)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::initial(80.0))
                .column(Column::initial(240.0).clip(true))
                .column(Column::initial(170.0).clip(true))
                .column(Column::initial(110.0).clip(true))
                .column(Column::initial(60.0))
                .column(Column::initial(60.0))
                .column(Column::initial(90.0))
                .column(Column::remainder())
                .header(24.0, |mut h| {
                    for t in ["Address", "Name", "Data type", "Last value", "Length", "Central", "Pass through", "Objects"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|mut body| {
                    for id in ids {
                        let Some(g) = proj.ga(*id) else { continue };
                        body.row(24.0, |mut row| {
                            row.set_selected(sel == Sel::Ga(*id));
                            row.col(|ui| {
                                ui.label(RichText::new(g.fmt(proj.ga_style)).monospace());
                            });
                            row.col(|ui| {
                                ui.label(&g.name);
                            });
                            row.col(|ui| {
                                ui.label(&g.dpt);
                            });
                            row.col(|ui| {
                                if let Some((v, t)) = last_values.get(&g.fmt(proj.ga_style)) {
                                    ui.label(RichText::new(if v.is_empty() { "–" } else { v.as_str() }).strong()).on_hover_text(format!("Received at {t}"));
                                }
                            });
                            row.col(|ui| {
                                ui.label(dpt_size(&g.dpt));
                            });
                            row.col(|ui| {
                                ui.label(if g.central { "Yes" } else { "" });
                            });
                            row.col(|ui| {
                                ui.label(if g.pass_through { "Yes" } else { "" });
                            });
                            row.col(|ui| {
                                ui.label(proj.links_of(g.id).len().to_string());
                            });
                            if row.response().clicked() {
                                click = Some(*id);
                            }
                            self.dnd_link(&row.response(), crate::panes::DragItem::Ga { id: g.id, label: format!("{} {}", g.fmt(proj.ga_style), g.name) });
                        });
                    }
                });
            });
            if ids.is_empty() {
                ui.label(RichText::new("No group addresses here. Use + Add in the tree to create one.").color(theme::MUTED));
            }
        });
        if let Some(g) = click {
            self.ws.p.sel = Sel::Ga(g);
        }
    }
}

pub fn new_ga(proj: &mut Project, main: u8, middle: u8, name: &str, dpt: &str) -> u32 {
    let id = proj.alloc_id();
    let sub = proj.free_sub(main, middle);
    proj.addresses.push(GroupAddress {
        id,
        main,
        middle,
        sub,
        name: name.into(),
        dpt: dpt.into(),
        central: false,
        pass_through: false,
        comment: String::new(),
    });
    id
}

pub fn center_title(ui: &mut Ui, title: &str, sub: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(title).size(17.0).strong()).truncate());
        if !sub.is_empty() {
            ui.add(egui::Label::new(RichText::new(sub).color(theme::MUTED)).truncate());
        }
    });
    ui.add_space(4.0);
}

/// Example input for a data type, shown in the empty value field.
fn dpt_hint(dpt: &str) -> &'static str {
    match dpt.split('.').next().unwrap_or("") {
        "1" => "0 / 1",
        "3" => "0–15",
        "5" => "0–100 %",
        "9" => "21.5",
        "10" => "Mon 14:30:00 / now",
        "11" => "25.11.2026 / today",
        "19" => "25.11.2026 14:30:00",
        "14" => "1.5",
        "16" => "text",
        "20" => "1",
        _ => "value",
    }
}