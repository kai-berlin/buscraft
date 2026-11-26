//! The panel stack of a project view: any number of equal panels arranged vertically on the
//! left (each can show topology, buildings, group addresses, devices, catalog or monitors),
//! plus drag & drop linking of group objects and group addresses across panels.

use egui::{Align, Color32, Layout, Margin, RichText, Sense, Stroke, Ui};

use crate::app::*;
use crate::model::*;
use crate::theme;

/// Something being dragged between panels.
#[derive(Clone, Debug)]
pub enum DragItem {
    /// A group object of a device (index into `Device::objects`)
    Object { dev: u32, idx: usize, label: String },
    /// A group address
    Ga { id: u32, label: String },
}

fn header_frame() -> egui::Frame {
    egui::Frame::new().fill(Color32::from_rgb(0xF5, 0xF5, 0xF5)).inner_margin(Margin::symmetric(6, 3))
}

impl App {
    pub(crate) fn ws_panes(&mut self, ui: &mut Ui, proj: &mut Project) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::WHITE)).show(ui, |ui| {
            let avail = ui.available_rect_before_wrap();
            let n = self.ws.panes.len();
            const SPLIT: f32 = 5.0;
            const MIN: f32 = 90.0;
            // Heights: the stored sizes, scaled to fill the available height.
            let mut sizes: Vec<f32> = (0..n).map(|i| if i == self.ws.focus { self.ws.p.size } else { self.ws.panes[i].size }.max(MIN)).collect();
            let total = (avail.height() - SPLIT * (n as f32 - 1.0)).max(MIN * n as f32);
            let sum: f32 = sizes.iter().sum();
            for s in &mut sizes {
                *s = (*s / sum * total).max(MIN);
            }
            let mut y = avail.top();
            for i in 0..n {
                let rect = egui::Rect::from_min_size(egui::pos2(avail.left(), y), egui::vec2(avail.width(), sizes[i]));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
                child.set_clip_rect(rect.intersect(ui.clip_rect()));
                self.pane(&mut child, proj, i);
                y += sizes[i];
                if i + 1 < n {
                    // Divider: drag to change the heights of the two panels
                    let sr = egui::Rect::from_min_size(egui::pos2(avail.left(), y), egui::vec2(avail.width(), SPLIT));
                    let id = crate::workspace::nid(("split", i, if i == self.ws.focus { self.ws.p.id } else { self.ws.panes[i].id }));
                    let r = ui.interact(sr, id, Sense::drag());
                    let hot = r.hovered() || r.dragged();
                    ui.painter().rect_filled(sr, 0.0, if hot { theme::ACCENT_SOFT } else { Color32::from_rgb(0xEC, 0xEC, 0xEC) });
                    ui.painter().hline(sr.left()..=sr.right(), sr.center().y, Stroke::new(1.0, if hot { theme::ACCENT } else { theme::LINE }));
                    if hot {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if r.dragged() {
                        let d = r.drag_delta().y;
                        let d = d.clamp(MIN - sizes[i], sizes[i + 1] - MIN);
                        sizes[i] += d;
                        sizes[i + 1] -= d;
                    }
                    y += SPLIT;
                }
            }
            for (i, s) in sizes.into_iter().enumerate() {
                if i == self.ws.focus {
                    self.ws.p.size = s;
                } else {
                    self.ws.panes[i].size = s;
                }
            }
            ui.allocate_rect(avail, Sense::hover());
        });
        self.finish_panes(ui.ctx(), proj);
    }

    /// Draw panel `i`. Its state is moved into `self.ws.p` while drawing.
    fn pane(&mut self, ui: &mut Ui, proj: &mut Project, i: usize) {
        let focused = i == self.ws.focus;
        let saved = if focused { None } else { Some(std::mem::replace(&mut self.ws.p, std::mem::take(&mut self.ws.panes[i]))) };
        let n = self.ws.panes.len();
        let rect = ui.max_rect();
        if ui.rect_contains_pointer(rect) && ui.input(|inp| inp.pointer.any_pressed()) && !focused {
            self.ws.new_focus = Some(i);
        }
        // Remember the last used tree panel (target for adding devices from a catalog panel)
        if focused && matches!(self.ws.p.view, View::Topology | View::Buildings | View::Devices) {
            self.ws.last_tree = (self.ws.p.view, self.ws.p.sel);
        }
        let pid = self.ws.p.id;
        ui.push_id(pid, |ui| {
            // Header: view picker, add panel, close
            egui::Panel::top("phead").frame(header_frame()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon, title) = crate::chrome::view_label(self.ws.p.view);
                    ui.label(RichText::new(icon).color(theme::ACCENT));
                    let mut pick = None;
                    egui::ComboBox::from_id_salt("viewpick").selected_text(RichText::new(title).strong()).width(170.0).show_ui(ui, |ui| {
                        for v in View::ALL {
                            let (vi, vt) = crate::chrome::view_label(v);
                            if ui.selectable_label(self.ws.p.view == v, format!("{vi} {vt}")).clicked() {
                                pick = Some(v);
                            }
                        }
                    });
                    if let Some(v) = pick {
                        self.set_view(v);
                        self.ws.p.dev_folder = 0;
                    }
                    if focused && n > 1 {
                        ui.label(RichText::new("●").small().color(theme::ACCENT)).on_hover_text("Active panel – the sidebar and toolbar refer to it");
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if n > 1 && theme::tool(ui, true, "✕").on_hover_text("Close this panel").clicked() {
                            self.ws.pane_ops.push(PaneOp::Close(i));
                        }
                        ui.menu_button(RichText::new("+ Panel").size(12.5), |ui| {
                            ui.label(RichText::new("Add a panel below showing").small().color(theme::MUTED));
                            for v in View::ALL {
                                let (vi, vt) = crate::chrome::view_label(v);
                                if ui.button(format!("{vi} {vt}")).clicked() {
                                    self.ws.pane_ops.push(PaneOp::Add(i, v));
                                    ui.close();
                                }
                            }
                        });
                    });
                });
            });
            // Tree on the left, content on the right
            let tree_w = self.ws.p.tree_w;
            let r = egui::Panel::left("ptree").resizable(true).default_size(tree_w).min_size(150.0).frame(theme::surface_frame(ui)).show(ui, |ui| {
                match self.ws.p.view {
                    View::Catalog => self.catalog_tree(ui),
                    View::Monitors => self.monitor_tree(ui),
                    _ => self.ws_tree(ui, proj),
                }
            });
            self.ws.p.tree_w = r.response.rect.width();
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(Margin::same(8))).show(ui, |ui| match self.ws.p.view {
                View::Catalog => self.catalog_pane(ui, proj),
                View::Monitors => match self.ws.p.mon_view {
                    MonView::GroupMonitor => self.monitor(ui, proj, false),
                    MonView::BusMonitor => self.monitor(ui, proj, true),
                    MonView::Jobs => self.jobs_view(ui, proj),
                    MonView::DeviceInfo => self.diag_page(ui, proj, crate::diag::DiagPage::Info),
                    MonView::ProgMode => self.diag_page(ui, proj, crate::diag::DiagPage::Prog),
                    MonView::AddressCheck => self.diag_page(ui, proj, crate::diag::DiagPage::Check),
                    MonView::LineScan => self.diag_page(ui, proj, crate::diag::DiagPage::Scan),
                },
                _ => {
                    if let Some(dev) = self.ws.p.open_device {
                        if proj.device(dev).is_some() {
                            self.device_editor(ui, proj, dev);
                            return;
                        }
                        self.ws.p.open_device = None;
                    }
                    self.ws_center(ui, proj);
                }
            });
        });
        // Thin divider between panels; accent line on the active one
        let c = if focused && n > 1 { theme::ACCENT } else { theme::LINE };
        ui.painter().hline(rect.left()..=rect.right(), rect.top(), Stroke::new(if focused && n > 1 { 2.0 } else { 1.0 }, c));
        if let Some(cur) = saved {
            self.ws.panes[i] = std::mem::replace(&mut self.ws.p, cur);
        }
    }

    /// Tree of a Monitors panel, grouped like the original: Monitor / Diagnostics.
    fn monitor_tree(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let section = |ui: &mut Ui, text: &str| {
            egui::Frame::new().fill(Color32::from_rgb(0xF0, 0xF0, 0xF0)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(format!("–   {text}")).size(13.0));
            });
        };
        section(ui, "Monitor");
        for (m, t) in [(MonView::GroupMonitor, "Group Monitor"), (MonView::BusMonitor, "Bus Monitor")] {
            if crate::knxedit::page_item(ui, t, 1, false, self.ws.p.mon_view == m).clicked() {
                self.ws.p.mon_view = m;
            }
        }
        ui.add_space(6.0);
        section(ui, "Diagnostics");
        if crate::knxedit::page_item(ui, "Device Info", 1, false, self.ws.p.mon_view == MonView::DeviceInfo).clicked() {
            self.ws.p.mon_view = MonView::DeviceInfo;
        }
        let ia_open = matches!(self.ws.p.mon_view, MonView::ProgMode | MonView::AddressCheck | MonView::LineScan);
        if crate::knxedit::page_item(ui, "Individual Addresses", 0, true, false).clicked() && !ia_open {
            self.ws.p.mon_view = MonView::ProgMode;
        }
        for (m, t) in [(MonView::ProgMode, "Programming Mode"), (MonView::AddressCheck, "Individual Address Check"), (MonView::LineScan, "Line Scan")] {
            if crate::knxedit::page_item(ui, t, 1, false, self.ws.p.mon_view == m).clicked() {
                self.ws.p.mon_view = m;
            }
        }
        let pending = self.bus.jobs.iter().filter(|j| matches!(j.state, crate::bus::JobState::Waiting | crate::bus::JobState::PressButton | crate::bus::JobState::Running)).count();
        let jt = if pending > 0 { format!("Pending Operations ({pending})") } else { "Pending Operations".into() };
        if crate::knxedit::page_item(ui, &jt, 1, false, self.ws.p.mon_view == MonView::Jobs).clicked() {
            self.ws.p.mon_view = MonView::Jobs;
        }
    }

    /// End of frame: focus changes, added/closed panels, "show in a panel" requests,
    /// drag & drop results and the drag ghost.
    fn finish_panes(&mut self, ctx: &egui::Context, proj: &mut Project) {
        // Put the focused panel back in place while rearranging.
        let focus = self.ws.focus.min(self.ws.panes.len().saturating_sub(1));
        self.ws.panes[focus] = std::mem::take(&mut self.ws.p);
        let mut focus = self.ws.new_focus.take().unwrap_or(focus);
        for op in std::mem::take(&mut self.ws.pane_ops) {
            match op {
                PaneOp::Add(i, v) => {
                    let mut p = Pane::new(v);
                    let h = self.ws.panes.get(i).map(|x| x.size).unwrap_or(300.0);
                    p.size = (h / 2.0).max(160.0);
                    if let Some(x) = self.ws.panes.get_mut(i) {
                        x.size = (h / 2.0).max(160.0);
                    }
                    self.ws.panes.insert(i + 1, p);
                    focus = i + 1;
                }
                PaneOp::Close(i) => {
                    if self.ws.panes.len() > 1 && i < self.ws.panes.len() {
                        self.ws.panes.remove(i);
                        if focus >= i && focus > 0 {
                            focus -= 1;
                        }
                    }
                }
            }
        }
        if let Some((view, mon)) = self.ws.pending_show.take() {
            let shows = |p: &Pane| p.view == view && mon.map(|m| p.mon_view == m).unwrap_or(true);
            if let Some(i) = self.ws.panes.iter().position(|p| shows(p)) {
                let _ = i; // already visible
            } else if let Some(i) = self.ws.panes.iter().position(|p| p.view == view) {
                if let Some(m) = mon {
                    self.ws.panes[i].mon_view = m;
                }
            } else if self.ws.panes.len() > 1 {
                // Reuse the last panel that is not the active one
                let i = (0..self.ws.panes.len()).rev().find(|j| *j != focus).unwrap_or(0);
                let p = &mut self.ws.panes[i];
                p.view = view;
                p.open_device = None;
                p.sel = Sel::None;
                if let Some(m) = mon {
                    p.mon_view = m;
                }
            } else {
                let mut p = Pane::new(view);
                if let Some(m) = mon {
                    p.mon_view = m;
                }
                p.size = 260.0;
                self.ws.panes.push(p);
            }
        }
        if let Some(t) = self.ws.diag_prefill.take() {
            for p in self.ws.panes.iter_mut().filter(|p| p.view == View::Monitors) {
                match p.mon_view {
                    MonView::LineScan => p.diag_line = t.clone(),
                    MonView::DeviceInfo | MonView::AddressCheck | MonView::ProgMode => p.diag_addr = t.clone(),
                    _ => {}
                }
            }
        }
        self.ws.focus = focus.min(self.ws.panes.len() - 1);
        self.ws.p = std::mem::take(&mut self.ws.panes[self.ws.focus]);

        // Drag & drop: link a group object with a group address
        if let Some((dev, idx, ga)) = self.ws.pending_link.take() {
            self.link_object(proj, dev, idx, ga);
        }
        if let Some(item) = egui::DragAndDrop::payload::<DragItem>(ctx) {
            if let Some(pos) = ctx.pointer_interact_pos() {
                let text = match &*item {
                    DragItem::Object { label, .. } => format!("🔗 {label}  → drop on a group address"),
                    DragItem::Ga { label, .. } => format!("🔗 {label}  → drop on a group object"),
                };
                egui::Area::new(crate::workspace::nid("dnd_ghost")).order(egui::Order::Tooltip).fixed_pos(pos + egui::vec2(14.0, 10.0)).interactable(false).show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new(text).small());
                    });
                });
            }
        }
    }

    /// Link a device's group object with a group address (from drag & drop).
    pub(crate) fn link_object(&mut self, proj: &mut Project, dev: u32, idx: usize, ga: u32) {
        let gtext = proj.ga(ga).map(|g| format!("{} {}", g.fmt(proj.ga_style), g.name)).unwrap_or_default();
        let Some(d) = proj.device_mut(dev) else { return };
        let addr = d.address();
        let Some(o) = d.objects.get_mut(idx) else { return };
        if o.links.contains(&ga) {
            self.say(format!("{gtext} is already linked with object {} of {addr}", o.number));
            return;
        }
        o.links.push(ga);
        let n = o.number;
        d.grp_ok = false;
        self.mark();
        self.say(format!("Linked {gtext} with object {n} of {addr}"));
    }

    /// Make a row/tree response a drag source and drop target for linking.
    /// `own` is what this element is; dropping the other kind on it requests a link.
    pub(crate) fn dnd_link(&mut self, r: &egui::Response, own: DragItem) {
        // Table rows already sense dragging; labels in trees need it added.
        let r = if r.sense.senses_drag() { r.clone() } else { r.interact(Sense::click_and_drag()) };
        r.dnd_set_drag_payload(own.clone());
        if let Some(p) = r.dnd_hover_payload::<DragItem>() {
            let compatible = matches!((&*p, &own), (DragItem::Object { .. }, DragItem::Ga { .. }) | (DragItem::Ga { .. }, DragItem::Object { .. }));
            if compatible {
                r.ctx.layer_painter(r.layer_id).rect_stroke(r.rect.expand(1.0), 2.0, Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Outside);
            }
        }
        if let Some(p) = r.dnd_release_payload::<DragItem>() {
            match (&*p, &own) {
                (DragItem::Object { dev, idx, .. }, DragItem::Ga { id, .. }) | (DragItem::Ga { id, .. }, DragItem::Object { dev, idx, .. }) => {
                    self.ws.pending_link = Some((*dev, *idx, *id));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::*;

    /// A project view with several panels survives saving and loading (restore on reopen).
    #[test]
    fn views_are_saved_and_restored() {
        let mut ws = Ws::second();
        ws.p.sel = Sel::Ga(42);
        ws.p.size = 333.0;
        ws.panes.push(Pane::new(View::Buildings));
        ws.panes[1].mon_view = MonView::BusMonitor;
        let json = serde_json::to_string(&vec![ws.saved()]).unwrap();
        let back: Vec<Ws> = serde_json::from_str(&json).unwrap();
        let w = back.into_iter().next().unwrap().restored();
        assert_eq!(w.panes.len(), 3);
        assert_eq!(w.p.view, View::GroupAddresses);
        assert_eq!(w.p.sel, Sel::Ga(42));
        assert_eq!(w.p.size, 333.0);
        assert_eq!(w.panes[1].view, View::Monitors);
        assert_eq!(w.panes[1].mon_view, MonView::BusMonitor);
        assert_eq!(w.panes[2].view, View::Buildings);
        // Old saved stores without views still load
        let none: std::collections::HashMap<String, Vec<Ws>> = serde_json::from_str("{}").unwrap();
        assert!(none.is_empty());
    }
}