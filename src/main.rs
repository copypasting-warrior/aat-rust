mod gui;
mod firewall;
mod models;

/// Initializes the eframe window with fixed dimensions and launches the GUI.
fn main() -> eframe::Result<()> {
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