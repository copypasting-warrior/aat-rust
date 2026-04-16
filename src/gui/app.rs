// Main application state and UI rendering logic for the SSD Health Checker

use crate::firewall::{scan_firewall, FirewallSnapshot};
// Import disk scanning functionality
use crate::gui::{disk_scanner::scan_disks, stat_card};
// Import disk information models
use crate::models::{DiskInfo, TelemetrySnapshot};
// Import egui for UI rendering
use eframe::egui;
// Regex for parsing system command output
use regex::Regex;
// Command execution for reading system temperatures
use std::process::Command;
// Arc for thread-safe reference counting
use std::sync::Arc;
// Duration and Instant for time-based operations
use std::time::{Duration, Instant};

use std::sync::Mutex;
use serde_json::json;
use std::env;

use crate::crypto::Encryptor;
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

    telemetry_data: Arc<Mutex<TelemetrySnapshot>>,
}

impl AppState {
    /// Creates a new application state instance.
    /// Sets light theme, performs initial data collection, and starts refresh timer.
    ///
    /// # Arguments
    /// * `cc` - eframe creation context containing egui context
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
            telemetry_data,
        };

        // Perform initial data collection
        s.refresh();
        s.update_system_temps();

        s
    }

    /// Refreshes the disk list by calling scan_disks.
    /// On success, updates the drives vector and adjusts selection if needed.
    /// On error, clears the drives vector and stores the error message.
    fn refresh(&mut self) {
        self.last_error = None;
        match scan_disks() {
            Ok(list) => {
                // Wrap each DiskInfo in Arc for efficient sharing
                self.drives = list.into_iter().map(Arc::new).collect();

                // Clamp selection to valid range if drives changed
                if !self.drives.is_empty() && self.selected >= self.drives.len() {
                    self.selected = 0;
                }

                // Reset selection if no drives found
                if self.drives.is_empty() {
                    self.selected = 0;
                }
            }
            Err(e) => {
                // Clear drives and store error for display
                self.drives.clear();
                self.last_error = Some(e);
            }
        }
        self.sync_telemetry();
    }

    /// Updates CPU and GPU temperature readings using external commands.
    /// Parses output from 'sensors' for CPU temperature and 'nvidia-smi' for GPU.
    /// Failures are silently ignored, leaving temperature fields as None.
    fn update_system_temps(&mut self) {
        // Parse CPU temperature from lm-sensors output
        if let Ok(output) = Command::new("sensors").output() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                // Regex to match temperature values like +47.0°C or +47°C
                let temp_re = Regex::new(r"\+([0-9]+(?:\.[0-9]+)?)°C").unwrap();
                let mut temps: Vec<f32> = Vec::new();

                // Look for common CPU temperature labels
                for line in text.lines() {
                    let lower = line.to_lowercase();
                    // Filter for lines containing CPU-related keywords
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

                // Compute average of all found temperature values
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

        // Debug: Print telemetry data
        println!("Telemetry synced:");
        println!("  Drives: {}", snapshot.drives.len());
        println!("  CPU Temp: {:?}", snapshot.cpu_temp);
        println!("  GPU Temp: {:?}", snapshot.gpu_temp);
        println!("  Incoming packets: {:?}", snapshot.incoming_packets);
        println!("  Blocked packets: {:?}", snapshot.blocked_packets);
        println!("  Approved packets: {:?}", snapshot.approved_packets);

        // Send telemetry data to Cloudflare Worker via POST in a background thread
        let snapshot_clone = (*snapshot).clone();
        let endpoint = env::var("TELEMETRY_ENDPOINT").unwrap_or_else(|_| "https://ssd-telemetry.ssd-telemetry.workers.dev".to_string());
        let key_b64 = env::var("TELEMETRY_KEY").ok();
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            let payload = json!({"telemetry": snapshot_clone});
            let plaintext = serde_json::to_string(&payload).unwrap();

            if let Some(k) = &key_b64 {
                // Encrypt the payload using the existing crypto module
                let encryptor = Encryptor::from_env();
                let encrypted_body = encryptor.encrypt(plaintext.as_bytes());

                let request = client.post(&endpoint)
                    .header("Authorization", format!("Bearer {}", k))
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
            } else {
                // No key, send plain JSON (for testing, but Worker will fail)
                let request = client.post(&endpoint).json(&payload);
                match request.send() {
                    Ok(response) => {
                        if response.status().is_success() {
                            println!("Telemetry sent successfully (no encryption)");
                        } else {
                            eprintln!("Failed to send telemetry: HTTP {}", response.status());
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to send telemetry: {}", e);
                    }
                }
            }
        });
    }

    /// Triggers a manual refresh of disk data and system temperatures.
    /// Also updates the last_refresh timestamp to reset the auto-refresh timer.
    fn manual_refresh(&mut self) {
        self.refresh();
        self.update_system_temps();
        self.last_refresh = Instant::now();
    }
}

impl eframe::App for AppState {
    /// Main UI update function called every frame.
    /// Handles automatic refresh, renders sidebar with drive list, and main content area.
    ///
    /// # Arguments
    /// * `ctx` - egui context for rendering
    /// * `_frame` - eframe frame (unused)
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request repaint every second to keep UI responsive
        ctx.request_repaint_after(Duration::from_secs(1));

        // Check if it's time for automatic refresh
        if self.last_refresh.elapsed() >= self.refresh_interval {
            self.refresh();
            self.update_system_temps();
            self.last_refresh = Instant::now();
        }

        // LEFT SIDEBAR: Drive list with modern design similar to reference
        egui::SidePanel::left("drive_panel")
            .resizable(false)
            .exact_width(180.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);

                // Header with title and refresh button
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("Storage").size(18.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Refresh button with hover tooltip
                        let refresh_btn = egui::Button::new(
                            egui::RichText::new("🔄").size(14.0)
                        )
                        .frame(false);
                        
                        if ui.add(refresh_btn).on_hover_text("Refresh").clicked() {
                            self.manual_refresh();
                        }
                    });
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                // Render each drive as a selectable card
                for (i, d) in self.drives.iter().enumerate() {
                    let is_selected = self.selected == i;

                    // Change appearance based on selection state
                    let frame = if is_selected {
                        // Selected: light blue background with blue border
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(220, 235, 255))
                            .stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(70, 130, 220)))
                            .rounding(8.0)
                            .inner_margin(12.0)
                    } else {
                        // Unselected: light gray background with subtle border
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(250, 250, 250))
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                            .rounding(8.0)
                            .inner_margin(12.0)
                    };

                    // Render drive card showing device path, model, health, and temperature
                    let response = frame.show(ui, |ui| {
                        ui.vertical(|ui| {
                            // Display device path (e.g., /dev/nvme0n1)
                            ui.label(
                                egui::RichText::new(&d.dev)
                                    .strong()
                                    .size(14.0)
                            );
                            ui.add_space(2.0);

                            // Display truncated model name if available
                            if let Some(model) = &d.model {
                                ui.label(
                                    egui::RichText::new(model)
                                        .size(11.0)
                                        .color(egui::Color32::from_gray(100))
                                );
                            }

                            ui.add_space(4.0);

                            // Health indicator and temperature display
                            ui.horizontal(|ui| {
                                // Health status with colored dot and percentage
                                let (color, text) = match d.health_percent {
                                    Some(p) if p > 84 => (egui::Color32::from_rgb(0, 160, 0), format!("{}%", p)),
                                    Some(p) if p >= 50 => (egui::Color32::from_rgb(220, 150, 0), format!("{}%", p)),
                                    Some(p) => (egui::Color32::from_rgb(200, 30, 30), format!("{}%", p)),
                                    None => (egui::Color32::GRAY, "?".to_string()),
                                };

                                ui.label(egui::RichText::new("●").color(color).size(12.0));
                                ui.label(egui::RichText::new(text).size(11.0));

                                // Temperature display on the right side
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
                        });
                    });

                    // Handle click to select this drive
                    if response.response.interact(egui::Sense::click()).clicked() {
                        self.selected = i;
                    }

                    ui.add_space(8.0);
                }

                // Display error message if present
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
                // Show helpful message if no drives detected
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

                // Get currently selected drive information
                let di = self.drives[self.selected].as_ref();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(20.0);

                    // Header Card with model info and health badge
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
                                    // Left side: Model and drive details
                                    ui.vertical(|ui| {
                                        ui.heading(egui::RichText::new(
                                            di.model.as_deref().unwrap_or("Unknown Drive")
                                        ).size(22.0));

                                        ui.add_space(4.0);

                                        // Drive details: capacity, protocol, type
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

                                    // Right side: Health badge
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

                    // Partition table showing mount points and space usage
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

                                    // Grid layout for partition data
                                    egui::Grid::new("part_grid")
                                        .striped(true)
                                        .spacing([25.0, 10.0])
                                        .show(ui, |ui| {
                                            // Calculate column widths
                                            let total_cols = 7.0;
                                            let col_width = ui.available_width() / total_cols;

                                            // Table headers
                                            for header in &["Partition", "Mount point", "Type", "Total", "Used", "Free", "Free%"] {
                                                ui.set_min_width(col_width);
                                                ui.label(egui::RichText::new(*header).strong().size(11.0));
                                            }
                                            ui.end_row();

                                            // Each partition row with usage statistics
                                            for part in &di.partitions {
                                                // Extract partition name from mount point
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

                                                // Calculate free percentage and color code it
                                                let free_pct = 100.0 - part.used_percent;
                                                let color = if free_pct < 10.0 {
                                                    egui::Color32::from_rgb(239, 68, 68)  // Red: critical
                                                } else if free_pct < 25.0 {
                                                    egui::Color32::from_rgb(245, 158, 11)  // Orange: warning
                                                } else {
                                                    egui::Color32::from_rgb(34, 197, 94)   // Green: good
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

                    // Drive information card showing serial, firmware, and type
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
                                        // Headers
                                        for header in &["Serial no.", "Firmware", "Type"] {
                                            ui.label(egui::RichText::new(*header).strong().size(11.0));
                                        }
                                        ui.end_row();

                                        // Values
                                        ui.label(egui::RichText::new(di.serial.as_deref().unwrap_or("--")).size(11.0));
                                        ui.label(egui::RichText::new(di.firmware.as_deref().unwrap_or("--")).size(11.0));
                                        ui.label(egui::RichText::new(di.device_type.as_deref().unwrap_or("--")).size(11.0));
                                        ui.end_row();
                                    });
                            });
                        ui.add_space(20.0);
                    });

                    ui.add_space(12.0);

                    // Statistics cards displayed in a 3-column grid
                    let card_width = 283.0;
                    let card_spacing = 11.0;
                    let card_height = 75.0;

                    // Row 1: Temperature readings
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);

                        // SSD temperature from SMART data
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "SSD Temperature",
                            &di.temp_c.map(|t| format!("{}°C", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(59, 130, 246),
                        );

                        ui.add_space(card_spacing);

                        // CPU temperature from sensors command
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "CPU Temp",
                            &self.cpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(139, 92, 246),
                        );

                        ui.add_space(card_spacing);

                        // GPU temperature from nvidia-smi
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "GPU Temp",
                            &self.gpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(236, 72, 153),
                        );
                    });

                    ui.add_space(10.0);

                    // Row 2: Data usage statistics
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);

                        // Total data written to drive
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Data written",
                            &di.data_written_tb.map(|t| format!("{:.1} TB", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(34, 197, 94),
                        );

                        ui.add_space(card_spacing);

                        // Total data read from drive
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Data read",
                            &di.data_read_tb.map(|t| format!("{:.1} TB", t)).unwrap_or("--".into()),
                            egui::Color32::from_rgb(251, 146, 60),
                        );

                        ui.add_space(card_spacing);

                        // Total hours drive has been powered on
                        stat_card(
                            ui,
                            card_width,
                            card_height,
                            "Power on hours",
                            &di.power_on_hours.map(|h| h.to_string()).unwrap_or("--".into()),
                            egui::Color32::from_rgb(168, 85, 247),
                        );
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
                            egui::Color32::from_rgb(59, 130, 246),
                        );

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
                });
            });
    }
}