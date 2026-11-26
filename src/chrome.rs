//! Workspace chrome: green title bar with the project tab, the breadcrumb/action bar,
//! and the right sidebar (Properties, Open tasks, Running operations, Undo history).

use egui::{Align, Color32, Layout, Margin, RichText, Sense, Stroke, Ui};

use crate::app::*;
use crate::bus::{JobKind, JobState, LinkState};
use crate::model::*;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Side {
    Properties,
    OpenTasks,
    Running,
}

pub fn view_label(v: View) -> (&'static str, &'static str) {
    match v {
        View::Buildings => ("🏢", "Buildings"),
        View::Topology => ("🔗", "Topology"),
        View::GroupAddresses => ("🔢", "Group Addresses"),
        View::Devices => ("🔌", "Devices"),
        View::Catalog => ("📚", "Catalog"),
        View::Monitors => ("📈", "Monitors"),
    }
}

/// The app mark: a small rounded square with a bus-line glyph.
pub fn brand(ui: &mut Ui, bg: Color32, fg: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, bg);
    let c = rect.center();
    let s = Stroke::new(2.0, fg);
    p.line_segment([c + egui::vec2(-6.0, 5.0), c + egui::vec2(6.0, 5.0)], s);
    p.line_segment([c + egui::vec2(-6.0, 5.0), c + egui::vec2(-6.0, -5.0)], s);
    p.line_segment([c + egui::vec2(0.0, 5.0), c + egui::vec2(0.0, -2.0)], s);
    p.line_segment([c + egui::vec2(6.0, 5.0), c + egui::vec2(6.0, -6.0)], s);
}

fn side_header(ui: &mut Ui, icon: &str, text: &str, active: bool, badge: usize) -> egui::Response {
    let w = ui.available_width();
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 32.0), Sense::click());
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, 0.0, Color32::from_rgb(0xF2, 0xF8, 0xFD));
    }
    p.hline(rect.left()..=rect.right(), rect.bottom(), Stroke::new(1.0, theme::LINE));
    let font = egui::FontId::proportional(13.5);
    let col = if active { theme::ACCENT } else { Color32::from_rgb(0x1F, 0x1F, 0x1F) };
    p.text(rect.left_center() + egui::vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, icon, font.clone(), col);
    let t = p.text(rect.left_center() + egui::vec2(32.0, 0.0), egui::Align2::LEFT_CENTER, text, font.clone(), col);
    if badge > 0 {
        let b = egui::Rect::from_min_size(egui::pos2(t.right() + 8.0, rect.center().y - 9.0), egui::vec2(22.0, 18.0));
        p.rect_filled(b, 9.0, if active { theme::ACCENT } else { Color32::from_gray(0xE0) });
        p.text(b.center(), egui::Align2::CENTER_CENTER, badge.to_string(), egui::FontId::proportional(11.0), if active { Color32::WHITE } else { Color32::from_gray(0x40) });
    }
    p.text(rect.right_center() - egui::vec2(12.0, 0.0), egui::Align2::RIGHT_CENTER, if active { "▾" } else { "▸" }, egui::FontId::proportional(11.0), theme::MUTED);
    r
}

impl App {
    pub(crate) fn ws_titlebar(&mut self, ui: &mut Ui, proj: &Project) {
        let pending = self.bus.jobs.iter().filter(|j| j.project == proj.id && matches!(j.state, JobState::Waiting | JobState::PressButton | JobState::Running)).count();
        egui::Panel::top("ws_title").frame(egui::Frame::new().fill(theme::HEADER).inner_margin(Margin { left: 8, right: 10, top: 4, bottom: 0 })).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.vertical(|ui| {
                    ui.add_space(1.0);
                    ui.horizontal(|ui| {
                        brand(ui, Color32::WHITE, theme::HEADER);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new("BusCraft").strong().size(13.5).color(theme::HEADER_TEXT));
                            ui.label(RichText::new("Version 0.2 · KNXnet/IP").size(10.0).color(theme::HEADER_MUTED));
                        });
                    });
                });
                ui.add_space(18.0);
                // One tab per view of the project (like the original: project name + what it shows)
                let ntabs = self.tabs.len();
                for t in 0..ntabs {
                    let active = t == self.active_tab;
                    let sub = if active { self.ws.subtitle() } else { self.tabs[t].subtitle() };
                    let fill = if active { Color32::WHITE } else { theme::HEADER_DARK };
                    let fg = if active { Color32::from_rgb(0x1F, 0x1F, 0x1F) } else { Color32::WHITE };
                    let mut close_rect = egui::Rect::NOTHING;
                    let resp = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 })
                        .inner_margin(Margin { left: 10, right: 6, top: 3, bottom: 3 })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("📁").color(if active { theme::HEADER } else { Color32::WHITE }));
                                let name = ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    ui.label(RichText::new(&proj.name).strong().color(fg).size(13.0));
                                    ui.label(RichText::new(sub).color(if active { theme::MUTED } else { theme::HEADER_MUTED }).size(10.5));
                                });
                                ui.add_space((120.0 - name.response.rect.width()).max(10.0));
                                // The ✕ is drawn here; its click is handled by the tab's click area below
                                // (which lies on top of it).
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
                                close_rect = rect;
                                let hot = ui.rect_contains_pointer(rect);
                                if hot {
                                    let bg = if active { Color32::from_gray(0xE4) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 50) };
                                    ui.painter().rect_filled(rect, 3.0, bg);
                                }
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "✕", egui::FontId::proportional(11.0), fg);
                            });
                        });
                    let r = ui.interact(resp.response.rect, crate::workspace::nid(("tab", t)), Sense::click());
                    let on_close = r.interact_pointer_pos().or(r.hover_pos()).is_some_and(|p| close_rect.contains(p));
                    let r = if on_close {
                        r.on_hover_text(if ntabs > 1 { "Close this view" } else { "Close this view and the project" })
                    } else {
                        r
                    };
                    if (r.clicked() && on_close) || r.middle_clicked() {
                        self.tab_op = Some(TabOp::Close(t));
                    } else if r.clicked() && !active && self.tab_op.is_none() {
                        self.tab_op = Some(TabOp::Switch(t));
                    }
                }
                if ui.add(egui::Button::new(RichText::new("+").size(16.0).color(Color32::WHITE)).frame(false)).on_hover_text("Open another view of this project (with its own panels)").clicked() {
                    self.tab_op = Some(TabOp::New);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let x = ui.add(egui::Button::new(RichText::new("✕").size(15.0).color(Color32::WHITE)).frame(false).min_size(egui::vec2(28.0, 28.0)));
                    if x.on_hover_text("Close the project (its views are kept for next time)").clicked() {
                        self.tab_op = Some(TabOp::CloseProject);
                    }
                    ui.add_space(6.0);
                    let (dot, txt) = match &self.bus.state {
                        LinkState::Connected => (Color32::from_rgb(0xB8, 0xF5, 0xB0), format!("{} · {}", self.bus.iface().name, self.bus.own_address)),
                        LinkState::Connecting => (Color32::from_rgb(0xFF, 0xE0, 0x80), format!("{} · connecting…", self.bus.iface().name)),
                        LinkState::Failed(_) => (Color32::from_rgb(0xFF, 0xB0, 0xB0), format!("{} · failed", self.bus.iface().name)),
                        LinkState::Disconnected => (Color32::from_rgb(0xFF, 0xB0, 0xB0), "No bus connection".into()),
                    };
                    let r = ui.add(egui::Button::new(RichText::new(format!("● {txt}")).color(Color32::WHITE).size(12.0)).fill(theme::HEADER_DARK).stroke(Stroke::new(1.0, dot)));
                    if r.on_hover_text("Bus connections").clicked() {
                        self.screen = Screen::Overview;
                        self.ov_tab = OvTab::Bus;
                    }
                    if pending > 0 {
                        ui.label(RichText::new(format!("⟳ {pending} running")).color(Color32::from_rgb(0xFF, 0xF0, 0xB0)));
                    }
                });
            });
        });
    }

    fn crumbs(&self, proj: &Project) -> Vec<(String, Sel)> {
        let (vi, vt) = view_label(self.ws.p.view);
        let mut out = vec![(format!("{vi} {vt}"), Sel::None)];
        let dev_path = |out: &mut Vec<(String, Sel)>, d: &Device| out.push((format!("🔌 {} {}", d.address(), d.name), Sel::Device(d.id)));
        match (self.ws.p.view, self.ws.p.sel) {
            (View::Catalog | View::Monitors, _) => {}
            (View::Topology, sel) | (View::Devices, sel) => {
                let (a, l, dev) = match sel {
                    Sel::Area(a) => (Some(a), None, None),
                    Sel::Line(a, l) => (Some(a), Some(l), None),
                    Sel::Device(d) => match proj.device(d) {
                        Some(x) => (Some(x.area), Some(x.line), Some(x)),
                        None => (None, None, None),
                    },
                    _ => (None, None, None),
                };
                if self.ws.p.view == View::Topology {
                    if let Some(a) = a {
                        let n = proj.areas.iter().find(|x| x.number == a).map(|x| x.name.clone()).unwrap_or_default();
                        out.push((format!("▦ {a} {n}"), Sel::Area(a)));
                    }
                    if let (Some(a), Some(l)) = (a, l) {
                        let n = proj.line(a, l).map(|x| x.name.clone()).unwrap_or_default();
                        out.push((format!("═ {a}.{l} {n}"), Sel::Line(a, l)));
                    }
                }
                if let Some(d) = dev {
                    dev_path(&mut out, d);
                }
            }
            (View::Buildings, sel) => {
                let target = match sel {
                    Sel::Space(s) => Some(s),
                    Sel::Device(d) => proj.room_of(d).map(|s| s.id),
                    _ => None,
                };
                fn path(s: &Space, id: u32, acc: &mut Vec<(String, Sel)>) -> bool {
                    acc.push((format!("{} {}", s.kind.icon(), s.name), Sel::Space(s.id)));
                    if s.id == id {
                        return true;
                    }
                    for c in &s.children {
                        if path(c, id, acc) {
                            return true;
                        }
                    }
                    acc.pop();
                    false
                }
                if let Some(t) = target {
                    for b in &proj.buildings {
                        let mut acc = vec![];
                        if path(b, t, &mut acc) {
                            out.extend(acc);
                            break;
                        }
                    }
                }
                if let Sel::Device(d) = sel {
                    if let Some(d) = proj.device(d) {
                        dev_path(&mut out, d);
                    }
                }
            }
            (View::GroupAddresses, sel) => {
                let (m, mi, g) = match sel {
                    Sel::Main(m) => (Some(m), None, None),
                    Sel::Middle(m, mi) => (Some(m), Some(mi), None),
                    Sel::Ga(g) => match proj.ga(g) {
                        Some(x) => (Some(x.main), Some(x.middle), Some(x)),
                        None => (None, None, None),
                    },
                    Sel::Device(d) => {
                        if let Some(d) = proj.device(d) {
                            dev_path(&mut out, d);
                        }
                        (None, None, None)
                    }
                    _ => (None, None, None),
                };
                if let Some(m) = m {
                    let mg = proj.main_groups.iter().find(|x| x.number == m);
                    out.push((format!("▤ {m} {}", mg.map(|x| x.name.clone()).unwrap_or_default()), Sel::Main(m)));
                    if let Some(mi) = mi {
                        let n = mg.and_then(|x| x.middles.iter().find(|y| y.number == mi)).map(|y| y.name.clone()).unwrap_or_default();
                        out.push((format!("▥ {m}/{mi} {n}"), Sel::Middle(m, mi)));
                    }
                }
                if let Some(g) = g {
                    out.push((format!("◉ {} {}", g.fmt(proj.ga_style), g.name), Sel::Ga(g.id)));
                }
            }
        }
        out
    }

    pub(crate) fn ws_actionbar(&mut self, ui: &mut Ui, proj: &mut Project) {
        egui::Panel::top("ws_actions").frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(Margin::symmetric(8, 5))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if theme::tool(ui, !self.ws.p.back.is_empty(), "←").on_hover_text("Back").clicked() {
                    if let Some(prev) = self.ws.p.back.pop() {
                        self.ws.p.fwd.push((self.ws.p.view, self.ws.p.sel));
                        self.ws.p.view = prev.0;
                        self.ws.p.sel = prev.1;
                        self.ws.p.last = prev;
                        self.ws.p.open_device = None;
                    }
                }
                if theme::tool(ui, !self.ws.p.fwd.is_empty(), "→").on_hover_text("Forward").clicked() {
                    if let Some(next) = self.ws.p.fwd.pop() {
                        self.ws.p.back.push((self.ws.p.view, self.ws.p.sel));
                        self.ws.p.view = next.0;
                        self.ws.p.sel = next.1;
                        self.ws.p.last = next;
                        self.ws.p.open_device = None;
                    }
                }
                ui.add_space(8.0);
                let crumbs = self.crumbs(proj);
                let n = crumbs.len();
                let mut go = None;
                for (i, (t, s)) in crumbs.into_iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("/").color(theme::MUTED));
                    }
                    let text = if i + 1 == n { RichText::new(t).strong() } else { RichText::new(t) };
                    if ui.add(egui::Button::new(text).frame_when_inactive(false)).clicked() {
                        go = Some(s);
                    }
                }
                if let Some(s) = go {
                    self.ws.p.sel = s;
                    self.ws.p.dev_folder = 0;
                    if !matches!(s, Sel::Device(_)) {
                        self.ws.p.open_device = None;
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if theme::tool(ui, true, "?").on_hover_text("About").clicked() {
                        self.about = true;
                    }
                    if theme::tool(ui, true, "⚙").on_hover_text("Settings").clicked() {
                        self.screen = Screen::Overview;
                        self.ov_tab = OvTab::Settings;
                    }
                    if theme::tool(ui, true, "💾 Backup").on_hover_text("Back up everything (all projects and products in one .zip file)").clicked() {
                        self.backup_all();
                    }
                    ui.menu_button(RichText::new("▥ Panels ▾").size(12.5), |ui| {
                        ui.checkbox(&mut self.ws.side_open, "Sidebar (properties, operations)");
                        ui.separator();
                        ui.label(RichText::new("Add a panel showing").small().color(theme::MUTED));
                        for v in View::ALL {
                            let (vi, vt) = view_label(v);
                            if ui.button(format!("{vi} {vt}")).clicked() {
                                let last = self.ws.panes.len() - 1;
                                self.ws.pane_ops.push(PaneOp::Add(last, v));
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.add_enabled(self.ws.panes.len() > 1, egui::Button::new("Close the active panel")).clicked() {
                            self.ws.pane_ops.push(PaneOp::Close(self.ws.focus));
                            ui.close();
                        }
                        if ui.button("Open another view of the project").clicked() {
                            self.tab_op = Some(TabOp::New);
                            ui.close();
                        }
                    });
                    ui.separator();
                    if theme::tool(ui, !self.redo.is_empty(), "↷ Redo").on_hover_text("Redo (Ctrl+Y)").clicked() {
                        self.do_redo(proj);
                    }
                    if theme::tool(ui, !self.undo.is_empty(), "↶ Undo").on_hover_text("Undo (Ctrl+Z)").clicked() {
                        self.do_undo(proj);
                    }
                    ui.separator();
                    ui.menu_button(RichText::new("🖥 Diagnostics ▾").size(12.5), |ui| {
                        if ui.button("Group monitor").clicked() {
                            self.open_dock(Dock::GroupMonitor);
                            ui.close();
                        }
                        if ui.button("Bus monitor").clicked() {
                            self.open_dock(Dock::BusMonitor);
                            ui.close();
                        }
                        if ui.button("Check individual addresses").clicked() {
                            self.check_addresses(proj);
                            ui.close();
                        }
                        ui.separator();
                        match self.bus.state {
                            LinkState::Connected | LinkState::Connecting => {
                                if ui.button("Disconnect from bus").clicked() {
                                    self.bus.disconnect();
                                    ui.close();
                                }
                            }
                            _ => {
                                if ui.button(format!("Connect to {}", self.bus.iface().name)).clicked() {
                                    self.bus.connect();
                                    ui.close();
                                }
                            }
                        }
                        if ui.button("Bus connections…").clicked() {
                            self.screen = Screen::Overview;
                            self.ov_tab = OvTab::Bus;
                            ui.close();
                        }
                    });
                    ui.menu_button(RichText::new("⬇ Program ▾").size(12.5), |ui| self.download_menu(ui, proj));
                    if theme::tool(ui, self.ws.p.sel != Sel::None, "✕ Delete").on_hover_text("Delete the selected item (Del)").clicked() {
                        self.delete_selection(proj);
                    }
                    self.add_menu(ui, proj);
                });
            });
        });
        // Divider under the action bar
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.left()..=r.right(), r.top(), Stroke::new(1.0, theme::LINE));
    }

    pub(crate) fn ws_sidebar(&mut self, ui: &mut Ui, proj: &mut Project) {
        if !self.ws.side_open {
            return;
        }
        egui::Panel::right("side").default_size(310.0).min_size(240.0).max_size(440.0).frame(egui::Frame::new().fill(Color32::WHITE)).show(ui, |ui| {
            // Undo history pinned to the bottom
            egui::Panel::bottom("undo_hist").frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(Margin::symmetric(8, 6))).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("↶ Undo history").size(13.0));
                    ui.label(RichText::new(format!("{} step(s)", self.undo.len())).small().color(theme::MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::tool(ui, !self.redo.is_empty(), "Redo").clicked() {
                            self.do_redo(proj);
                        }
                        if theme::tool(ui, !self.undo.is_empty(), "Undo").clicked() {
                            self.do_undo(proj);
                        }
                    });
                });
            });
            let need = proj.devices.iter().filter(|d| d.needs_download()).count();
            let active = self.bus.jobs.iter().filter(|j| j.project == proj.id && matches!(j.state, JobState::Waiting | JobState::PressButton | JobState::Running)).count();
            for (s, icon, label, badge) in [
                (Side::Properties, "📄", "Properties", 0),
                (Side::OpenTasks, "📋", "Open tasks", need),
                (Side::Running, "▶", "Running operations", active),
            ] {
                if side_header(ui, icon, label, self.ws.side == s, badge).clicked() {
                    self.ws.side = s;
                }
            }
            egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| match self.ws.side {
                Side::Properties => self.ws_properties(ui, proj),
                Side::OpenTasks => self.open_tasks(ui, proj),
                Side::Running => self.running_ops(ui, proj),
            });
        });
    }

    fn open_tasks(&mut self, ui: &mut Ui, proj: &mut Project) {
        let list: Vec<(u32, String, &'static str)> = proj.devices.iter().filter(|d| d.needs_download()).map(|d| (d.id, format!("{} {}", d.address(), d.name), d.status_text())).collect();
        if list.is_empty() {
            ui.label(RichText::new("Nothing to do – all devices are up to date.").color(theme::MUTED));
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} device(s) need programming", list.len())).strong());
        });
        if ui.button("⬇ Program all").clicked() {
            self.download_all(proj);
            self.ws.side = Side::Running;
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("tasks").show(ui, |ui| {
            for (id, name, status) in list {
                let r = ui.selectable_label(self.ws.p.sel == Sel::Device(id), RichText::new(format!("⬇ {name}")));
                ui.label(RichText::new(status).small().color(theme::WARN));
                if r.clicked() {
                    self.ws.p.sel = Sel::Device(id);
                }
            }
        });
    }

    fn running_ops(&mut self, ui: &mut Ui, proj: &mut Project) {
        ui.columns(2, |c| {
            if theme::tab(&mut c[0], self.ws.run_tab == 0, "Active").clicked() {
                self.ws.run_tab = 0;
            }
            if theme::tab(&mut c[1], self.ws.run_tab == 1, "History").clicked() {
                self.ws.run_tab = 1;
            }
        });
        ui.add_space(4.0);
        let pid = proj.id;
        let jobs: Vec<usize> = self
            .bus
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| j.project == pid)
            .filter(|(_, j)| {
                let finished = matches!(j.state, JobState::Done | JobState::Failed);
                if self.ws.run_tab == 0 { !finished } else { finished }
            })
            .map(|(i, _)| i)
            .collect();
        if self.ws.run_tab == 1 {
            if theme::tool(ui, !jobs.is_empty(), "🗑 Clear history").clicked() {
                self.bus.jobs.retain(|j| j.project != pid || !matches!(j.state, JobState::Done | JobState::Failed));
                return;
            }
        }
        if jobs.is_empty() {
            ui.label(RichText::new(if self.ws.run_tab == 0 { "No operations running." } else { "No finished operations yet." }).color(theme::MUTED));
            return;
        }
        let mut press = None;
        let mut show_report = None;
        egui::ScrollArea::vertical().id_salt("runops").show(ui, |ui| {
            for i in jobs.into_iter().rev() {
                let j = &self.bus.jobs[i];
                let (mark, col) = match j.state {
                    JobState::Done => ("✔", theme::OK),
                    JobState::Failed => ("✖", theme::BAD),
                    JobState::PressButton => ("!", theme::WARN),
                    _ => ("⟳", theme::ACCENT),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(mark).color(col));
                    ui.label(RichText::new(format!("{} {}", j.address, j.name)).strong());
                });
                let kind = match j.kind {
                    JobKind::Address => "Program address",
                    JobKind::Partial => "Program (partial)",
                    JobKind::Unload => "Unload",
                    JobKind::Check => "Check device",
                    _ => "Program (all)",
                };
                let status = match j.state {
                    JobState::Done => "finished",
                    JobState::Failed => "failed",
                    _ => "",
                };
                ui.label(RichText::new(format!("▸ {kind}{}{status}", if status.is_empty() { "" } else { ": " })).small().color(if j.state == JobState::Failed { theme::BAD } else { theme::MUTED }));
                if matches!(j.state, JobState::Running | JobState::PressButton | JobState::Waiting) {
                    ui.add(egui::ProgressBar::new(j.progress).desired_height(6.0));
                }
                ui.add(egui::Label::new(RichText::new(&j.message).small().color(col)).wrap());
                if j.state == JobState::PressButton && !j.real && ui.small_button("Button pressed").clicked() {
                    press = Some(i);
                }
                if let Some(rep) = &j.report {
                    if ui.small_button("Show report").clicked() {
                        show_report = Some((format!("Device check {} {}", j.address, j.name), rep.clone()));
                    }
                }
                ui.add_space(6.0);
            }
        });
        if let Some(i) = press {
            self.bus.jobs[i].timer = 0.0;
        }
        if let Some(r) = show_report {
            self.report_view = Some(r);
        }
    }
}
