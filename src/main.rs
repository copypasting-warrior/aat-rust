mod ai_client;
mod crypto;

mod gui;
mod firewall;
mod models;


use std::env;
use std::fs;

fn load_env_file() {
    let Ok(contents) = fs::read_to_string(".env") else {
        return;
    };

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() || env::var_os(key).is_some() {
            continue;
        }

        let mut value = value.trim().to_string();
        if value.len() >= 2 {
            let quoted = (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''));
            if quoted {
                value = value[1..value.len() - 1].to_string();
            }
        }

        env::set_var(key, value);
    }
}


/// Initializes the eframe window with fixed dimensions and launches the GUI.
fn main() -> eframe::Result<()> {
    load_env_file();

    // Configure window options with fixed size of 1200x675 pixels
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 675.0])
            .with_resizable(false),
        ..Default::default()
    };

    // Start the native eframe application with the configured options
    // Creates a new AppState instance to manage the application
    eframe::run_native(
        "SSD Health Checker",
        options,
        Box::new(|cc| Ok(Box::new(gui::AppState::new(cc)))),
    )
}