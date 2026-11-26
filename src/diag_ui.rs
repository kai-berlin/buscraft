//! The Diagnostics pages of a Monitors panel: Device Info, Programming Mode,
//! Individual Address Check and Line Scan.

use egui::{Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::*;
use crate::diag::*;
use crate::knx::{self, fmt_ia};
use crate::model::*;
use crate::theme;

/// The project device at an individual address (if any).
fn device_at(proj: &Project, ia: u16) -> Option<&Device> {
    proj.devices.iter().find(|d| d.has_app() && knx::parse_ia(&d.address()) == Some(ia))
}

impl App {
    pub(crate) fn diag_page(&mut self, ui: &mut Ui, proj: &mut Project, page: DiagPage) {
        if self.bus.diag_busy() {
            ui.ctx().request_repaint();
        }
        match page {
            DiagPage::Info => self.diag_info(ui, proj),
            DiagPage::Prog => self.diag_prog(ui, proj),
            DiagPage::Check => self.diag_check(ui, proj),
            DiagPage::Scan => self.diag_scan(ui, proj),
        }
    }

    fn start(&mut self, kind: DiagKind, target: &str) {
        if let Err(e) = self.bus.start_diag(kind, target) {
            self.say(e);
        }
    }

    /// Address field with a picker of the project's devices. Returns true on Enter.
    fn address_field(&mut self, ui: &mut Ui, proj: &Project) -> bool {
        ui.label("Individual address");
        let r = ui.add(egui::TextEdit::singleline(&mut self.ws.p.diag_addr).hint_text("1.1.1").desired_width(80.0));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut devs: Vec<&Device> = proj.devices.iter().filter(|d| d.has_app()).collect();
        devs.sort_by_key(|d| (d.area, d.line, d.number));
        ui.menu_button(RichText::new("▾").size(12.5), |ui| {
            ui.label(RichText::new("Devices in the project").small().color(theme::MUTED));
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for d in devs {
                    if ui.button(format!("{}  {}", d.address(), d.name)).clicked() {
                        self.ws.p.diag_addr = d.address();
                        ui.close();
                    }
                }
            });
        })
        .response
        .on_hover_text("Pick a device of the project");
        enter
    }

    /// Status line under the toolbar: progress and the latest message.
    fn diag_status(ui: &mut Ui, t: Option<&DiagTask>, idle: &str) {
        ui.add_space(4.0);
        match t {
            None => {
                ui.label(RichText::new(idle).color(theme::MUTED));
            }
            Some(t) => {
                ui.horizontal(|ui| {
                    let (icon, c) = match t.state {
                        DiagState::Running => ("⟳", ui.visuals().text_color()),
                        DiagState::Done => ("✔", theme::OK),
                        DiagState::Failed => ("✖", theme::BAD),
                        DiagState::Stopped => ("◻", theme::WARN),
                    };
                    ui.label(RichText::new(icon).color(c).strong());
                    ui.label(RichText::new(&t.message).color(c));
                    if t.running() {
                        ui.add(egui::ProgressBar::new(t.progress).desired_width(160.0).show_percentage());
                    } else {
                        ui.label(RichText::new(format!("· {}", t.started)).small().color(theme::MUTED));
                    }
                });
            }
        }
        ui.add_space(4.0);
        theme::divider(ui);
    }

    fn sim_hint(&self, ui: &mut Ui) {
        if !self.bus.is_real() {
            ui.label(
                RichText::new("Simulated bus: devices of this project answer once their individual address has been programmed. Connect a KNX IP interface on the Bus tab of the start page to work with a real installation.")
                    .small()
                    .color(theme::MUTED),
            );
        }
    }

    // ------------------------------------------------------------ Device Info

    fn diag_info(&mut self, ui: &mut Ui, proj: &mut Project) {
        let busy = self.bus.diag_busy();
        let mut go = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            go |= self.address_field(ui, proj);
            go |= theme::tool(ui, !busy, "▷ Read").on_hover_text("Read the device's identity and load states from the bus").clicked();
        });
        theme::divider(ui);
        let t = self.bus.diag_latest(DiagPage::Info).cloned();
        Self::diag_status(ui, t.as_ref(), "Enter an individual address and choose Read.");
        if go && !busy {
            let a = self.ws.p.diag_addr.clone();
            self.start(DiagKind::DeviceInfo, &a);
        }
        let Some(t) = t else {
            self.sim_hint(ui);
            return;
        };
        // In the project
        if let Some(ia) = knx::parse_ia(&t.target) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("In the project:").color(theme::MUTED));
                match device_at(proj, ia) {
                    Some(d) => {
                        ui.label(RichText::new(format!("{} {}", d.address(), d.name)).strong());
                        ui.label(RichText::new(format!("{} · {}", d.manufacturer, d.order_no)).color(theme::MUTED));
                    }
                    None => {
                        ui.label(RichText::new("no device with this address").color(theme::WARN));
                    }
                }
            });
            ui.add_space(4.0);
        }
        if t.info.is_empty() {
            return;
        }
        TableBuilder::new(ui)
            .id_salt(("devinfo", self.ws.p.id))
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::initial(220.0))
            .column(Column::remainder().clip(true))
            .header(22.0, |mut h| {
                for c in ["Property", "Value"] {
                    h.col(|ui| {
                        ui.strong(c);
                    });
                }
            })
            .body(|mut body| {
                for (k, v) in &t.info {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.label(k);
                        });
                        row.col(|ui| {
                            let c = match v.as_str() {
                                "Loaded" | "On" => theme::OK,
                                "Unloaded" | "Error" | "not available" => theme::WARN,
                                _ => ui.visuals().text_color(),
                            };
                            ui.label(RichText::new(v).color(c));
                        });
                    });
                }
            });
    }

    // ------------------------------------------------------------ Programming Mode

    fn diag_prog(&mut self, ui: &mut Ui, proj: &mut Project) {
        let busy = self.bus.diag_busy();
        let now = ui.input(|i| i.time);
        let mut action: Option<(DiagKind, String)> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let auto = self.ws.p.diag_auto;
            if theme::tool(ui, !auto, "▷ Start").on_hover_text("Keep looking for devices in programming mode").clicked() {
                self.ws.p.diag_auto = true;
                self.ws.p.diag_next = 0.0;
            }
            if theme::tool(ui, auto, "◻ Stop").clicked() {
                self.ws.p.diag_auto = false;
            }
            if theme::tool(ui, !busy && !auto, "🔍 Search once").clicked() {
                action = Some((DiagKind::ProgScan, String::new()));
            }
            ui.separator();
            let enter = self.address_field(ui, proj);
            if theme::tool(ui, !busy, "Switch on").on_hover_text("Switch the device's programming mode on, as if its button were pressed").clicked() || enter {
                action = Some((DiagKind::SetProg(true), self.ws.p.diag_addr.clone()));
            }
            if theme::tool(ui, !busy, "Switch off").clicked() {
                action = Some((DiagKind::SetProg(false), self.ws.p.diag_addr.clone()));
            }
        });
        theme::divider(ui);
        // Repeated search
        if self.ws.p.diag_auto {
            if !busy && now >= self.ws.p.diag_next {
                action = action.or(Some((DiagKind::ProgScan, String::new())));
                self.ws.p.diag_next = now + PROG_SCAN_REPEAT_S as f64 + PROG_SCAN_WAIT_MS as f64 / 1000.0;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        let t = self.bus.diag_latest(DiagPage::Prog).cloned();
        Self::diag_status(ui, t.as_ref(), "Choose Start to look for devices whose programming button is pressed.");
        // Devices currently in programming mode: from the latest search.
        let list = self.bus.diags.iter().rev().find(|t| t.kind == DiagKind::ProgScan && t.state == DiagState::Done).map(|t| (t.hits.clone(), t.started.clone()));
        let mut off = None;
        let mut info = None;
        match list {
            None => {
                self.sim_hint(ui);
            }
            Some((hits, when)) if hits.is_empty() => {
                ui.add_space(6.0);
                ui.label(RichText::new(format!("No device was in programming mode at {when}.")).color(theme::MUTED));
                self.sim_hint(ui);
            }
            Some((hits, when)) => {
                ui.label(RichText::new(format!("In programming mode at {when}:")).small().color(theme::MUTED));
                TableBuilder::new(ui)
                    .id_salt(("progmode", self.ws.p.id))
                    .striped(true)
                    .sense(Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::initial(100.0))
                    .column(Column::initial(300.0).clip(true))
                    .column(Column::initial(260.0).clip(true))
                    .column(Column::remainder())
                    .header(22.0, |mut h| {
                        for c in ["Address", "Device in the project", "Type", ""] {
                            h.col(|ui| {
                                ui.strong(c);
                            });
                        }
                    })
                    .body(|mut body| {
                        for h in &hits {
                            body.row(24.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(fmt_ia(h.ia));
                                });
                                row.col(|ui| {
                                    match device_at(proj, h.ia) {
                                        Some(d) => ui.label(&d.name),
                                        None if h.ia == FACTORY_IA => ui.label(RichText::new("factory address – not programmed yet").color(theme::WARN)),
                                        None => ui.label(RichText::new("not in the project").color(theme::WARN)),
                                    };
                                });
                                row.col(|ui| {
                                    ui.label(h.mask.map(mask_text).unwrap_or_else(|| "–".into()));
                                });
                                row.col(|ui| {
                                    if ui.small_button("Switch off").clicked() {
                                        off = Some(h.ia);
                                    }
                                });
                                if row.response().double_clicked() {
                                    info = Some(h.ia);
                                }
                            });
                        }
                    });
            }
        }
        if let Some(ia) = off {
            action = Some((DiagKind::SetProg(false), fmt_ia(ia)));
        }
        if let Some((kind, target)) = action {
            if !busy {
                self.start(kind, &target);
            }
        }
        if let Some(ia) = info {
            self.open_info(ia);
        }
    }

    /// Show Device Info for an address and read it.
    fn open_info(&mut self, ia: u16) {
        self.ws.p.diag_addr = fmt_ia(ia);
        self.ws.p.mon_view = MonView::DeviceInfo;
        if !self.bus.diag_busy() {
            self.start(DiagKind::DeviceInfo, &fmt_ia(ia));
        }
    }

    // ------------------------------------------------------------ Individual Address Check

    fn diag_check(&mut self, ui: &mut Ui, proj: &mut Project) {
        let busy = self.bus.diag_busy();
        let mut go = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            go |= self.address_field(ui, proj);
            go |= theme::tool(ui, !busy, "▷ Check").on_hover_text("Find out whether a device answers at this address").clicked();
        });
        theme::divider(ui);
        if go && !busy {
            let a = self.ws.p.diag_addr.clone();
            self.start(DiagKind::Check, &a);
        }
        let t = self.bus.diag_latest(DiagPage::Check).cloned();
        Self::diag_status(ui, t.as_ref(), "Enter an individual address and choose Check: before giving a device an address, make sure no other device uses it.");
        let Some(t) = t else {
            self.sim_hint(ui);
            return;
        };
        if t.running() {
            return;
        }
        let Some(ia) = knx::parse_ia(&t.target) else { return };
        ui.add_space(6.0);
        egui::Grid::new(("check", self.ws.p.id)).num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label(RichText::new("Address").color(theme::MUTED));
            ui.label(RichText::new(&t.target).strong());
            ui.end_row();
            ui.label(RichText::new("On the bus").color(theme::MUTED));
            match t.hits.len() {
                0 if t.state == DiagState::Done => ui.label(RichText::new("No device answers – the address is free").color(theme::OK)),
                0 => ui.label(RichText::new("Not checked").color(theme::MUTED)),
                1 => ui.label(RichText::new("A device answers").color(theme::WARN)),
                n => ui.label(RichText::new(format!("{n} devices answer – the address is used more than once")).color(theme::BAD)),
            };
            ui.end_row();
            for (k, v) in &t.info {
                ui.label(RichText::new(k).color(theme::MUTED));
                ui.label(v);
                ui.end_row();
            }
            ui.label(RichText::new("In the project").color(theme::MUTED));
            match device_at(proj, ia) {
                Some(d) => ui.label(format!("{} {} ({})", d.address(), d.name, if d.adr_ok { "address programmed" } else { "address not programmed yet" })),
                None => ui.label(RichText::new("no device with this address").color(theme::MUTED)),
            };
            ui.end_row();
        });
        if !t.hits.is_empty() {
            ui.add_space(8.0);
            if ui.button("Show device info").clicked() {
                self.open_info(ia);
            }
        }
    }

    // ------------------------------------------------------------ Line Scan

    fn diag_scan(&mut self, ui: &mut Ui, proj: &mut Project) {
        let busy = self.bus.diag_busy();
        let running_scan = self.bus.diags.iter().any(|t| t.kind == DiagKind::LineScan && t.running());
        if self.ws.p.diag_line.is_empty() {
            if let Some((a, l)) = proj.areas.iter().find_map(|a| a.lines.iter().find(|l| l.number != 0).map(|l| (a.number, l.number))) {
                self.ws.p.diag_line = format!("{a}.{l}");
            }
        }
        let mut go = false;
        let mut stop = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label("Line");
            let r = ui.add(egui::TextEdit::singleline(&mut self.ws.p.diag_line).hint_text("1.1").desired_width(56.0));
            go |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.menu_button(RichText::new("▾").size(12.5), |ui| {
                ui.label(RichText::new("Lines in the project").small().color(theme::MUTED));
                for a in &proj.areas {
                    for l in &a.lines {
                        if ui.button(format!("{}.{}  {}", a.number, l.number, l.name)).clicked() {
                            self.ws.p.diag_line = format!("{}.{}", a.number, l.number);
                            ui.close();
                        }
                    }
                }
            });
            go |= theme::tool(ui, !busy, "▷ Start").on_hover_text("Ask every address 0–255 of the line whether a device is there").clicked();
            stop = theme::tool(ui, running_scan, "◻ Stop").clicked();
        });
        theme::divider(ui);
        if go && !busy {
            let l = self.ws.p.diag_line.clone();
            self.start(DiagKind::LineScan, &l);
        }
        if stop {
            self.bus.stop_diag();
        }
        let t = self.bus.diag_latest(DiagPage::Scan).cloned();
        Self::diag_status(ui, t.as_ref(), "Choose a line and Start to find every device that answers on it.");
        let Some(t) = t else {
            self.sim_hint(ui);
            return;
        };
        let Some((area, line)) = parse_line(&t.target) else { return };
        // Rows: devices found, then project devices of the line that did not answer (when finished).
        struct Row {
            ia: u16,
            mask: Option<u16>,
            name: String,
            status: (&'static str, egui::Color32),
        }
        let mut rows: Vec<Row> = t
            .hits
            .iter()
            .map(|h| {
                let d = device_at(proj, h.ia);
                Row {
                    ia: h.ia,
                    mask: h.mask,
                    name: d.map(|d| d.name.clone()).unwrap_or_default(),
                    status: if d.is_some() { ("Found", theme::OK) } else { ("Not in the project", theme::WARN) },
                }
            })
            .collect();
        if t.state == DiagState::Done {
            for d in proj.devices.iter().filter(|d| d.has_app() && d.area == area && d.line == line) {
                let Some(ia) = knx::parse_ia(&d.address()) else { continue };
                if !t.hits.iter().any(|h| h.ia == ia) {
                    rows.push(Row { ia, mask: None, name: d.name.clone(), status: ("Did not answer", theme::BAD) });
                }
            }
        }
        rows.sort_by_key(|r| r.ia);
        let found = t.hits.len();
        let missing = rows.iter().filter(|r| r.status.0 == "Did not answer").count();
        let unknown = rows.iter().filter(|r| r.status.0 == "Not in the project").count();
        ui.label(RichText::new(format!("{found} answered · {unknown} not in the project{}", if t.state == DiagState::Done { format!(" · {missing} from the project did not answer") } else { String::new() })).small().color(theme::MUTED));
        let mut info = None;
        TableBuilder::new(ui)
            .id_salt(("linescan", self.ws.p.id))
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .stick_to_bottom(t.running())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::initial(90.0))
            .column(Column::initial(280.0).clip(true))
            .column(Column::initial(280.0).clip(true))
            .column(Column::remainder())
            .header(22.0, |mut h| {
                for c in ["Address", "Device in the project", "Type", "Result"] {
                    h.col(|ui| {
                        ui.strong(c);
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, rows.len(), |mut row| {
                    let r = &rows[row.index()];
                    row.col(|ui| {
                        ui.label(fmt_ia(r.ia));
                    });
                    row.col(|ui| {
                        ui.label(&r.name);
                    });
                    row.col(|ui| {
                        ui.label(r.mask.map(mask_text).unwrap_or_else(|| "–".into()));
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(r.status.0).color(r.status.1));
                    });
                    if row.response().double_clicked() && r.status.0 != "Did not answer" {
                        info = Some(r.ia);
                    }
                });
            });
        if let Some(ia) = info {
            self.open_info(ia);
        }
    }
}
