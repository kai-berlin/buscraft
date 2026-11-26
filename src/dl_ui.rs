//! Application download in the UI: building the plan for a device and the confirmation
//! window shown before anything is written to a real device.

use egui::{Align2, Color32, RichText};

use crate::app::*;
use crate::bus::JobKind;
use crate::knxload::{self, DlKind, Plan, Step};
use crate::model::*;
use crate::theme;

pub struct DlConfirm {
    pub device: u32,
    pub kind: JobKind,
    pub title: String,
    pub plan: Plan,
}

impl App {
    /// The download plan for a device with an imported application. `Ok(None)`: nothing to plan
    /// (address programming, or a built-in sample product).
    pub(crate) fn plan_for(&mut self, proj: &Project, dev: u32, kind: JobKind) -> Result<Option<Plan>, String> {
        let Some(d) = proj.device(dev) else { return Err("Device not found".into()) };
        let Some(app_id) = d.knx_app.clone() else { return Ok(None) };
        let dk = match kind {
            JobKind::Address => return Ok(None),
            JobKind::Check => DlKind::Full,
            JobKind::Full | JobKind::Application => DlKind::Full,
            JobKind::Partial => DlKind::Partial,
            JobKind::Unload => DlKind::Unload,
        };
        let app = self.library.app(&app_id).ok_or("The product data of this device is not in the library – import its product file again")?;
        let li = app
            .load
            .as_ref()
            .ok_or("This product was imported before downloads were possible – import its product file (or the project) again to enable downloading")?;
        if app.format < 3 {
            return Err("This product was imported with an older version that ignored the application's automatic parameter rules. Import its product file or the project again (once) – then download.".into());
        }
        // Parameter values including those the application sets itself (Assign rules, calculations)
        let (eff, notes) = app.effective_values(&d.knx_values);
        let active = app.active(&d.knx_values);
        let mut d2 = d.clone();
        d2.knx_values = eff;
        let d = &d2;
        let mask = self.library.masks.get(&li.mask).cloned();
        knxload::supported(li, mask.as_ref())?;
        if kind == JobKind::Check {
            return knxload::build_check_plan(li, mask.as_ref().unwrap(), d, proj, Some(&active)).map(|mut p| {
                p.warnings.extend(notes);
                Some(p)
            });
        }
        knxload::build_plan(li, mask.as_ref().unwrap(), d, proj, dk, Some(&active)).map(|mut p| {
            p.warnings.extend(notes);
            Some(p)
        })
    }

    pub(crate) fn download_confirm_dialog(&mut self, ctx: &egui::Context, proj: &mut Project) {
        let Some(c) = &self.dl_confirm else { return };
        let mut start = false;
        let mut cancel = false;
        let mut save = false;
        egui::Window::new("Download to device").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.label(RichText::new(&c.title).strong().size(14.0));
            ui.label(format!(
                "{} steps · {} bytes will be written to the device's memory via {}.",
                c.plan.steps.len(),
                c.plan.total_bytes(),
                self.bus.iface().name
            ));
            ui.add_space(4.0);
            egui::Frame::new().fill(Color32::from_rgb(0xFF, 0xF6, 0xE0)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.label(RichText::new("⚠ Experimental").strong().color(theme::WARN));
                ui.label(
                    "Downloading follows the load procedure from the product file. The device type is checked first and \
                     every write is read back. It has not been tried on every device yet: if something goes wrong, the device \
                     can stay unloaded until a download succeeds (with this app or another tool).",
                );
            });
            for w in &c.plan.warnings {
                ui.label(RichText::new(format!("ⓘ {w}")).small().color(theme::MUTED));
            }
            ui.add_space(4.0);
            egui::CollapsingHeader::new(format!("Show the {} steps", c.plan.steps.len())).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (i, s) in c.plan.steps.iter().enumerate() {
                        ui.label(RichText::new(format!("{:>3}. {}", i + 1, s.describe())).small().monospace());
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Start download").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                    start = true;
                }
                if ui.button("Save steps…").on_hover_text("Save a readable list of all steps and bytes").clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if save {
            let c = self.dl_confirm.as_ref().unwrap();
            if let Some(path) = rfd::FileDialog::new().add_filter("Text", &["txt"]).set_file_name("download-plan.txt").save_file() {
                let text = plan_text(&c.title, &c.plan);
                match std::fs::write(&path, text) {
                    Ok(()) => self.say(format!("Saved the download steps to {}", path.display())),
                    Err(e) => self.say(format!("Could not save: {e}")),
                }
            }
        }
        if cancel {
            self.dl_confirm = None;
        } else if start {
            let c = self.dl_confirm.take().unwrap();
            match self.bus.enqueue_plan(proj, c.device, c.kind, Some(c.plan.steps)) {
                Ok(()) => {
                    self.say(format!("{} started", c.plan.kind));
                    self.ws.side = crate::chrome::Side::Running;
                    self.open_dock(Dock::Jobs);
                }
                Err(e) => self.say(e),
            }
        }
    }
}

/// A download that could not start, shown in a window (not only in the status line).
pub struct Problem {
    pub title: String,
    pub text: String,
    /// The fix is importing the product data again.
    pub reimport: bool,
}

impl App {
    /// Window explaining why a download could not start, with the fix where there is one.
    pub(crate) fn problem_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.problem else { return };
        let mut close = false;
        let mut import = false;
        egui::Window::new(p.title.as_str()).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_max_width(480.0);
            ui.label(&p.text);
            if p.reimport {
                ui.add_space(6.0);
                ui.label(RichText::new("Choose the device's product file (.knxprod) or the project file (.knxproj). For a project file no password is needed – only its products are read. The download then continues automatically.").small().color(theme::MUTED));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if p.reimport {
                    if ui.add(egui::Button::new(RichText::new("📥 Import products…").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                        import = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                } else if ui.add(egui::Button::new(RichText::new("OK").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.problem = None;
            self.pending_download = None;
        }
        if import {
            self.problem = None;
            self.pick_and_import(true);
            if self.import_job.is_none() {
                // File dialog cancelled
                self.pending_download = None;
            }
        }
    }

    /// Window with the result of a device check.
    pub(crate) fn report_dialog(&mut self, ctx: &egui::Context) {
        let Some((title, text)) = &self.report_view else { return };
        let mut close = false;
        let mut save = false;
        egui::Window::new(title.as_str()).collapsible(false).resizable(true).default_width(620.0).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            let problems = text.lines().filter(|l| l.contains('⚠')).count();
            if problems == 0 {
                ui.label(RichText::new("✔ The device matches the project: group addresses, links, group objects and parameters are as planned.").color(theme::OK));
            } else {
                ui.label(RichText::new(format!("⚠ {problems} difference(s) between the device and the project.")).color(theme::WARN).strong());
                ui.label(RichText::new("If parameters or tables differ, run a full download again. If everything matches but the device still does not react, check the parameters on its pages (e.g. channel functions) and watch the group monitor while using the device.").small().color(theme::MUTED));
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                for l in text.lines() {
                    let rt = RichText::new(l).monospace().size(12.0);
                    ui.label(if l.contains('⚠') { rt.color(theme::BAD) } else if l.contains('✔') { rt.color(theme::OK) } else { rt });
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                if ui.button("Save…").clicked() {
                    save = true;
                }
                if ui.add(egui::Button::new(RichText::new("Close").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                    close = true;
                }
            });
        });
        if save {
            if let Some((title, text)) = &self.report_view {
                if let Some(path) = rfd::FileDialog::new().add_filter("Text", &["txt"]).set_file_name(format!("{}.txt", title.replace(['/', '\\', ':'], "-"))).save_file() {
                    let _ = std::fs::write(&path, text);
                }
            }
        }
        if close {
            self.report_view = None;
        }
    }
}

/// A readable listing of a plan, including the bytes written.
pub fn plan_text(title: &str, plan: &Plan) -> String {
    let mut s = format!("{title}\n{} – {} steps, {} bytes\n\n", plan.kind, plan.steps.len(), plan.total_bytes());
    for w in &plan.warnings {
        s.push_str(&format!("Note: {w}\n"));
    }
    for (i, st) in plan.steps.iter().enumerate() {
        s.push_str(&format!("{:>3}. {}\n", i + 1, st.describe()));
        let bytes: Option<&[u8]> = match st {
            Step::WriteProp { data, .. } => Some(data),
            Step::PatchProp { value, .. } => Some(value),
            Step::WriteMem { data, .. } => Some(data),
            _ => None,
        };
        if let Some(b) = bytes {
            for line in b.chunks(16).take(64) {
                s.push_str("       ");
                s.push_str(&line.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" "));
                s.push('\n');
            }
            if b.len() > 1024 {
                s.push_str(&format!("       … {} more bytes\n", b.len() - 1024));
            }
        }
    }
    s
}
