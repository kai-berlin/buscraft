#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backup;
mod bus;
mod chrome;
mod dl_ui;
mod dock;
mod editor;
mod knx;
mod import_ui;
mod knxapp;
mod knxcalc;
mod knxedit;
mod knxfile;
mod knxload;
mod knxproj;
#[cfg(test)]
mod knxtests;
#[cfg(test)]
mod dltests;
mod library;
mod model;
mod panes;
mod props;
mod theme;
mod workspace;

use std::path::PathBuf;

struct Runner(app::App);

impl eframe::App for Runner {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.frame(ui);
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let shot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1)).map(PathBuf::from);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("BusCraft")
        .with_inner_size([1366.0, 840.0])
        .with_min_inner_size([960.0, 600.0]);
    if shot.is_some() {
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    #[cfg(target_os = "macos")]
    if shot.is_some() {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        options.event_loop_builder = Some(Box::new(|b| {
            b.with_activate_ignoring_other_apps(false);
        }));
    }

    let mut state = app::App::new(shot);
    // Optional start view for previews: BPS_PREVIEW=topology|buildings|groups|device|params|monitor
    if let Ok(v) = std::env::var("BPS_PREVIEW") {
        state.preview(&v);
    }
    eframe::run_native("BusCraft", options, Box::new(move |_cc| Ok(Box::new(Runner(state)))))
}
