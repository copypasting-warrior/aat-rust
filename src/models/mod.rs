// Data models for disk information and SMART attributes
use serde::Serialize;
/// Represents a single SMART attribute from disk diagnostics.
/// Contains the attribute ID, name, values, and health status.

#[derive(Clone, Debug, Serialize)]
pub struct SmartAttribute {
    /// Attribute identifier number
    #[allow(dead_code)]
    pub id: String,
    /// Human-readable attribute name
    #[allow(dead_code)]
    pub name: String,
    /// Current value of the attribute
    #[allow(dead_code)]
    pub current: String,
    /// Worst value ever recorded for this attribute
    #[allow(dead_code)]
    pub worst: String,
    /// Failure threshold for this attribute
    #[allow(dead_code)]
    pub threshold: String,
    /// Raw value as reported by the drive
    #[allow(dead_code)]
    pub raw_value: String,
    /// Health status based on threshold comparison
    #[allow(dead_code)]
    pub status: AttributeStatus,
}

/// Health status classification for SMART attributes.
/// Determines if an attribute is healthy, approaching failure, or critical.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum AttributeStatus {
    /// Attribute is within normal operating parameters
    Good,
    /// Attribute is approaching threshold (within 10 units)
    Warning,
    /// Attribute has exceeded failure threshold
    Critical,
}

/// Information about a single partition on a disk.
/// Includes mount point, filesystem type, and space usage statistics.
#[derive(Clone, Debug, Serialize)]
pub struct PartitionInfo {
    /// Directory where the partition is mounted (e.g., /home)
    pub mount_point: String,
    /// Filesystem type (e.g., ext4, ntfs)
    pub fs_type: String,
    /// Total capacity in gigabytes
    pub total_gb: f64,
    /// Used space in gigabytes
    pub used_gb: f64,
    /// Available free space in gigabytes
    pub free_gb: f64,
    /// Percentage of space currently used (0-100)
    pub used_percent: f64,
}

/// Complete information about a disk drive.
/// Aggregates device details, SMART data, temperature, and partition information.
#[derive(Clone, Debug, Serialize)]
pub struct DiskInfo {
    /// Device path (e.g., /dev/nvme0n1, /dev/sda)
    pub dev: String,
    /// Drive type hint (e.g., NVMe, SATA, HDD)
    pub kind: String,
    /// Manufacturer model name
    pub model: Option<String>,
    /// Serial number for unique identification
    pub serial: Option<String>,
    /// Firmware version string
    pub firmware: Option<String>,
    /// Raw capacity in bytes
    pub capacity: Option<f64>,
    /// Formatted capacity string (e.g., "500 GB")
    pub capacity_str: Option<String>,
    /// Overall health percentage (0-100, higher is better)
    pub health_percent: Option<u8>,
    /// Current temperature in Celsius
    pub temp_c: Option<i32>,
    /// Total data written in terabytes
    pub data_written_tb: Option<f64>,
    /// Total data read in terabytes
    pub data_read_tb: Option<f64>,
    /// Total hours the drive has been powered on
    pub power_on_hours: Option<u64>,
    /// Number of power on/off cycles
    pub power_cycles: Option<u64>,
    /// Count of unsafe shutdowns (power loss)
    pub unsafe_shutdowns: Option<u64>,
    /// Rotational speed in RPM (None for SSDs)
    pub rotation_rpm: Option<u64>,
    /// Communication protocol (NVMe, ATA)
    pub protocol: Option<String>,
    /// Device classification (SSD or HDD)
    pub device_type: Option<String>,
    /// List of SMART attributes reported by the drive
    pub smart_attributes: Vec<SmartAttribute>,
    /// List of partitions on this drive
    pub partitions: Vec<PartitionInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TelemetrySnapshot {
    pub drives: Vec<DiskInfo>,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub incoming_packets: Option<u64>,
    pub blocked_packets: Option<u64>,
    pub approved_packets: Option<u64>,
}

impl DiskInfo {
    /// Creates an empty DiskInfo structure with default values.
    /// Only the device path is required; all other fields are None or empty.
    pub fn empty(dev: impl Into<String>) -> Self {
        Self {
            dev: dev.into(),
            kind: String::from("Unknown"),
            model: None,
            serial: None,
            firmware: None,
            capacity: None,
            capacity_str: None,
            health_percent: None,
            temp_c: None,
            data_written_tb: None,
            data_read_tb: None,
            power_on_hours: None,
            power_cycles: None,
            unsafe_shutdowns: None,
            rotation_rpm: None,
            protocol: None,
            device_type: None,
            smart_attributes: vec![],
            partitions: vec![],
        }
    }
}

/// Result returned by the Python AI service after analysing a drive.
/// Carries the health classification, confidence, and human-readable
/// explanation so the UI can display them directly.
#[derive(Clone, Debug)]
pub struct AiResult {
    /// Health label: "healthy", "watchlist", or "risky"
    pub label: String,
    /// Confidence in the prediction, 0.0 (low) to 1.0 (high)
    pub confidence: f32,
    /// One-sentence explanation of the prediction
    pub reason: String,
    /// Recommended next action for the user
    pub next_step: String,
}