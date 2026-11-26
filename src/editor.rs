//! Device editor: group objects with links and flags, and parameter pages.

use egui::{Align, Color32, Layout, Margin, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::*;
use crate::bus::JobKind;
use crate::model::*;
use crate::theme;
use crate::workspace::new_ga;

impl App {
    pub(crate) fn device_editor(&mut self, ui: &mut Ui, proj: &mut Project, id: u32) {
        let (title, sub, nobj) = {
            let d = proj.device(id).unwrap();
            (format!("{} {}", d.address(), d.name), format!("{} · {} · {}", d.product, d.manufacturer, d.status_text()), d.objects.len())
        };
        ui.horizontal(|ui| {
            if ui.button("← Back").clicked() {
                self.ws.p.open_device = None;
            }
            ui.add(egui::Label::new(RichText::new(&title).size(17.0).strong()).truncate());
            ui.add(egui::Label::new(RichText::new(&sub).color(theme::MUTED)).truncate());
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if theme::tab(ui, self.ws.p.dev_tab == DevTab::GroupObjects, &format!("▦ Group Objects  {nobj}")).clicked() {
                self.ws.p.dev_tab = DevTab::GroupObjects;
            }
            if theme::tab(ui, self.ws.p.dev_tab == DevTab::Parameters, "⚙ Parameters").clicked() {
                self.ws.p.dev_tab = DevTab::Parameters;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("⬇ Download").clicked() {
                    let kind = if proj.device(id).map(|x| x.adr_ok).unwrap_or(false) { JobKind::Partial } else { JobKind::Full };
                    self.enqueue(proj, id, kind);
                }
            });
        });
        ui.separator();
        match self.ws.p.dev_tab {
            DevTab::GroupObjects => self.group_objects(ui, proj, id),
            DevTab::Parameters => self.parameters(ui, proj, id),
        }
    }

    fn group_objects(&mut self, ui: &mut Ui, proj: &mut Project, id: u32) {
        let link_texts: Vec<String> = proj.device(id).unwrap().objects.iter().map(|o| o.links.iter().map(|l| proj.ga_text(*l)).collect::<Vec<_>>().join(", ")).collect();
        if link_texts.is_empty() {
            ui.label(RichText::new("This device has no group objects.").color(theme::MUTED));
            return;
        }
        let sel_obj = self.ws.p.sel_object;
        let mut click = None;
        let mut flag_changes: Vec<(usize, Flags)> = vec![];
        {
            let d = proj.device(id).unwrap();
            let table_h = (ui.available_height() - 190.0).max(140.0);
            theme::card_frame(ui).inner_margin(Margin::same(4)).show(ui, |ui| {
                egui::ScrollArea::horizontal().id_salt("objects_h").show(ui, |ui| {
                TableBuilder::new(ui)
                    .id_salt("objects")
                    .striped(true)
                    .resizable(true)
                    .sense(Sense::click_and_drag())
                    .max_scroll_height(table_h)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .column(Column::initial(56.0))
                    .column(Column::initial(90.0).clip(true))
                    .column(Column::initial(150.0).clip(true))
                    .column(Column::initial(130.0).clip(true))
                    .column(Column::initial(52.0))
                    .columns(Column::exact(24.0), 5)
                    .column(Column::initial(170.0).clip(true))
                    .column(Column::initial(60.0))
                    .header(24.0, |mut h| {
                        for t in ["Number", "Name", "Object function", "Group addresses", "Length", "C", "R", "W", "T", "U", "Data type", "Priority"] {
                            h.col(|ui| {
                                ui.strong(t);
                            });
                        }
                    })
                    .body(|mut body| {
                        for (i, o) in d.objects.iter().enumerate() {
                            body.row(24.0, |mut row| {
                                row.set_selected(sel_obj == Some(i));
                                row.col(|ui| {
                                    ui.label(format!("■ {}", o.number));
                                });
                                row.col(|ui| {
                                    ui.label(&o.name);
                                });
                                row.col(|ui| {
                                    ui.label(&o.function);
                                });
                                row.col(|ui| {
                                    if link_texts[i].is_empty() {
                                        ui.label(RichText::new("—").color(theme::MUTED));
                                    } else {
                                        ui.label(RichText::new(&link_texts[i]).monospace());
                                    }
                                });
                                row.col(|ui| {
                                    ui.label(if o.size.is_empty() { dpt_size(&o.dpt).to_string() } else { o.size.clone() });
                                });
                                let mut f = o.flags;
                                let mut changed = false;
                                for v in [&mut f.c, &mut f.r, &mut f.w, &mut f.t, &mut f.u] {
                                    row.col(|ui| {
                                        changed |= ui.checkbox(v, "").changed();
                                    });
                                }
                                if changed {
                                    flag_changes.push((i, f));
                                }
                                row.col(|ui| {
                                    ui.label(&o.dpt);
                                });
                                row.col(|ui| {
                                    ui.label(&o.priority);
                                });
                                if row.response().clicked() {
                                    click = Some(i);
                                }
                                // Drag onto a group address (in any panel) to link – or drop one here
                                let label = format!("{} – object {} {} {}", d.address(), o.number, o.name, o.function);
                                self.dnd_link(&row.response(), crate::panes::DragItem::Object { dev: id, idx: i, label });
                            });
                        }
                    });
                });
            });
        }
        if !flag_changes.is_empty() {
            let d = proj.device_mut(id).unwrap();
            for (i, f) in flag_changes {
                d.objects[i].flags = f;
            }
            d.cfg_ok = false;
            self.mark();
        }
        if let Some(i) = click {
            self.ws.p.sel_object = Some(i);
        }

        ui.add_space(8.0);
        let Some(oi) = self.ws.p.sel_object.filter(|i| *i < link_texts.len()) else {
            ui.label(RichText::new("Select a group object to link it with group addresses.").color(theme::MUTED));
            return;
        };
        let (oname, odpt, links) = {
            let o = &proj.device(id).unwrap().objects[oi];
            (format!("{} {} – {}", o.number, o.name, o.function), o.dpt.clone(), o.links.clone())
        };
        let mut unlink = None;
        let mut make_first = None;
        let mut link = None;
        let mut create = false;
        theme::card_frame(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Links of object {oname}")).strong());
                ui.label(RichText::new(&odpt).color(theme::MUTED));
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if links.is_empty() {
                    ui.label(RichText::new("Not linked").color(theme::MUTED));
                }
                for (k, l) in links.iter().enumerate() {
                    let name = proj.ga(*l).map(|g| g.name.clone()).unwrap_or_default();
                    let fill = if k == 0 { theme::ACCENT_SOFT } else { ui.visuals().faint_bg_color };
                    egui::Frame::new().fill(fill).corner_radius(egui::CornerRadius::same(10)).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let c = if k == 0 { Color32::from_rgb(0x0E, 0x4A, 0x82) } else { ui.visuals().text_color() };
                            ui.label(RichText::new(format!("{} {}", proj.ga_text(*l), name)).color(c));
                            if k == 0 {
                                ui.label(RichText::new("sending").small().color(theme::MUTED));
                            } else if ui.small_button("Set sending").on_hover_text("Make this the sending address").clicked() {
                                make_first = Some(k);
                            }
                            if ui.small_button("✕").on_hover_text("Unlink").clicked() {
                                unlink = Some(*l);
                            }
                        });
                    });
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Link with");
                let mut gas: Vec<&GroupAddress> = proj.addresses.iter().filter(|g| !links.contains(&g.id)).collect();
                gas.sort_by_key(|g| (g.dpt != odpt && !g.dpt.is_empty(), g.raw()));
                let cur = self.ws.p.link_pick.and_then(|g| proj.ga(g)).map(|g| format!("{} {}", g.fmt(proj.ga_style), g.name)).unwrap_or_else(|| "Choose a group address…".into());
                egui::ComboBox::from_id_salt("linkpick").selected_text(cur).width(320.0).show_ui(ui, |ui| {
                    for g in &gas {
                        let ok = g.dpt == odpt || g.dpt.is_empty();
                        let t = RichText::new(format!("{} {} {}", if ok { "✔" } else { "   " }, g.fmt(proj.ga_style), g.name));
                        let t = if ok { t } else { t.color(theme::MUTED) };
                        if ui.selectable_label(self.ws.p.link_pick == Some(g.id), t).clicked() {
                            self.ws.p.link_pick = Some(g.id);
                        }
                    }
                });
                let pick = self.ws.p.link_pick.filter(|g| !links.contains(g) && proj.ga(*g).is_some());
                if ui.add_enabled(pick.is_some(), egui::Button::new("Link")).clicked() {
                    link = pick;
                }
                if ui.button("+ New address and link").on_hover_text("Create a new group address for this object and link it").clicked() {
                    create = true;
                }
            });
        });
        let mut changed = false;
        if let Some(g) = link {
            proj.device_mut(id).unwrap().objects[oi].links.push(g);
            self.ws.p.link_pick = None;
            changed = true;
        }
        if create {
            if proj.main_groups.is_empty() {
                proj.main_groups.push(MainGroup { number: 0, name: "New main group".into(), middles: vec![] });
            }
            if proj.main_groups[0].middles.is_empty() {
                proj.main_groups[0].middles.push(MiddleGroup { number: 0, name: "New middle group".into() });
            }
            let (m, mi) = (proj.main_groups[0].number, proj.main_groups[0].middles[0].number);
            let name = {
                let d = proj.device(id).unwrap();
                format!("{} {} {}", d.name, d.objects[oi].name, d.objects[oi].function)
            };
            let g = new_ga(proj, m, mi, &name, &odpt);
            proj.device_mut(id).unwrap().objects[oi].links.push(g);
            let t = proj.ga_text(g);
            self.say(format!("Created and linked group address {t}"));
            changed = true;
        }
        if let Some(l) = unlink {
            proj.device_mut(id).unwrap().objects[oi].links.retain(|x| *x != l);
            changed = true;
        }
        if let Some(k) = make_first {
            let o = &mut proj.device_mut(id).unwrap().objects[oi];
            let g = o.links.remove(k);
            o.links.insert(0, g);
            changed = true;
        }
        if changed {
            proj.device_mut(id).unwrap().grp_ok = false;
            self.mark();
        }
    }

    fn parameters(&mut self, ui: &mut Ui, proj: &mut Project, id: u32) {
        if proj.device(id).map(|d| d.knx_app.is_some()).unwrap_or(false) {
            self.knx_parameters(ui, proj, id);
            return;
        }
        let mut pages: Vec<String> = vec![];
        for p in &proj.device(id).unwrap().params {
            if !pages.contains(&p.page) {
                pages.push(p.page.clone());
            }
        }
        if pages.is_empty() {
            ui.label(RichText::new("This device has no parameters.").color(theme::MUTED));
            return;
        }
        if !pages.contains(&self.ws.p.param_page) {
            self.ws.p.param_page = pages[0].clone();
        }
        let mut changed = false;
        let mut typing = false;
        egui_extras::StripBuilder::new(ui).size(egui_extras::Size::exact(200.0)).size(egui_extras::Size::remainder()).horizontal(|mut strip| {
            strip.cell(|ui| {
                theme::card_frame(ui).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    for p in &pages {
                        if ui.selectable_label(&self.ws.p.param_page == p, p).clicked() {
                            self.ws.p.param_page = p.clone();
                        }
                    }
                });
            });
            strip.cell(|ui| {
                theme::card_frame(ui).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                let page = self.ws.p.param_page.clone();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&page).size(15.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Reset page to defaults").clicked() {
                            for p in proj.device_mut(id).unwrap().params.iter_mut().filter(|p| p.page == page) {
                                if p.value != p.default {
                                    p.value = p.default.clone();
                                    changed = true;
                                }
                            }
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt("params").show(ui, |ui| {
                    egui::Grid::new("paramgrid").num_columns(2).spacing([24.0, 10.0]).striped(true).show(ui, |ui| {
                        let d = proj.device_mut(id).unwrap();
                        for (k, p) in d.params.iter_mut().enumerate().filter(|(_, p)| p.page == page) {
                            let label = if p.value != p.default { RichText::new(&p.name).strong() } else { RichText::new(&p.name) };
                            ui.label(label);
                            match p.kind.clone() {
                                ParamKind::Choice(opts) if opts.len() <= 2 => {
                                    ui.horizontal(|ui| {
                                        for o in &opts {
                                            if ui.radio(&p.value == o, o.as_str()).clicked() && &p.value != o {
                                                p.value = o.clone();
                                                changed = true;
                                            }
                                        }
                                    });
                                }
                                ParamKind::Choice(opts) => {
                                    egui::ComboBox::from_id_salt(("pc", k)).selected_text(p.value.clone()).width(220.0).show_ui(ui, |ui| {
                                        for o in &opts {
                                            if ui.selectable_label(&p.value == o, o).clicked() && &p.value != o {
                                                p.value = o.clone();
                                                changed = true;
                                            }
                                        }
                                    });
                                }
                                ParamKind::Number { min, max, unit } => {
                                    let mut v: i64 = p.value.parse().unwrap_or(min);
                                    ui.horizontal(|ui| {
                                        if ui.add(egui::DragValue::new(&mut v).range(min..=max)).changed() {
                                            p.value = v.to_string();
                                            changed = true;
                                            typing = true;
                                        }
                                        ui.label(RichText::new(format!("{unit}  ({min}…{max})")).color(theme::MUTED));
                                    });
                                }
                                ParamKind::Flag => {
                                    let b = p.value == "1";
                                    ui.horizontal(|ui| {
                                        if ui.radio(!b, "no").clicked() && b {
                                            p.value = "0".into();
                                            changed = true;
                                        }
                                        if ui.radio(b, "yes").clicked() && !b {
                                            p.value = "1".into();
                                            changed = true;
                                        }
                                    });
                                }
                                ParamKind::Text => {
                                    if ui.add(egui::TextEdit::singleline(&mut p.value).desired_width(220.0)).changed() {
                                        changed = true;
                                        typing = true;
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
                });
            });
            });
        });
        if changed {
            proj.device_mut(id).unwrap().par_ok = false;
            if typing {
                self.mark_typing()
            } else {
                self.mark()
            }
        }
    }
}
