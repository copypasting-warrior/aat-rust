// Main application state and UI rendering logic for the SSD Health Checker

// Import AI client for calling Python AI service
use crate::ai_client;
use crate::firewall::{scan_firewall, FirewallSnapshot};
// Import disk scanning functionality
use crate::gui::{disk_scanner::scan_disks, stat_card};
// Import disk information models
use crate::models::{AiResult, DiskInfo, TelemetrySnapshot};

// Import egui for UI rendering
use eframe::egui;
// Regex for parsing system command output
use regex::Regex;
// Command execution for reading system temperatures
use std::process::Command;
// Arc for thread-safe reference counting, Mutex for shared state across threads
use std::sync::{Arc, Mutex};
// Duration and Instant for time-based operations
use std::time::{Duration, Instant};
// Collections for storing AI results keyed by device path
use std::collections::HashMap;

use serde_json::json;
use std::env;

use crate::crypto::Encryptor;

struct TelemetryConfig {
    endpoint: String,
    key_b64: String,
}

fn telemetry_config_from_env() -> Option<TelemetryConfig> {
    let endpoint = env::var("TELEMETRY_ENDPOINT").ok()?.trim().to_string();
    if endpoint.is_empty()
        || endpoint.contains("YOUR-SUBDOMAIN")
    {
        eprintln!("Telemetry disabled: set TELEMETRY_ENDPOINT to your deployed worker URL");
        return None;
    }

    let key_b64 = env::var("TELEMETRY_KEY").ok()?;
    Some(TelemetryConfig { endpoint, key_b64 })
}

/// Main application state for the eframe app.
/// Manages disk information, system temperatures, and UI state.
pub struct AppState {
    /// The discovered drives wrapped in Arc for efficient cloning
    drives: Vec<Arc<DiskInfo>>,

    /// Index of currently selected drive in the drives vector
    selected: usize,

    /// Last error message if scanning drives failed
    last_error: Option<String>,

    /// Cached CPU temperature average in Celsius
    cpu_temp: Option<f32>,

    /// Cached GPU temperature in Celsius
    gpu_temp: Option<f32>,

    /// Snapshot of firewall state discovered from ufw/nftables/iptables
    firewall: FirewallSnapshot,

    /// Total incoming RX packets across non-loopback interfaces
    incoming_packets: Option<u64>,

    /// RX packets treated as blocked/failed (errors + drops)
    blocked_packets: Option<u64>,

    /// Packets approved/accepted at interface level
    approved_packets: Option<u64>,

    /// Timestamp of the last automatic refresh
    last_refresh: Instant,

    /// How often to automatically refresh drive data
    refresh_interval: Duration,

    // ---------------------------------------------------------------- //
    // AI state
    // ---------------------------------------------------------------- //

    /// AI health predictions keyed by device path (e.g. "/dev/nvme0n1").
    /// Populated in a background thread after each scan.
    ai_results: Arc<Mutex<HashMap<String, AiResult>>>,

    /// True while the background AI call is in flight
    ai_loading: bool,

    /// User's typed question in the NLP Q&A text box
    nlp_input: String,

    /// Last NLP answer received from the AI service
    nlp_response: Arc<Mutex<Option<String>>>,

    /// True while the background NLP call is in flight
    nlp_loading: bool,

    telemetry_data: Arc<Mutex<TelemetrySnapshot>>,
    telemetry_config: Option<TelemetryConfig>,

}

impl AppState {
    /// Creates a new application state instance.
    /// Sets light theme, performs initial data collection, and starts refresh timer.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Configure light theme for consistent appearance
        cc.egui_ctx.set_visuals(egui::Visuals::light());

        let telemetry_data = Arc::new(Mutex::new(TelemetrySnapshot {
            drives: Vec::new(),
            cpu_temp: None,
            gpu_temp: None,
            incoming_packets: None,
            blocked_packets: None,
            approved_packets: None,
        }));

        let mut s = Self {
            drives: Vec::new(),
            selected: 0,
            last_error: None,
            cpu_temp: None,
            gpu_temp: None,
            firewall: FirewallSnapshot::unavailable("Firewall not scanned yet"),
            incoming_packets: None,
            blocked_packets: None,
            approved_packets: None,
            // Force immediate refresh by setting last refresh to 10 seconds ago
            last_refresh: Instant::now() - Duration::from_secs(10),
            // Automatically refresh data every 5 seconds
            refresh_interval: Duration::from_secs(5),
            // AI state
            ai_results: Arc::new(Mutex::new(HashMap::new())),
            ai_loading: false,
            nlp_input: String::new(),
            nlp_response: Arc::new(Mutex::new(None)),
            nlp_loading: false,
            telemetry_data,
            telemetry_config: telemetry_config_from_env(),

        };

        s.refresh();
        s.update_system_temps();
        s
    }

    /// Refreshes the disk list by calling scan_disks.
    fn refresh(&mut self) {
        self.last_error = None;
        match scan_disks() {
            Ok(list) => {
                self.drives = list.into_iter().map(Arc::new).collect();

                if !self.drives.is_empty() && self.selected >= self.drives.len() {
                    self.selected = 0;
                }
                if self.drives.is_empty() {
                    self.selected = 0;
                }

                // Kick off background AI prediction for all drives
                self.run_ai_predictions();
            }
            Err(e) => {
                self.drives.clear();
                self.last_error = Some(e);
            }
        }
        self.sync_telemetry();
    }

    /// Spawn a background thread that calls /predict for every drive.
    /// Results are written into `self.ai_results` (Arc<Mutex<…>>) so the UI
    /// thread can read them safely on the next frame.
    fn run_ai_predictions(&mut self) {
        if self.drives.is_empty() {
            return;
        }

        self.ai_loading = true;
        let drives = self.drives.clone();
        let results_arc = Arc::clone(&self.ai_results);

        std::thread::spawn(move || {
            let mut map: HashMap<String, AiResult> = HashMap::new();
            for drive in &drives {
                if let Some(result) = ai_client::predict(drive) {
                    map.insert(drive.dev.clone(), result);
                }
            }
            // Replace old results atomically
            if let Ok(mut guard) = results_arc.lock() {
                *guard = map;
            }
        });

        self.ai_loading = false;
    }

    /// Updates CPU and GPU temperature readings using external commands.
    fn update_system_temps(&mut self) {
        // Parse CPU temperature from lm-sensors output
        if let Ok(output) = Command::new("sensors").output() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                let temp_re = Regex::new(r"\+([0-9]+(?:\.[0-9]+)?)°C").unwrap();
                let mut temps: Vec<f32> = Vec::new();

                for line in text.lines() {
                    let lower = line.to_lowercase();
                    if lower.contains("tctl")
                        || lower.contains("tdie")
                        || lower.contains("package")
                        || lower.contains("core")
                    {
                        if let Some(caps) = temp_re.captures(line) {
                            if let Some(m) = caps.get(1) {
                                if let Ok(v) = m.as_str().parse::<f32>() {
                                    temps.push(v);
                                }
                            }
                        }
                    }
                }

                if !temps.is_empty() {
                    self.cpu_temp = Some(temps.iter().sum::<f32>() / temps.len() as f32);
                }
            }
        }

        // Parse GPU temperature from nvidia-smi
        if let Ok(output) = Command::new("nvidia-smi")
            .args(&["--query-gpu=temperature.gpu", "--format=csv,noheader,nounits"])
            .output()
        {
            if let Ok(text) = String::from_utf8(output.stdout) {
                if let Ok(temp) = text.trim().parse::<f32>() {
                    self.gpu_temp = Some(temp);
                }
            }
        }

        // Refresh firewall status from local backends (ufw, nftables, iptables).
        self.firewall = scan_firewall();

        // Parse RX packet counters from /proc/net/dev.
        self.incoming_packets = None;
        self.blocked_packets = None;
        self.approved_packets = None;

        if let Ok(text) = std::fs::read_to_string("/proc/net/dev") {
            let mut incoming_total: u64 = 0;
            let mut blocked_total: u64 = 0;

            for line in text.lines().skip(2) {
                let mut parts = line.split(':');
                let iface = match parts.next() {
                    Some(v) => v.trim(),
                    None => continue,
                };

                if iface == "lo" {
                    continue;
                }

                let data = match parts.next() {
                    Some(v) => v,
                    None => continue,
                };

                let cols: Vec<&str> = data.split_whitespace().collect();
                if cols.len() < 4 {
                    continue;
                }

                let rx_packets = cols[1].parse::<u64>().unwrap_or(0);
                let rx_errs = cols[2].parse::<u64>().unwrap_or(0);
                let rx_drop = cols[3].parse::<u64>().unwrap_or(0);

                incoming_total = incoming_total.saturating_add(rx_packets);
                blocked_total = blocked_total.saturating_add(rx_errs.saturating_add(rx_drop));
            }

            self.incoming_packets = Some(incoming_total);
            self.blocked_packets = Some(blocked_total);
            self.approved_packets = Some(incoming_total.saturating_sub(blocked_total));
        }
        self.sync_telemetry();
    }

    fn sync_telemetry(&self) {
        let mut snapshot = self.telemetry_data.lock().unwrap();
        snapshot.drives = self.drives.iter().map(|a| (**a).clone()).collect();
        snapshot.cpu_temp = self.cpu_temp;
        snapshot.gpu_temp = self.gpu_temp;
        snapshot.incoming_packets = self.incoming_packets;
        snapshot.blocked_packets = self.blocked_packets;
        snapshot.approved_packets = self.approved_packets;

        let Some(telemetry_config) = self.telemetry_config.as_ref() else {
            return;
        };

        // Debug: Print telemetry data
        println!("Telemetry synced:");
        println!("  Drives: {}", snapshot.drives.len());
        println!("  CPU Temp: {:?}", snapshot.cpu_temp);
        println!("  GPU Temp: {:?}", snapshot.gpu_temp);
        println!("  Incoming packets: {:?}", snapshot.incoming_packets);
        println!("  Blocked packets: {:?}", snapshot.blocked_packets);
        println!("  Approved packets: {:?}", snapshot.approved_packets);

        // Send telemetry data to Cloudflare Worker via POST in a background thread.
        let snapshot_clone = (*snapshot).clone();
        let endpoint = telemetry_config.endpoint.clone();
        let key_b64 = telemetry_config.key_b64.clone();
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            let payload = json!({"telemetry": snapshot_clone});
            let plaintext = serde_json::to_string(&payload).unwrap();

            let encryptor = Encryptor::from_env();
            let encrypted_body = encryptor.encrypt(plaintext.as_bytes());

            let request = client.post(&endpoint)
                .header("Authorization", format!("Bearer {}", key_b64))
                .body(encrypted_body);
            match request.send() {
                Ok(response) => {
                    if response.status().is_success() {
                        println!("Telemetry sent successfully");
                    } else {
                        eprintln!("Failed to send telemetry: HTTP {}", response.status());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to send telemetry: {}", e);
                }
            }
        });
    }

    /// Triggers a manual refresh of disk data and system temperatures.
    fn manual_refresh(&mut self) {
        self.refresh();
        self.update_system_temps();
        self.last_refresh = Instant::now();
    }

    /// Spawn a background thread that sends the user's NLP question to /ask.
    /// Result is stored in `self.nlp_response` for the UI to display.
    fn submit_nlp_question(&mut self, disk: Arc<DiskInfo>) {
        if self.nlp_input.trim().is_empty() || self.nlp_loading {
            return;
        }

        self.nlp_loading = true;
        let question = self.nlp_input.clone();
        let response_arc = Arc::clone(&self.nlp_response);

        // Get any existing AI result for this drive to provide extra context
        let ai_result_clone: Option<AiResult> = self
            .ai_results
            .lock()
            .ok()
            .and_then(|g| g.get(&disk.dev).cloned());

        std::thread::spawn(move || {
            let answer = ai_client::ask(&question, &disk, ai_result_clone.as_ref());
            if let Ok(mut guard) = response_arc.lock() {
                *guard = Some(
                    answer.unwrap_or_else(|| {
                        "AI service is not running. Start it with: bash ai_service/start.sh".to_string()
                    }),
                );
            }
        });

    }
}

impl eframe::App for AppState {
    /// Main UI update function called every frame.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request repaint every second to keep UI responsive
        ctx.request_repaint_after(Duration::from_secs(1));

        // Clear nlp_loading flag once we receive a response
        if self.nlp_loading {
            if let Ok(guard) = self.nlp_response.lock() {
                if guard.is_some() {
                    self.nlp_loading = false;
                }
            }
        }

        // Check if it's time for automatic refresh
        if self.last_refresh.elapsed() >= self.refresh_interval {
            self.refresh();
            self.update_system_temps();
            self.last_refresh = Instant::now();
        }

        // LEFT SIDEBAR: Drive list
        egui::SidePanel::left("drive_panel")
            .resizable(false)
            .exact_width(180.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("Storage").size(18.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let refresh_btn = egui::Button::new(
                            egui::RichText::new("Refresh").size(12.0)
                        )
                        .frame(true);

                        if ui.add(refresh_btn).on_hover_text("Refresh").clicked() {
                            self.manual_refresh();
                        }
                    });
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                // Snapshot AI results for display
                let ai_snapshot: HashMap<String, AiResult> = self
                    .ai_results
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();

                for (i, d) in self.drives.iter().enumerate() {
                    let is_selected = self.selected == i;

                    let frame = if is_selected {
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(220, 235, 255))
                            .stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(70, 130, 220)))
                            .rounding(8.0)
                            .inner_margin(12.0)
                    } else {
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(250, 250, 250))
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                            .rounding(8.0)
                            .inner_margin(12.0)
                    };

                    let response = frame.show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&d.dev)
                                    .strong()
                                    .size(14.0)
                            );
                            ui.add_space(2.0);

                            if let Some(model) = &d.model {
                                ui.label(
                                    egui::RichText::new(model)
                                        .size(11.0)
                                        .color(egui::Color32::from_gray(100))
                                );
                            }

                            ui.add_space(4.0);

                            ui.horizontal(|ui| {
                                let (color, text) = match d.health_percent {
                                    Some(p) if p > 84 => (egui::Color32::from_rgb(0, 160, 0), format!("{}%", p)),
                                    Some(p) if p >= 50 => (egui::Color32::from_rgb(220, 150, 0), format!("{}%", p)),
                                    Some(p) => (egui::Color32::from_rgb(200, 30, 30), format!("{}%", p)),
                                    None => (egui::Color32::GRAY, "?".to_string()),
                                };

                                ui.label(egui::RichText::new("●").color(color).size(12.0));
                                ui.label(egui::RichText::new(text).size(11.0));

                                if let Some(temp) = d.temp_c {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(
                                            egui::RichText::new(format!("{}°C", temp))
                                                .size(11.0)
                                                .color(egui::Color32::from_gray(100))
                                        );
                                    });
                                }
                            });

                            // Show AI label badge in the sidebar card
                            if let Some(ai) = ai_snapshot.get(&d.dev) {
                                ui.add_space(4.0);
                                let (badge_color, badge_text) = ai_label_style(&ai.label);
                                egui::Frame::none()
                                    .fill(badge_color)
                                    .rounding(4.0)
                                    .inner_margin(egui::vec2(6.0, 2.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(badge_text)
                                                .color(egui::Color32::WHITE)
                                                .size(10.0)
                                                .strong()
                                        );
                                    });
                            }
                        });
                    });

                    if response.response.interact(egui::Sense::click()).clicked() {
                        self.selected = i;
                        // Clear previous NLP response when switching drives
                        if let Ok(mut g) = self.nlp_response.lock() {
                            *g = None;
                        }
                    }

                    ui.add_space(8.0);
                }

                if let Some(err) = &self.last_error {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.colored_label(egui::Color32::RED, err);
                }
            });

        // CENTRAL PANEL: Main content area with drive details
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(245, 247, 250)))
            .show(ctx, |ui| {
                if self.drives.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.heading("No drives detected");
                            ui.add_space(8.0);
                            ui.label("Make sure you have smartctl installed and run with sudo");
                            if let Some(err) = &self.last_error {
                                ui.add_space(6.0);
                                ui.label(format!("Last error: {}", err));
                            }
                        });
                    });
                    return;
                }

                let di = self.drives[self.selected].clone();
                let dev_path = di.dev.clone();

                // Snapshot AI result for this drive
                let ai_result: Option<AiResult> = self
                    .ai_results
                    .lock()
                    .ok()
                    .and_then(|g| g.get(&dev_path).cloned());

                // Snapshot NLP response
                let nlp_response: Option<String> = self
                    .nlp_response
                    .lock()
                    .ok()
                    .and_then(|g| g.clone());

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(20.0);

                    // ---- Header Card ----
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        egui::Frame::none()
                            .fill(egui::Color32::WHITE)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(230)))
                            .rounding(12.0)
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 40.0);

                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.heading(egui::RichText::new(
                                            di.model.as_deref().unwrap_or("Unknown Drive")
                                        ).size(22.0));

                                        ui.add_space(4.0);

                                        ui.horizontal(|ui| {
                                            if let Some(cap) = &di.capacity_str {
                                                ui.label(egui::RichText::new(cap).size(16.0).color(egui::Color32::from_gray(100)));
                                                ui.label(egui::RichText::new("•").color(egui::Color32::from_gray(150)));
                                            }
                                            if let Some(protocol) = &di.protocol {
                                                ui.label(egui::RichText::new(protocol).size(16.0).color(egui::Color32::from_gray(100)));
                                                ui.label(egui::RichText::new("•").color(egui::Color32::from_gray(150)));
                                            }
                                            if let Some(dtype) = &di.device_type {
                                                ui.label(egui::RichText::new(dtype).size(16.0).color(egui::Color32::from_gray(100)));
                                            }
                                        });
                                    });

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let (health_color, health_text) = match di.health_percent {
                                            Some(p) if p > 84 => (egui::Color32::from_rgb(16, 185, 129), "Good"),
                                            Some(p) if p >= 50 => (egui::Color32::from_rgb(245, 158, 11), "Warning"),
                                            Some(_) => (egui::Color32::from_rgb(239, 68, 68), "Critical"),
                                            None => (egui::Color32::from_gray(150), "Unknown"),
                                        };

                                        egui::Frame::none()
                                            .fill(health_color)
                                            .rounding(8.0)
                                            .inner_margin(egui::vec2(20.0, 10.0))
                                            .show(ui, |ui| {
                                                ui.vertical_centered(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(health_text)
                                                            .color(egui::Color32::WHITE)
                                                            .size(14.0)
                                                            .strong()
                                                    );
                                                    if let Some(p) = di.health_percent {
                                                        ui.label(
                                                            egui::RichText::new(format!("{}%", p))
                                                                .color(egui::Color32::WHITE)
                                                                .size(28.0)
                                                                .strong()
                                                        );
                                                    }
                                                });
                                            });
                                    });
                                });
                            });
                        ui.add_space(20.0);
                    });

                    ui.add_space(15.0);

                    // ---- Partition Table ----
                    if !di.partitions.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            egui::Frame::none()
                                .fill(egui::Color32::WHITE)
                                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                                .rounding(10.0)
                                .inner_margin(15.0)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width() - 40.0);

                                    ui.label(egui::RichText::new("Partitions").size(14.0).strong());
                                    ui.add_space(8.0);

                                    egui::Grid::new("part_grid")
                                        .striped(true)
                                        .spacing([25.0, 10.0])
                                        .show(ui, |ui| {
                                            let total_cols = 7.0;
                                            let col_width = ui.available_width() / total_cols;

                                            for header in &["Partition", "Mount point", "Type", "Total", "Used", "Free", "Free%"] {
                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(*header).strong().size(11.0));
                                            }
                                            ui.end_row();

                                            for part in &di.partitions {
                                                let partition_name =
                                                    part.mount_point.rsplit('/').next().unwrap_or(&part.mount_point).to_string();

                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(partition_name).size(11.0));

                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(&part.mount_point).size(11.0));

                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(&part.fs_type).size(11.0));

                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(format!("{:.1} GB", part.total_gb)).size(11.0));

                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(format!("{:.1} GB", part.used_gb)).size(11.0));

                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(format!("{:.1} GB", part.free_gb)).size(11.0));

                                                let free_pct = 100.0 - part.used_percent;
                                                let color = if free_pct < 10.0 {
                                                    egui::Color32::from_rgb(239, 68, 68)
                                                } else if free_pct < 25.0 {
                                                    egui::Color32::from_rgb(245, 158, 11)
                                                } else {
                                                    egui::Color32::from_rgb(34, 197, 94)
                                                };

                                                ui.set_min_width(col_width);
                                                ui.colored_label(color, egui::RichText::new(format!("{:.1}%", free_pct)).size(11.0));

                                                ui.end_row();
                                            }
                                        });
                                });
                            ui.add_space(20.0);
                        });

                        ui.add_space(12.0);
                    }

                    // ---- Drive Information Card ----
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        egui::Frame::none()
                            .fill(egui::Color32::WHITE)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                            .rounding(10.0)
                            .inner_margin(15.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 40.0);

                                ui.label(egui::RichText::new("Drive Information").size(14.0).strong());
                                ui.add_space(8.0);

                                egui::Grid::new("info_grid")
                                    .striped(true)
                                    .spacing([15.0, 6.0])
                                    .show(ui, |ui| {
                                        for header in &["Serial no.", "Firmware", "Type"] {
                                            ui.label(egui::RichText::new(*header).strong().size(11.0));
                                        }
                                        ui.end_row();

                                        ui.label(egui::RichText::new(di.serial.as_deref().unwrap_or("--")).size(11.0));
                                        ui.label(egui::RichText::new(di.firmware.as_deref().unwrap_or("--")).size(11.0));
                                        ui.label(egui::RichText::new(di.device_type.as_deref().unwrap_or("--")).size(11.0));
                                        ui.end_row();
                                    });
                            });
                        ui.add_space(20.0);
                    });

                    ui.add_space(12.0);

                    // ---- Statistics Cards ----
                    let card_width = 283.0;
                    let card_spacing = 11.0;
                    let card_height = 75.0;

                    // Row 1: Temperature readings
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);

                        stat_card(ui, card_width, card_height, "SSD Temperature",
                            &di.temp_c.map(|t| format!("{}°C", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(59, 130, 246));

                        ui.add_space(card_spacing);

                        stat_card(ui, card_width, card_height, "CPU Temp",
                            &self.cpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(139, 92, 246));

                        ui.add_space(card_spacing);

                        stat_card(ui, card_width, card_height, "GPU Temp",
                            &self.gpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(236, 72, 153));
                    });

                    ui.add_space(10.0);

                    // Row 2: Data usage
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);

                        stat_card(ui, card_width, card_height, "Data written",
                            &di.data_written_tb.map(|t| format!("{:.1} TB", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(34, 197, 94));

                        ui.add_space(card_spacing);

                        stat_card(ui, card_width, card_height, "Data read",
                            &di.data_read_tb.map(|t| format!("{:.1} TB", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(251, 146, 60));

                        ui.add_space(card_spacing);

                        stat_card(ui, card_width, card_height, "Power on hours",
                            &di.power_on_hours.map(|h| h.to_string()).unwrap_or("--".into()),
                            egui::Color32::from_rgb(168, 85, 247));
                    });

                    ui.add_space(10.0);

                    // Row 3: Firewall summary
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);

                        let fw_status = if self.firewall.enabled { "Active" } else { "Inactive" };
                        let fw_status_color = if self.firewall.enabled {
                            egui::Color32::from_rgb(34, 197, 94)
                        } else {
                            egui::Color32::from_rgb(239, 68, 68)
                        };

                        // Current firewall activation status
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            &format!("Firewall ({})", self.firewall.backend),
                            fw_status,
                            fw_status_color,
                        );

                        ui.add_space(card_spacing);

                        // Default input policy (when detectable)
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Default input policy",
                            &self.firewall.default_input_policy,
                            egui::Color32::from_rgb(59, 130, 246),
                        );

                        ui.add_space(card_spacing);

                        // Total active rules parsed from backend output
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Rules loaded",
                            &self.firewall.rules_count.to_string(),
                            egui::Color32::from_rgb(139, 92, 246),
                        );
                    });

                    ui.add_space(10.0);

                    // Row 4: Packet counters requested by user.
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);

                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Incoming packets",

                            &self.incoming_packets.map(|v| v.to_string()).unwrap_or("--".into()),
                            egui::Color32::from_rgb(59, 130, 246));

                        ui.add_space(card_spacing);

                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Blocked packets",
                            &self.blocked_packets.map(|v| v.to_string()).unwrap_or("--".into()),
                            egui::Color32::from_rgb(239, 68, 68),
                        );

                        ui.add_space(card_spacing);

                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Approved packets",
                            &self.approved_packets.map(|v| v.to_string()).unwrap_or("--".into()),
                            egui::Color32::from_rgb(34, 197, 94),
                        );

                    });

                    ui.add_space(10.0);

                    // Detailed firewall block to expose our custom module output.
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        egui::Frame::none()
                            .fill(egui::Color32::WHITE)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                            .rounding(10.0)
                            .inner_margin(15.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 40.0);

                                ui.label(egui::RichText::new("Firewall Details").size(14.0).strong());
                                ui.add_space(8.0);

                                egui::Grid::new("firewall_grid")
                                    .striped(true)
                                    .spacing([15.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new("Backend").strong().size(11.0));
                                        ui.label(egui::RichText::new("Status").strong().size(11.0));
                                        ui.label(egui::RichText::new("Default output policy").strong().size(11.0));
                                        ui.end_row();

                                        ui.label(egui::RichText::new(&self.firewall.backend).size(11.0));
                                        ui.label(egui::RichText::new(&self.firewall.status_line).size(11.0));
                                        ui.label(egui::RichText::new(&self.firewall.default_output_policy).size(11.0));
                                        ui.end_row();
                                    });

                                ui.add_space(6.0);
                                let open_ports_text = if self.firewall.open_ports.is_empty() {
                                    "none".to_string()
                                } else {
                                    self.firewall.open_ports.join(", ")
                                };

                                ui.label(
                                    egui::RichText::new(format!("Open allowed ports: {}", open_ports_text))
                                        .size(11.0)
                                        .color(egui::Color32::from_gray(90)),
                                );

                                if let Some(note) = &self.firewall.note {
                                    ui.add_space(4.0);
                                    ui.colored_label(
                                        egui::Color32::from_rgb(245, 158, 11),
                                        egui::RichText::new(format!("Note: {}", note)).size(11.0),
                                    );
                                }
                            });
                        ui.add_space(20.0);
                    });

                    ui.add_space(15.0);

                    // Telemetry debug section
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        egui::Frame::none()
                            .fill(egui::Color32::WHITE)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                            .rounding(10.0)
                            .inner_margin(15.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 40.0);

                                ui.label(egui::RichText::new("Telemetry Data").size(14.0).strong());
                                ui.add_space(8.0);

                                // Display telemetry snapshot
                                if let Ok(snapshot) = self.telemetry_data.try_lock() {
                                    egui::Grid::new("telemetry_grid")
                                        .striped(true)
                                        .spacing([15.0, 6.0])
                                        .show(ui, |ui| {
                                            ui.label(egui::RichText::new("Drives").strong().size(11.0));
                                            ui.label(egui::RichText::new(format!("{}", snapshot.drives.len())).size(11.0));
                                            ui.end_row();

                                            ui.label(egui::RichText::new("CPU Temp").strong().size(11.0));
                                            ui.label(egui::RichText::new(snapshot.cpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("--".to_string())).size(11.0));
                                            ui.end_row();

                                            ui.label(egui::RichText::new("GPU Temp").strong().size(11.0));
                                            ui.label(egui::RichText::new(snapshot.gpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("--".to_string())).size(11.0));
                                            ui.end_row();

                                            ui.label(egui::RichText::new("Incoming Packets").strong().size(11.0));
                                            ui.label(egui::RichText::new(snapshot.incoming_packets.map(|v| v.to_string()).unwrap_or("--".to_string())).size(11.0));
                                            ui.end_row();

                                            ui.label(egui::RichText::new("Blocked Packets").strong().size(11.0));
                                            ui.label(egui::RichText::new(snapshot.blocked_packets.map(|v| v.to_string()).unwrap_or("--".to_string())).size(11.0));
                                            ui.end_row();

                                            ui.label(egui::RichText::new("Packets Allowed").strong().size(11.0));
                                            ui.label(egui::RichText::new(snapshot.approved_packets.map(|v| v.to_string()).unwrap_or("--".to_string())).size(11.0));
                                            ui.end_row();
                                        });
                                } else {
                                    ui.label("Telemetry data locked");
                                }
                            });
                        ui.add_space(20.0);
                    });

                    ui.add_space(15.0);

                    // ================================================================ //
                    // AI HEALTH INSIGHT PANEL
                    // ================================================================ //
                    render_ai_panel(ui, ai_result.as_ref());

                    ui.add_space(12.0);

                    // ================================================================ //
                    // NLP Q&A PANEL
                    // ================================================================ //
                    let mut submit_drive: Option<Arc<DiskInfo>> = None;
                    render_nlp_panel(
                        ui,
                        &mut self.nlp_input,
                        self.nlp_loading,
                        nlp_response.as_deref(),
                        || {
                            // Called when the user clicks "Ask"
                            di.clone()
                        },
                        |drive| {
                            submit_drive = Some(drive);
                        },
                    );
                    if let Some(drive) = submit_drive {
                        self.submit_nlp_question(drive);
                    }

                    ui.add_space(20.0);
                });
            });
    }
}

// ------------------------------------------------------------------ //
// Helper: AI label colour and display text
// ------------------------------------------------------------------ //

/// Returns the fill colour and display text for an AI label.
fn ai_label_style(label: &str) -> (egui::Color32, &'static str) {
    match label {
        "healthy"   => (egui::Color32::from_rgb(16, 185, 129),  "✓ Healthy"),
        "watchlist" => (egui::Color32::from_rgb(245, 158, 11),  "⚠ Watchlist"),
        "risky"     => (egui::Color32::from_rgb(239, 68, 68),   "✗ Risky"),
        _           => (egui::Color32::from_gray(150),          "? Unknown"),
    }
}

// ------------------------------------------------------------------ //
// Render: AI Health Insight Panel
// ------------------------------------------------------------------ //

/// Renders the AI Health Insight card below the statistics rows.
/// Shows a label badge, confidence bar, one-sentence reason, and next step.
/// If no AI result is available yet, shows a loading/unavailable message.
fn render_ai_panel(ui: &mut egui::Ui, ai: Option<&AiResult>) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        egui::Frame::none()
            .fill(egui::Color32::WHITE)
            .stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(99, 102, 241)))
            .rounding(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 40.0);

                // Section header
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("AI Health Insight")
                            .size(15.0)
                            .strong()
                            .color(egui::Color32::from_rgb(99, 102, 241))
                    );
                });

                ui.add_space(10.0);

                match ai {
                    None => {
                        ui.label(
                            egui::RichText::new(
                                "AI service is not running.  Start it with:  bash ai_service/start.sh"
                            )
                            .size(12.0)
                            .color(egui::Color32::from_gray(130))
                        );
                    }
                    Some(r) => {
                        ui.horizontal(|ui| {
                            // Label badge
                            let (badge_color, badge_text) = ai_label_style(&r.label);
                            egui::Frame::none()
                                .fill(badge_color)
                                .rounding(6.0)
                                .inner_margin(egui::vec2(14.0, 6.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(badge_text)
                                            .color(egui::Color32::WHITE)
                                            .size(13.0)
                                            .strong()
                                    );
                                });

                            ui.add_space(12.0);

                            // Confidence bar
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(
                                        format!("Confidence: {:.0}%", r.confidence * 100.0)
                                    )
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(100))
                                );
                                ui.add_space(4.0);
                                let bar_width = 160.0;
                                let bar_height = 8.0;
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(bar_width, bar_height),
                                    egui::Sense::hover(),
                                );
                                // Background track
                                ui.painter().rect_filled(
                                    rect,
                                    4.0,
                                    egui::Color32::from_gray(220),
                                );
                                // Filled portion
                                let filled = egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(bar_width * r.confidence, bar_height),
                                );
                                let (bar_col, _) = ai_label_style(&r.label);
                                ui.painter().rect_filled(filled, 4.0, bar_col);
                            });
                        });

                        ui.add_space(10.0);

                        // Reason
                        ui.label(
                            egui::RichText::new(&r.reason)
                                .size(12.0)
                                .color(egui::Color32::from_gray(50))
                        );

                        ui.add_space(6.0);

                        // Next step (highlighted box)
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(238, 242, 255))
                            .rounding(6.0)
                            .inner_margin(egui::vec2(10.0, 6.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("Note: ")
                                            .size(12.0)
                                            .strong()
                                    );
                                    ui.label(
                                        egui::RichText::new(&r.next_step)
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(67, 56, 202))
                                    );
                                });
                            });
                    }
                }
            });
        ui.add_space(20.0);
    });
}

// ------------------------------------------------------------------ //
// Render: NLP Q&A Panel
// ------------------------------------------------------------------ //

/// Renders the "Ask a Question" panel with a text input, Ask button,
/// optional loading indicator, and the AI's answer.
///
/// `get_drive_fn` is called to retrieve the current drive when the user
/// clicks "Ask".  `submit_fn` is the callback that actually fires the
/// background HTTP call.
fn render_nlp_panel(
    ui: &mut egui::Ui,
    input: &mut String,
    loading: bool,
    response: Option<&str>,
    get_drive_fn: impl Fn() -> Arc<DiskInfo>,
    mut submit_fn: impl FnMut(Arc<DiskInfo>),
) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        egui::Frame::none()
            .fill(egui::Color32::WHITE)
            .stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(16, 185, 129)))
            .rounding(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 40.0);

                // Section header
                ui.label(
                    egui::RichText::new("Ask a Question")
                        .size(15.0)
                        .strong()
                        .color(egui::Color32::from_rgb(5, 150, 105))
                );

                ui.add_space(8.0);

                // Hint text
                ui.label(
                    egui::RichText::new("e.g.  \"Is this drive safe?\"  •  \"Why is it risky?\"  •  \"What does unsafe shutdown mean?\"")
                        .size(11.0)
                        .color(egui::Color32::from_gray(140))
                );

                ui.add_space(8.0);

                // Input row: text box + Ask button
                ui.horizontal(|ui| {
                    let text_edit = egui::TextEdit::singleline(input)
                        .hint_text("Type your question here…")
                        .desired_width(ui.available_width() - 80.0);
                    let te_response = ui.add(text_edit);

                    ui.add_space(8.0);

                    // Submit on Enter key or button click
                    let enter_pressed = te_response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    let ask_btn = egui::Button::new(
                        egui::RichText::new(if loading { "Thinking..." } else { "Submit" })
                            .size(13.0)
                            .strong()
                    )
                    .min_size(egui::vec2(60.0, 30.0))
                    .fill(egui::Color32::from_rgb(16, 185, 129));

                    let clicked = ui.add_enabled(!loading, ask_btn).clicked();

                    if (clicked || enter_pressed) && !loading && !input.trim().is_empty() {
                        let drive = get_drive_fn();
                        submit_fn(drive);
                    }
                });

                // Show answer
                if let Some(ans) = response {
                    ui.add_space(10.0);
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(240, 253, 250))
                        .rounding(8.0)
                        .inner_margin(egui::vec2(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new(ans)
                                    .size(12.5)
                                    .color(egui::Color32::from_rgb(6, 78, 59))
                            );
                        });
                } else if loading {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Thinking…")
                            .size(12.0)
                            .color(egui::Color32::from_gray(130))
                    );
                }
            });
        ui.add_space(20.0);
    });
}