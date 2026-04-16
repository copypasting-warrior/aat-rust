# SSD Health Checker

A modern GUI application for monitoring SSD and HDD health using SMART data. Built with Rust and egui for a fast, native experience.

The app includes an in-house firewall scanner module in src/firewall that reads local firewall state directly (UFW, firewalld, nftables, or iptables) without using a firewall crate.

## Screenshots

![SSD Health Checker](image.png)

## Prerequisites

### Required System Packages

**Ubuntu/Debian:**
```bash
sudo apt-get update
sudo apt-get install smartmontools lm-sensors

sudo chmod +s /usr/sbin/smartctl
```

**Fedora/RHEL/CentOS:**
```bash
sudo dnf install smartmontools lm-sensors
```

**Arch Linux:**
```bash
sudo pacman -S smartmontools lm-sensors
```

**openSUSE:**
```bash
sudo zypper install smartmontools sensors
```

For GPU temperature monitoring

**NVIDIA GPU:**
```bash
sudo apt-get install nvidia-utils
```

For firewall status monitoring

```bash
# Most Debian/Ubuntu systems already include one of these, but install if needed
sudo apt-get install ufw nftables iptables
```

### Rust Toolchain

You need Rust 1.75 or newer:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Installation

### From Source

1. Clone the repository:
```bash
git clone https://github.com/yourusername/aat-rust.git
cd aat-rust
```

2. Build the application:
```bash
cargo build --release
```

3. Run the application:
```bash
sudo ./target/release/ssd_info_cli
```

## Troubleshooting

### No drives detected

1. Ensure you're running with sudo:
   ```bash
   sudo ssd_info_cli
   ```

2. Verify smartctl is installed:
   ```bash
   which smartctl
   ```

3. Check if smartctl can detect your drives:
   ```bash
   sudo smartctl --scan
   ```

### Temperature not showing

**CPU Temperature:**
- Install lm-sensors: `sudo apt-get install lm-sensors`
- Run sensor detection: `sudo sensors-detect` (answer YES to all)
- Test: `sensors`

**GPU Temperature:**
- For NVIDIA: Install nvidia-utils
- For AMD: Temperature detection may vary by GPU model

### Permission errors

The application needs root access to read SMART data. Always run with `sudo`.

## Building from Source

### Dependencies

The following Rust crates are used:
- `eframe` - GUI framework
- `egui` - Immediate mode GUI
- `regex` - Pattern matching for parsing smartctl output
- `sysinfo` - System information and partition data
- `image` - Image loading support
- `nix` - Unix system calls

## Firewall Module (Custom)

Firewall support is implemented with local Rust code in src/firewall/mod.rs.

### Design goals

- Avoid external firewall crates.
- Work on common Linux setups.
- Keep behavior explicit and easy to debug.
- Fail safely when permissions are limited.

### Detection pipeline

The scanner runs in this order:
1. UFW
2. firewalld
3. nftables
4. iptables
5. systemd active-service fallback

The first backend that provides usable data is shown in the UI.

### Data model

Each refresh produces one FirewallSnapshot with:
- backend: selected firewall backend name.
- enabled: active or inactive status flag.
- default_input_policy: default input policy (when detectable).
- default_output_policy: default output policy (when detectable).
- rules_count: number of parsed rule lines.
- open_ports: allowed destination ports parsed from backend output.
- status_line: short backend status text.
- note: optional warning, usually for permission limits.

### Command sources

- UFW: ufw status numbered
- firewalld: firewall-cmd --state, --get-default-zone, --list-ports
- nftables: nft list ruleset
- iptables: iptables -S
- fallback: systemctl is-active ufw, nftables, firewalld

### What each firewall card means

- Firewall (backend): active or inactive based on parsed backend state.
- Default input policy: backend default for incoming traffic. If unknown, shows --.
- Rules loaded: count of parsed rules in the selected backend.

### Packet cards (network interface counters)

These three cards come from /proc/net/dev, not from firewall logs:

- Incoming packets: sum of RX packets on non-loopback interfaces.
- Blocked packets: RX errors plus RX drops.
- Approved packets: incoming packets minus blocked packets.

Important note: blocked packets here are interface-level receive failures. This is not the same as firewall-denied packet counters in all firewall engines.

### Why values can look unexpected

- Default input policy is --:
   The backend did not expose policy in current command output, or the process lacked privileges for full rule inspection.

- Rules loaded is 0:
   The detected backend may be active with no parsed filter rules in visible output, or rule listing was restricted by permissions.

- Blocked packets is 0:
   This is common on healthy links because RX error/drop counters are often zero.

### Permissions and accuracy

Firewall rule inspection may require root privileges.

If the app is run without elevated permissions:
- backend service may still be detected as active,
- detailed rules or policies can be unavailable,
- note field in Firewall Details will explain the limitation.

For best fidelity during firewall inspection, run with sudo.

### Development

```bash
# Build release (default make target)
make

# Clean build artifacts
make clean

# Run in debug mode
make run

# Run in debug mode
sudo cargo run

# Run with logging
sudo RUST_LOG=debug cargo run

# Build release version
cargo build --release

# Run tests
cargo test
```

## Configuration

The application auto-detects drives in `/dev/` and automatically refreshes every 5 seconds. Firewall data is refreshed on the same interval. No configuration file is needed.

Telemetry configuration is read from `.env` if present. Set `TELEMETRY_KEY` and `TELEMETRY_ENDPOINT` there or export them in your shell before launching the app.

## License

This project is licensed under the GNU General Public License v3.0 - see the LICENSE file for details.
