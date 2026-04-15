# SSD Health Checker

A modern GUI application for monitoring SSD and HDD health using SMART data. Built with Rust and egui for a fast, native experience. Includes an AI layer for drive health prediction and natural language question-answering.

## Screenshots

![SSD Health Checker](image.png)

## Features

- Scans NVMe and SATA drives using smartctl
- Displays real-time SMART attributes, temperature, health percentage, and partition usage
- Shows CPU and GPU temperatures alongside drive data
- Auto-refreshes every 5 seconds
- AI health prediction: classifies each drive as Healthy, Watchlist, or Risky with a confidence score
- NLP Q&A: ask plain-English questions about any drive and get short, clear answers

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

For GPU temperature monitoring (NVIDIA):
```bash
sudo apt-get install nvidia-utils
```

### Rust Toolchain

Rust 1.75 or newer:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Python (for AI features)

Python 3.10 or newer. The AI service is optional — the rest of the app works without it.

## Installation

### From Source

1. Clone the repository:
```bash
git clone https://github.com/yourusername/ssd_info_cli.git
cd ssd_info_cli
```

2. Build the application:
```bash
cargo build --release
```

3. Run the application:
```bash
sudo ./target/release/ssd_info_cli
```

## AI Features (Optional)

The AI layer runs as a separate Python service. Start it before launching the Rust app to enable health prediction and Q&A.

### Setup API Key
The NLP feature uses the Gemini API. To use it, simply copy the `.env.example` file to `.env`:
```bash
cp ai_service/.env.example ai_service/.env
```
Then, edit the `ai_service/.env` file and insert your Google Gemini API key: `GEMINI_API_KEY=your_gemini_api_key_here`.

### Start the AI service

```bash
bash ai_service/start.sh
```

This installs Python dependencies and starts the service on `http://127.0.0.1:5001`. Keep this terminal open.

Verify it is running:
```bash
curl http://127.0.0.1:5001/health
```

### What the AI adds

- **AI Health Insight panel**: shows a Healthy / Watchlist / Risky label, a confidence bar, a one-sentence reason, and a recommended next step — displayed below the statistics cards for the selected drive.
- **Ask a Question panel**: type any question about the drive (e.g. "Is this safe to use?", "What does unsafe shutdown mean?", "Should I back up?") and receive a plain-English answer using the current SMART data as context.

If the AI service is not running, both panels show a notice and the rest of the app is unaffected.

For full technical documentation on the AI components, see [AI_FEATURES.md](AI_FEATURES.md).

### Run AI unit tests

```bash
cd ai_service
pip install -r requirements.txt
python -m pytest test_model.py test_nlp.py -v
```

## Troubleshooting

### No drives detected

1. Ensure you are running with sudo:
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
- For NVIDIA: install nvidia-utils
- For AMD: temperature detection may vary by GPU model

### Permission errors

The application needs root access to read SMART data. Always run with `sudo`.

### AI service not connecting or returning API Errors

- Confirm the service is running: `curl http://127.0.0.1:5001/health`
- Check that port 5001 is not blocked by a firewall
- Restart the service: `bash ai_service/start.sh`
- **Missing API Key:** If the AI Q&A panel shows **"Error: GEMINI_API_KEY is not set..."**, make sure you copied `.env.example` to `.env` and set a valid Google Gemini API key.
- **API Rate Limits:** If you receive **"An error occurred..."** referring to rate limits, check your Google AI Studio quota limits.

## Building from Source

### Rust Dependencies

| Crate      | Purpose                                              |
|------------|------------------------------------------------------|
| `eframe`   | GUI framework                                        |
| `egui`     | Immediate mode GUI                                   |
| `regex`    | Pattern matching for parsing smartctl output         |
| `sysinfo`  | System information and partition data                |
| `image`    | Image loading support                                |
| `nix`      | Unix system calls                                    |
| `ureq`     | Synchronous HTTP client for calling the AI service   |

### Python Dependencies (ai_service/)

| Package    | Purpose                            |
|------------|------------------------------------|
| `fastapi`  | Web framework for the AI service   |
| `uvicorn`  | ASGI server                        |
| `pydantic` | Request/response schema validation |

### Development Commands

```bash
# Run in debug mode
sudo cargo run

# Run with logging
sudo RUST_LOG=debug cargo run

# Build release version
cargo build --release

# Run Rust tests
cargo test

# Run AI unit tests
cd ai_service && python -m pytest test_model.py test_nlp.py -v
```

## Configuration

The application auto-detects drives in `/dev/` and automatically refreshes every 5 seconds. The AI service URL defaults to `http://127.0.0.1:5001` and is defined in `src/ai_client.rs`. No configuration file is needed.

## License

This project is licensed under the GNU General Public License v3.0 — see the LICENSE file for details.