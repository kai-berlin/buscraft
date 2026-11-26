//! Parameter editor for devices whose application comes from an imported product file.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, Margin, RichText, Sense, Ui};

use crate::app::*;
use crate::knxapp::{Item, PType, Visible};
use crate::model::*;
use crate::theme;

fn values_hash(dev: &Device) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    dev.knx_values.hash(&mut h);
    dev.knx_app.hash(&mut h);
    h.finish()
}

pub(crate) fn page_item(ui: &mut Ui, text: &str, depth: usize, header: bool, selected: bool) -> egui::Response {
    let w = ui.available_width();
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0.0, Color32::WHITE);
        p.vline(rect.left() + 1.0, rect.y_range(), egui::Stroke::new(2.0, theme::ACCENT));
    } else if r.hovered() {
        p.rect_filled(rect, 0.0, Color32::from_rgb(0xEC, 0xF4, 0xFB));
    }
    p.hline(rect.left()..=rect.right(), rect.bottom(), egui::Stroke::new(1.0, theme::LINE));
    let x = rect.left() + 12.0 + depth as f32 * 14.0;
    let col = if selected { theme::ACCENT } else { Color32::from_rgb(0x1F, 0x1F, 0x1F) };
    let font = if header { egui::FontId::proportional(13.0) } else { egui::FontId::proportional(12.5) };
    if header {
        p.text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, "–", font.clone(), theme::MUTED);
    }
    let galley = p.layout(text.to_string(), font, col, (rect.right() - x - 22.0).max(40.0));
    let pos = egui::pos2(x + if header { 14.0 } else { 0.0 }, rect.center().y - galley.size().y.min(28.0) / 2.0);
    p.with_clip_rect(rect).galley(pos, galley, col);
    r
}

impl App {
    fn knx_visible(&mut self, dev: &Device) -> Option<(Arc<crate::knxapp::KnxApp>, Arc<Visible>)> {
        let app = self.library.app(dev.knx_app.as_ref()?)?;
        let h = values_hash(dev);
        if let Some((d, hh, v)) = &self.ws.knx_cache {
            if *d == dev.id && *hh == h {
                return Some((app, v.clone()));
            }
        }
        let v = Arc::new(app.visible(&dev.knx_values));
        self.ws.knx_cache = Some((dev.id, h, v.clone()));
        Some((app, v))
    }

    pub(crate) fn knx_parameters(&mut self, ui: &mut Ui, proj: &mut Project, id: u32) {
        let dev = proj.device(id).unwrap().clone();
        let Some((app, vis)) = self.knx_visible(&dev) else {
            ui.add_space(8.0);
            ui.label(RichText::new("The product data for this device is not in the library.").strong());
            ui.label(RichText::new(format!("Import the manufacturer's product file (.knxprod) or the project again to edit its parameters. Application: {}", dev.app_program)).color(theme::MUTED));
            return;
        };
        if vis.pages.is_empty() {
            ui.label(RichText::new("This application has no parameters.").color(theme::MUTED));
            return;
        }
        // Current page
        let key = |i: usize, t: &str| format!("{i}|{t}");
        let mut cur = vis.pages.iter().enumerate().position(|(i, p)| !p.header && key(i, &p.text) == self.ws.p.param_page);
        if cur.is_none() {
            let wanted = self.ws.p.param_page.split_once('|').map(|x| x.1).unwrap_or("").to_string();
            cur = vis.pages.iter().position(|p| !p.header && p.text == wanted);
        }
        let cur = cur.or_else(|| vis.pages.iter().position(|p| !p.header)).unwrap_or(0);
        let mut changes: Vec<(String, String)> = vec![];

        egui_extras::StripBuilder::new(ui).size(egui_extras::Size::exact(230.0)).size(egui_extras::Size::remainder()).horizontal(|mut strip| {
            strip.cell(|ui| {
                egui::Frame::new().fill(Color32::from_rgb(0xF5, 0xF5, 0xF5)).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    egui::ScrollArea::vertical().id_salt("knxpages").show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, p) in vis.pages.iter().enumerate() {
                            if page_item(ui, &p.text, p.depth, p.header, i == cur).clicked() && !p.header {
                                self.ws.p.param_page = key(i, &p.text);
                            }
                        }
                    });
                });
            });
            strip.cell(|ui| {
                egui::Frame::new().inner_margin(Margin { left: 14, right: 8, top: 6, bottom: 6 }).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let page = &vis.pages[cur];
                    // Breadcrumb: device > parent headers > page
                    let mut crumbs = vec![format!("{} {}", dev.address(), dev.name)];
                    let mut depth = page.depth;
                    for p in vis.pages[..cur].iter().rev() {
                        if p.depth < depth {
                            crumbs.insert(1, p.text.clone());
                            depth = p.depth;
                        }
                    }
                    crumbs.push(page.text.clone());
                    ui.label(RichText::new(crumbs.join(" > ")).strong().size(13.5));
                    for n in &app.notes {
                        ui.label(RichText::new(format!("ⓘ {n}")).small().color(theme::MUTED));
                    }
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("knxparams").show(ui, |ui| {
                        egui::Grid::new("knxgrid").num_columns(2).spacing([28.0, 10.0]).min_col_width(200.0).show(ui, |ui| {
                            for item in &page.items {
                                match item {
                                    Item::Sep(t) => {
                                        ui.end_row();
                                        if t.is_empty() {
                                            ui.separator();
                                        } else {
                                            ui.label(RichText::new(t).color(theme::MUTED));
                                        }
                                        ui.label("");
                                        ui.end_row();
                                    }
                                    Item::Heading(t) => {
                                        ui.label(RichText::new(t).strong());
                                        ui.label("");
                                        ui.end_row();
                                    }
                                    Item::Param(pid) => {
                                        let Some(def) = app.params.get(pid) else { continue };
                                        let value = app.value_of(&dev.knx_values, pid);
                                        ui.add(egui::Label::new(&def.text).wrap());
                                        let enabled = def.access >= 2;
                                        let ptype = app.types.get(&def.ptype).cloned().unwrap_or(PType::Text);
                                        ui.add_enabled_ui(enabled, |ui| {
                                            if let Some(v) = param_widget(ui, pid, &ptype, &value, &def.unit) {
                                                changes.push((pid.clone(), v));
                                            }
                                        });
                                        ui.end_row();
                                    }
                                }
                            }
                        });
                    });
                });
            });
        });

        if !changes.is_empty() {
            let d = proj.device_mut(id).unwrap();
            for (pid, v) in changes {
                let default = app.params.get(&pid).map(|p| p.value.clone()).unwrap_or_default();
                if v == default {
                    d.knx_values.remove(&pid);
                } else {
                    d.knx_values.insert(pid, v);
                }
            }
            d.par_ok = false;
            app.sync_objects(d);
            self.mark_typing();
        }
    }
}

/// Draw the input for one parameter; returns the new value when changed.
fn param_widget(ui: &mut Ui, pid: &str, ptype: &PType, value: &str, unit: &str) -> Option<String> {
    let mut out = None;
    match ptype {
        PType::Enum(opts) if opts.len() <= 2 => {
            ui.horizontal(|ui| {
                for (v, t) in opts {
                    if ui.radio(value == v, t.as_str()).clicked() && value != v {
                        out = Some(v.clone());
                    }
                }
            });
        }
        PType::Enum(opts) => {
            let cur = opts.iter().find(|(v, _)| v == value).map(|(_, t)| t.clone()).unwrap_or_else(|| value.to_string());
            egui::ComboBox::from_id_salt(("knxp", pid)).selected_text(cur).width(220.0).show_ui(ui, |ui| {
                for (v, t) in opts {
                    if ui.selectable_label(value == v, t.as_str()).clicked() && value != v {
                        out = Some(v.clone());
                    }
                }
            });
        }
        PType::Int { min, max } => {
            let mut v: i64 = value.trim().parse::<f64>().map(|f| f as i64).unwrap_or(*min as i64);
            ui.horizontal(|ui| {
                if ui.add(egui::DragValue::new(&mut v).range(*min as i64..=*max as i64)).changed() {
                    out = Some(v.to_string());
                }
                ui.label(RichText::new(format!("{unit}  ({}…{})", *min as i64, *max as i64)).color(theme::MUTED));
            });
        }
        PType::Float { min, max } => {
            let mut v: f64 = value.trim().replace(',', ".").parse().unwrap_or(*min);
            ui.horizontal(|ui| {
                if ui.add(egui::DragValue::new(&mut v).range(*min..=*max).speed(0.1).max_decimals(2)).changed() {
                    out = Some(format!("{v}"));
                }
                ui.label(RichText::new(unit).color(theme::MUTED));
            });
        }
        PType::Text => {
            let mut s = value.to_string();
            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(220.0)).changed() {
                out = Some(s);
            }
        }
        PType::None => {
            ui.label("");
        }
    }
    out
}
