//! Panel contents: catalog, group monitor, bus monitor and pending operations.

use egui::{Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::*;
use crate::bus::{JobKind, JobState, Service};
use crate::model::*;
use crate::theme;

impl App {
    /// Catalog panel: add bar and the product list (manufacturers are in the panel's tree).
    /// Devices are added where the last used tree panel points (line or room).
    pub(crate) fn catalog_pane(&mut self, ui: &mut Ui, proj: &mut Project) {
        // Use the selection of the last used tree panel as the target.
        let (pv, ps) = (self.ws.p.view, self.ws.p.sel);
        self.ws.p.view = self.ws.last_tree.0;
        self.ws.p.sel = self.ws.last_tree.1;
        let target = self.add_target(proj);
        let mut add_sel = false;
        ui.horizontal(|ui| {
            ui.label("Elements");
            ui.add(egui::DragValue::new(&mut self.ws.add_count).range(1..=20));
            ui.label("in");
            ui.label(RichText::new(&target).strong());
            let can = self.ws.p.cat_sel.is_some();
            if ui.add_enabled(can, egui::Button::new("Add")).on_hover_text("Add the selected product (or double-click a product)").clicked() {
                add_sel = true;
            }
            ui.label(RichText::new("Select a line or room in another panel to choose where devices go.").small().color(theme::MUTED));
        });
        self.ws.p.view = pv;
        self.ws.p.sel = ps;
        let picked = self.catalog_browser(ui, true, false);
        let n = if add_sel { self.ws.add_count } else { 1 };
        if let Some(i) = picked.or(if add_sel { self.ws.p.cat_sel } else { None }) {
            self.ws.p.view = self.ws.last_tree.0;
            self.ws.p.sel = self.ws.last_tree.1;
            for _ in 0..n {
                self.add_product(proj, i);
            }
            self.ws.p.view = pv;
            self.ws.p.sel = ps;
        }
    }

    pub(crate) fn line_for_add(&self, proj: &Project) -> (u8, u8) {
        let first = || {
            proj.areas
                .iter()
                .find_map(|a| a.lines.iter().find(|l| l.number != 0).map(|l| (a.number, l.number)))
                .or_else(|| proj.areas.first().and_then(|a| a.lines.first().map(|l| (a.number, l.number))))
                .unwrap_or((1, 1))
        };
        match self.ws.p.sel {
            Sel::Line(a, l) => (a, l),
            Sel::Device(d) => proj.device(d).map(|d| (d.area, d.line)).unwrap_or_else(first),
            Sel::Area(a) => proj
                .areas
                .iter()
                .find(|x| x.number == a)
                .and_then(|x| x.lines.iter().find(|l| l.number != 0).or(x.lines.first()))
                .map(|l| (a, l.number))
                .unwrap_or((a, 0)),
            _ => first(),
        }
    }

    pub(crate) fn room_for_add(&self, proj: &Project) -> Option<u32> {
        match self.ws.p.sel {
            Sel::Space(s) => proj.space(s).filter(|x| x.kind.holds_devices()).map(|x| x.id),
            Sel::Device(d) if self.ws.p.view == View::Buildings => proj.room_of(d).map(|s| s.id),
            _ => None,
        }
    }

    pub(crate) fn add_target(&self, proj: &Project) -> String {
        let (a, l) = self.line_for_add(proj);
        let mut t = format!("line {a}.{l}");
        if let Some(r) = self.room_for_add(proj).and_then(|r| proj.space(r)) {
            t.push_str(&format!(" in {}", r.name));
        }
        t
    }

    pub(crate) fn add_product(&mut self, proj: &mut Project, i: usize) {
        if proj.areas.is_empty() {
            self.say("Add an area and a line in the topology first");
            return;
        }
        let (a, l) = self.line_for_add(proj);
        let room = self.room_for_add(proj);
        let prod = self.catalog[i].clone();
        let id = proj.add_device_from(&prod, a, l);
        if let Some(app_id) = &prod.app_id {
            match self.library.app(app_id) {
                Some(app) => app.sync_objects(proj.device_mut(id).unwrap()),
                None => self.say("The application data of this product is missing – import its product file again"),
            }
        }
        if let Some(r) = room {
            if let Some(s) = proj.space_mut(r) {
                s.devices.push(id);
            }
        }
        let addr = proj.device(id).unwrap().address();
        self.say(format!("Added {} as {addr}", prod.name));
        if self.ws.p.view != View::Buildings {
            self.ws.p.sel = Sel::Device(id);
        }
        self.mark();
    }

    /// Monitor view like the original's: toolbar, active interfaces, telegram list, counter.
    pub(crate) fn monitor(&mut self, ui: &mut Ui, proj: &mut Project, bus_mode: bool) {
        // Footer with the message counter
        egui::Panel::bottom(("monfoot", self.ws.p.id)).frame(egui::Frame::new().fill(egui::Color32::from_rgb(0xF3, 0xF3, 0xF3)).inner_margin(egui::Margin::symmetric(10, 3))).show(ui, |ui| {
            ui.label(RichText::new(format!("Message count  {}", self.bus.log.len())).small());
        });
        // Toolbar
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let running = self.bus.monitor_running;
            if theme::tool(ui, !running, "▷ Start").clicked() {
                self.bus.monitor_running = true;
            }
            if theme::tool(ui, running, "◻ Stop").clicked() {
                self.bus.monitor_running = false;
            }
            if theme::tool(ui, !self.bus.log.is_empty(), "🗑 Clear").clicked() {
                self.bus.log.clear();
            }
            if !self.bus.is_real() {
                ui.checkbox(&mut self.bus.simulate_traffic, RichText::new("Simulated traffic").size(12.5));
            }
            let label = if self.ws.p.show_ifaces { "⌃ Active interfaces" } else { "⌄ Active interfaces" };
            if ui.add(egui::Button::new(RichText::new(label).size(12.5)).selected(self.ws.p.show_ifaces).frame_when_inactive(false)).clicked() {
                self.ws.p.show_ifaces = !self.ws.p.show_ifaces;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ws.p.mon_filter).hint_text("🔍 Search").desired_width(180.0));
            });
        });
        theme::divider(ui);
        if self.ws.p.show_ifaces {
            let iface = self.bus.iface().clone();
            let state = self.bus.state_text();
            let n = self.bus.log.len();
            ui.push_id("ifaces", |ui| {
                TableBuilder::new(ui)
                    .striped(false)
                    .resizable(true)
                    .max_scroll_height(70.0)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::initial(320.0).clip(true))
                    .column(Column::initial(200.0).clip(true))
                    .column(Column::remainder())
                    .header(20.0, |mut h| {
                        for t in ["Interface", "State", "Activity"] {
                            h.col(|ui| {
                                ui.strong(t);
                            });
                        }
                    })
                    .body(|mut body| {
                        body.row(20.0, |mut row| {
                            row.col(|ui| {
                                ui.label(format!("{} ({})", iface.name, if self.bus.connected { self.bus.own_address.clone() } else { iface.address.clone() }));
                            });
                            row.col(|ui| {
                                ui.label(state.clone());
                            });
                            row.col(|ui| {
                                ui.label(if self.bus.monitor_running { format!("Recording · {n} telegrams") } else { "Stopped".into() });
                            });
                        });
                    });
            });
            ui.add_space(4.0);
            theme::divider(ui);
        }
        if !bus_mode {
            ui.horizontal(|ui| {
                ui.label("Send to");
                let cur = self.ws.p.send_ga.and_then(|g| proj.ga(g)).map(|g| format!("{} {}", g.fmt(proj.ga_style), g.name)).unwrap_or_else(|| "Choose a group address…".into());
                let mut gas: Vec<&GroupAddress> = proj.addresses.iter().collect();
                gas.sort_by_key(|g| g.raw());
                egui::ComboBox::from_id_salt(("sendga", self.ws.p.id)).selected_text(cur).width(280.0).show_ui(ui, |ui| {
                    for g in gas {
                        if ui.selectable_label(self.ws.p.send_ga == Some(g.id), format!("{} {}", g.fmt(proj.ga_style), g.name)).clicked() {
                            self.ws.p.send_ga = Some(g.id);
                        }
                    }
                });
                ui.label("Value");
                ui.add(egui::TextEdit::singleline(&mut self.ws.p.send_value).desired_width(60.0));
                let ok = self.ws.p.send_ga.is_some() && self.bus.connected;
                if ui.add_enabled(ok, egui::Button::new("Write")).clicked() {
                    let v = self.ws.p.send_value.trim().to_string();
                    if let Err(e) = self.bus.send_group(proj, self.ws.p.send_ga.unwrap(), Service::Write, &v, None) {
                        self.say(e);
                    }
                }
                if ui.add_enabled(ok, egui::Button::new("Read")).clicked() {
                    if let Err(e) = self.bus.send_group(proj, self.ws.p.send_ga.unwrap(), Service::Read, "", None) {
                        self.say(e);
                    }
                }
            });
            theme::divider(ui);
        }
        let f = self.ws.p.mon_filter.to_lowercase();
        let rows: Vec<usize> = self
            .bus
            .log
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                f.is_empty() || t.source.contains(&f) || t.dest.contains(&f) || t.dest_name.to_lowercase().contains(&f) || t.source_name.to_lowercase().contains(&f) || t.info.to_lowercase().contains(&f)
            })
            .map(|(i, _)| i)
            .collect();
        let log = &self.bus.log;
        let sel = self.ws.p.mon_sel;
        let mut click = None;
        let mut tb = TableBuilder::new(ui)
            .id_salt((if bus_mode { "busmon" } else { "grpmon" }, self.ws.p.id))
            .striped(true)
            .resizable(true)
            .stick_to_bottom(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::initial(44.0))
            .column(Column::initial(92.0))
            .column(Column::initial(66.0))
            .column(Column::initial(44.0))
            .column(Column::initial(84.0))
            .column(Column::initial(170.0).clip(true))
            .column(Column::initial(84.0))
            .column(Column::initial(190.0).clip(true))
            .column(Column::initial(36.0))
            .column(Column::initial(130.0).clip(true))
            .column(Column::initial(120.0).clip(true));
        let headers: &[&str] = if bus_mode {
            tb = tb.column(Column::initial(110.0).clip(true)).column(Column::remainder().clip(true));
            &["#", "Time", "Service", "Prio", "Source address", "Source name", "Destination address", "Destination name", "Hop", "Type", "DPT", "Info", "Raw data"]
        } else {
            tb = tb.column(Column::remainder().clip(true));
            &["#", "Time", "Service", "Prio", "Source address", "Source name", "Destination address", "Destination name", "Hop", "Type", "DPT", "Info"]
        };
        tb.header(22.0, |mut h| {
            for t in headers {
                h.col(|ui| {
                    ui.strong(*t);
                });
            }
        })
        .body(|body| {
            body.rows(20.0, rows.len(), |mut row| {
                let t = &log[rows[row.index()]];
                row.set_selected(sel == Some(t.seq));
                let svc = if t.from_bus { "from bus" } else { "to bus" };
                let mut cells: Vec<String> = vec![
                    t.seq.to_string(),
                    t.time.clone(),
                    svc.into(),
                    t.prio.into(),
                    t.source.clone(),
                    t.source_name.clone(),
                    t.dest.clone(),
                    t.dest_name.clone(),
                    t.rout.to_string(),
                    t.service.into(),
                    t.dpt.clone(),
                    t.info.clone(),
                ];
                if bus_mode {
                    cells.push(t.raw.clone());
                }
                for c in cells {
                    row.col(|ui| {
                        ui.label(RichText::new(c).size(12.5));
                    });
                }
                if row.response().clicked() {
                    click = Some(t.seq);
                }
            });
        });
        if let Some(s) = click {
            self.ws.p.mon_sel = Some(s);
        }
    }

    pub(crate) fn jobs_view(&mut self, ui: &mut Ui, proj: &mut Project) {
        ui.horizontal(|ui| {
            if ui.button("⬇ Download all that need it").clicked() {
                self.download_all(proj);
            }
            if ui.button("Clear finished").clicked() {
                self.bus.jobs.retain(|j| !matches!(j.state, JobState::Done | JobState::Failed));
            }
            if ui.button("Cancel waiting").clicked() {
                self.bus.jobs.retain(|j| j.state != JobState::Waiting);
            }
        });
        let pid = proj.id;
        let jobs: Vec<usize> = self.bus.jobs.iter().enumerate().filter(|(_, j)| j.project == pid).map(|(i, _)| i).collect();
        if jobs.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("No operations yet. Select a device and choose Download or Program address.").color(theme::MUTED));
            return;
        }
        let mut press = None;
        TableBuilder::new(ui)
            .id_salt("jobs")
            .striped(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::initial(70.0))
            .column(Column::initial(220.0).clip(true))
            .column(Column::initial(190.0))
            .column(Column::initial(200.0))
            .column(Column::remainder())
            .header(22.0, |mut h| {
                for t in ["Address", "Device", "Operation", "Progress", "Status"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for i in jobs {
                    let j = &self.bus.jobs[i];
                    body.row(26.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&j.address);
                        });
                        row.col(|ui| {
                            ui.label(&j.name);
                        });
                        row.col(|ui| {
                            ui.label(j.kind.label());
                        });
                        row.col(|ui| {
                            ui.add(egui::ProgressBar::new(j.progress).desired_width(180.0).show_percentage());
                        });
                        row.col(|ui| {
                            let c = match j.state {
                                JobState::Done => theme::OK,
                                JobState::Failed => theme::BAD,
                                JobState::PressButton => theme::WARN,
                                _ => ui.visuals().text_color(),
                            };
                            ui.label(RichText::new(&j.message).color(c));
                            if j.state == JobState::PressButton && !j.real && ui.small_button("Button pressed").clicked() {
                                press = Some(i);
                            }
                        });
                    });
                }
            });
        if let Some(i) = press {
            self.bus.jobs[i].timer = 0.0;
        }
    }

    pub(crate) fn download_all(&mut self, proj: &Project) {
        if self.bus.is_real() {
            self.say("On a real bus, download devices one at a time (select a device and choose Program)");
            return;
        }
        let ids: Vec<u32> = proj.devices.iter().filter(|d| d.needs_download()).map(|d| d.id).collect();
        if ids.is_empty() {
            self.say("All devices are up to date");
            return;
        }
        let mut n = 0;
        for d in ids {
            let kind = if proj.device(d).map(|x| x.adr_ok).unwrap_or(false) { JobKind::Partial } else { JobKind::Full };
            if self.bus.enqueue(proj, d, kind).is_ok() {
                n += 1;
            }
        }
        self.say(format!("Queued {n} download(s)"));
        self.open_dock(Dock::Jobs);
    }
}