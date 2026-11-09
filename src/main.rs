// No console window in normal use on Windows.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod cli;
mod config_edit;
mod fleet;
mod gateway;
#[cfg(test)]
mod gateway_tests;
mod identity;
mod look;
mod manager;
mod manager_ui;
mod model;
mod probe;
mod settings;
mod setup;
mod ui;
mod update;

use eframe::egui;

/// The window icon (the same picture as the program file's icon).
pub fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default()
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[cfg(windows)]
fn attach_console() {
    // A windows-subsystem program has no console; reconnect to the caller's
    // so `--probe` can print.
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
    }
    unsafe {
        AttachConsole(0xFFFF_FFFF);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        attach_console();
        println!("OpenClaw Bot {}", update::CURRENT);
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "--update-now") {
        attach_console();
        update::cleanup();
        std::process::exit(update::run_cli());
    }
    if args.iter().any(|a| a == "--probe" || a == "--probe-admin") {
        attach_console();
        std::process::exit(probe::run(&args));
    }
    let screenshot = arg_value(&args, "--screenshot").map(std::path::PathBuf::from);
    update::cleanup();
    if args.iter().any(|a| a == "--updated") {
        std::thread::spawn(setup::refresh_registered_version);
    }
    if args.iter().any(|a| a == "--uninstall") {
        return setup::run_uninstall();
    }
    if setup::is_setup_mode(&args) {
        if args.iter().any(|a| a == "--auto-install") {
            attach_console();
        }
        return setup::run_setup(args, screenshot);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenClaw Bot")
            .with_icon(app_icon())
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenClaw Bot",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, screenshot.clone())))),
    )
}
