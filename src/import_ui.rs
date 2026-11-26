//! Import of KNX product (.knxprod) and project (.knxproj) files in the UI:
//! background job, password prompt, results summary.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

use egui::{Align2, Color32, RichText};

use crate::app::*;
use crate::knxfile::{self, ImportErr, ImportOut, Progress};
use crate::model::*;
use crate::theme;

pub struct ImportJob {
    rx: Receiver<Result<ImportOut, ImportErr>>,
    progress: Progress,
    path: PathBuf,
    products_only: bool,
}

pub struct PasswordPrompt {
    path: PathBuf,
    name: String,
    password: String,
    error: Option<String>,
}

pub struct ImportReport {
    title: String,
    lines: Vec<String>,
    warnings: Vec<String>,
    open_project: Option<u64>,
}

pub const KNX_EXTS: &[&str] = &["knxproj", "knxprod", "vd1", "vd2", "vd3", "vd4", "vd5"];

pub const LANGUAGES: &[(&str, &str)] = &[
    ("en-US", "English"),
    ("de-DE", "Deutsch"),
    ("fr-FR", "Français"),
    ("it-IT", "Italiano"),
    ("es-ES", "Español"),
    ("nl-NL", "Nederlands"),
    ("pl-PL", "Polski"),
    ("sv-SE", "Svenska"),
];

impl App {
    /// Rebuild the catalog list: built-in sample products plus imported ones.
    pub fn refresh_catalog(&mut self) {
        let mut c = catalog();
        c.extend(self.library.products.iter().cloned());
        self.catalog = c;
        self.ws.p.cat_sel = None;
    }

    /// Ask for a KNX file and import it. `products_only` is used by the catalog's import button.
    pub fn pick_and_import(&mut self, products_only: bool) {
        let mut d = rfd::FileDialog::new().add_filter("KNX project and product files", KNX_EXTS);
        if !products_only {
            d = d.add_filter("BusCraft project", &["bcproj", "bpsproj", "json"]);
        }
        let Some(path) = d.pick_file() else { return };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if ext == "bcproj" || ext == "bpsproj" || ext == "json" {
            self.import_own_project(path);
        } else {
            self.start_import(path, None, products_only);
        }
    }

    pub fn import_own_project(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Project>(&s).map_err(|e| e.to_string())) {
            Ok(mut p) => {
                p.id = fresh_id();
                let name = p.name.clone();
                self.ov_sel = Some(p.id);
                self.store.projects.push(p);
                self.dirty = true;
                self.say(format!("Imported project \"{name}\""));
            }
            Err(e) => self.say(format!("Import failed: {e}")),
        }
    }

    pub fn start_import(&mut self, path: PathBuf, password: Option<String>, products_only: bool) {
        if self.import_job.is_some() {
            self.say("Another import is still running");
            return;
        }
        let (tx, rx) = channel();
        let progress = Progress::default();
        progress.set("Opening file…");
        let p2 = progress.clone();
        let lang = self.store.product_language.clone();
        let path2 = path.clone();
        std::thread::spawn(move || {
            let r = knxfile::import(&path2, password.as_deref(), products_only, &lang, &p2);
            let _ = tx.send(r);
        });
        self.import_job = Some(ImportJob { rx, progress, path, products_only });
    }

    pub fn poll_import(&mut self) {
        let Some(job) = &self.import_job else { return };
        let Ok(res) = job.rx.try_recv() else { return };
        let job = self.import_job.take().unwrap();
        let file = job.path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
        match res {
            Ok(out) => {
                let napps = out.apps.len();
                let nprod = out.products.len();
                let mut lines = vec![];
                self.library.add_masks(out.masks);
                match self.library.add(out.products, out.apps) {
                    Ok((new, upd)) => {
                        lines.push(format!("{nprod} product(s) found: {new} added to the catalog, {upd} updated"));
                        lines.push(format!("{napps} application program(s) with parameters and group objects"));
                    }
                    Err(e) => lines.push(format!("The products could not be saved: {e}")),
                }
                self.refresh_catalog();
                if self.pending_download.is_some() {
                    // Continue the download that was waiting for this product data.
                    self.retry_download = true;
                }
                let mut open_project = None;
                let mut title = format!("Imported {file}");
                if let Some(mut p) = out.project {
                    p.id = fresh_id();
                    let links: usize = p.devices.iter().flat_map(|d| d.objects.iter()).map(|o| o.links.len()).sum();
                    lines.insert(0, format!("Project \"{}\": {} devices, {} group addresses, {} links", p.name, p.devices.len(), p.addresses.len(), links));
                    title = format!("Imported project \"{}\"", p.name);
                    open_project = Some(p.id);
                    self.ov_sel = Some(p.id);
                    self.store.projects.insert(0, p);
                    self.dirty = true;
                } else if job.products_only && file.to_lowercase().ends_with(".knxproj") {
                    lines.push("Only the products were imported (no project data)".into());
                }
                self.say(title.clone());
                self.import_report = Some(ImportReport { title, lines, warnings: out.warnings, open_project });
            }
            Err(ImportErr::NeedPassword(name)) => {
                self.pw_prompt = Some(PasswordPrompt { path: job.path, name, password: String::new(), error: None });
            }
            Err(ImportErr::WrongPassword(name)) => {
                self.pw_prompt = Some(PasswordPrompt { path: job.path, name, password: String::new(), error: Some("The password is not correct. Please try again.".into()) });
            }
            Err(ImportErr::Failed(e)) => {
                self.pending_download = None;
                self.say(format!("Import of {file} failed"));
                self.import_report = Some(ImportReport { title: format!("Could not import {file}"), lines: vec![e], warnings: vec![], open_project: None });
            }
        }
    }

    pub fn import_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.import_job {
            let text = job.progress.get();
            let file = job.path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
            egui::Window::new("Importing").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(&file).strong());
                });
                ui.label(RichText::new(text).color(theme::MUTED));
            });
            ctx.request_repaint();
        }

        let mut pw_action: Option<(bool, bool)> = None; // (cancel, products_only)
        if let Some(pw) = &mut self.pw_prompt {
            egui::Window::new("Project password").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(400.0);
                ui.label(format!("The project \"{}\" is protected with a password. Enter it to import the project.", pw.name));
                ui.add_space(6.0);
                let r = ui.add(egui::TextEdit::singleline(&mut pw.password).password(true).hint_text("Password").desired_width(f32::INFINITY));
                r.request_focus();
                if let Some(e) = &pw.error {
                    ui.label(RichText::new(e).color(theme::BAD));
                }
                ui.label(RichText::new("The password is only used to open the file and is not stored.").small().color(theme::MUTED));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.add_enabled(!pw.password.is_empty(), egui::Button::new(RichText::new("Import project").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() || (enter && !pw.password.is_empty()) {
                        pw_action = Some((false, false));
                    }
                    if ui.button("Import products only").on_hover_text("Add the products used in this project to the catalog, without the project itself").clicked() {
                        pw_action = Some((false, true));
                    }
                    if ui.button("Cancel").clicked() {
                        pw_action = Some((true, false));
                    }
                });
            });
        }
        if let Some((cancel, products_only)) = pw_action {
            let pw = self.pw_prompt.take().unwrap();
            if !cancel {
                let password = if products_only { None } else { Some(pw.password) };
                self.start_import(pw.path, password, products_only);
            }
        }

        let mut close = false;
        let mut open = None;
        if let Some(rep) = &self.import_report {
            egui::Window::new(&rep.title).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(460.0);
                for l in &rep.lines {
                    ui.label(l);
                }
                if !rep.warnings.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("{} note(s):", rep.warnings.len())).color(theme::WARN));
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                        for w in &rep.warnings {
                            ui.label(RichText::new(w).small().color(theme::MUTED));
                        }
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(id) = rep.open_project {
                        if ui.add(egui::Button::new(RichText::new("Open project").color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                            open = Some(id);
                            close = true;
                        }
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        }
        if close {
            self.import_report = None;
        }
        if let Some(id) = open {
            self.open_project(id);
        }
    }

    pub fn language_picker(&mut self, ui: &mut egui::Ui) {
        let cur = LANGUAGES.iter().find(|(c, _)| *c == self.store.product_language).map(|(_, n)| *n).unwrap_or("English");
        egui::ComboBox::from_id_salt("prodlang").selected_text(cur).show_ui(ui, |ui| {
            for (code, name) in LANGUAGES {
                if ui.selectable_label(self.store.product_language == *code, *name).clicked() {
                    self.store.product_language = code.to_string();
                    self.dirty = true;
                }
            }
        });
    }
}
