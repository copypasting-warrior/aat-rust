# AI Feature Documentation

## Overview

The AI layer adds two capabilities on top of the existing SMART data scan:

1. **Drive Health Prediction** (deep learning layer): classifies each drive as Healthy, Watchlist, or Risky with a confidence score.
2. **Natural Language Q&A** (NLP layer): lets the user type a plain-English question about the selected drive and get a short, clear answer.

Both features live in the `ai_service/` folder, which is a small Python microservice that the Rust app calls over HTTP after every scan.

---

## Deep Learning Component

### What it does

After the SMART scan finishes, the Rust app serialises the key SMART fields into a JSON payload and sends it to the `/predict` endpoint. The model returns:

| Field        | Type    | Description                                     |
|--------------|---------|-------------------------------------------------|
| `label`      | string  | `"healthy"`, `"watchlist"`, or `"risky"`        |
| `confidence` | float   | 0.0 to 1.0 — how certain the model is           |
| `reason`     | string  | One sentence explaining the prediction          |
| `next_step`  | string  | Recommended action for the user                 |

### Input features

The following SMART fields are fed to the model:

| Feature                | Source in DiskInfo       |
|------------------------|--------------------------|
| Temperature (C)        | `temp_c`                 |
| Health percent         | `health_percent`         |
| Power-on hours         | `power_on_hours`         |
| Unsafe shutdowns       | `unsafe_shutdowns`       |
| Reallocated sectors    | `smart_attributes` (ID 5)|
| Pending sectors        | `smart_attributes` (ID 197)|
| Uncorrectable sectors  | `smart_attributes` (ID 198)|

NVMe drives report health via "Percentage Used" which the Rust scanner converts to a 0-100 health percent. SATA drives expose individual SMART attributes in the attributes table.

### Label thresholds (Phase 1 — rule-based)

The current model uses research-backed thresholds derived from the Backblaze drive failure dataset (2016-2023):

| Indicator              | Watchlist threshold | Risky threshold  |
|------------------------|---------------------|------------------|
| Temperature            | >= 55 C             | >= 65 C          |
| Health percent         | <= 60%              | <= 30%           |
| Power-on hours         | >= 26,280 (3 yr)    | >= 43,800 (5 yr) |
| Reallocated sectors    | >= 1                | >= 100           |
| Pending sectors        | >= 1                | >= 50            |
| Uncorrectable sectors  | >= 1                | >= 10            |
| Unsafe shutdowns       | >= 20               | >= 100           |

The final label is determined by the worst single indicator. Confidence is computed as the fraction of indicators that agree with the final label, normalised to [0.5, 0.99].

### Upgrading to a real neural network

The `DriveHealthModel` class in `ai_service/model.py` is designed to be a drop-in replacement:

```
# Current
class DriveHealthModel:
    def predict(self, data: dict) -> PredictionResult: ...

# Future — replace _compute_scores() with actual inference
import onnxruntime
model = onnxruntime.InferenceSession("drive_health.onnx")
```

The `predict()` method signature, input dict schema, and `PredictionResult` dataclass do not need to change. The FastAPI route (`/predict` in `main.py`) and the Rust client code (`ai_client.rs`) remain untouched.

A suitable public training dataset is the Backblaze Hard Drive Stats dataset: https://www.backblaze.com/b2/hard-drive-test-data.html

---

## NLP Component

### What it does

The `/ask` endpoint accepts a user question and the current drive's SMART summary. It returns a short answer (1-3 sentences) in plain English. The engine uses the **Google Gemini API** (`gemini-1.5-flash`) to generate intelligent, context-aware responses, restricted exclusively to storage/hardware questions to prevent off-topic prompts.

To use the AI NLP feature, configure your `.env` file first.

### Supported question intents

The engine detects the intent from the question text using a keyword map and routes to a purpose-built answer function:

| Intent              | Example questions                                      |
|---------------------|--------------------------------------------------------|
| Unsafe shutdown     | "What does unsafe shutdown mean?"                      |
| Backup              | "Should I back up my data?"                            |
| Replace             | "When should I replace this drive?"                    |
| Risky reason        | "Why is this drive risky?"                             |
| Safe status         | "Is this drive safe to use?"                           |
| Temperature         | "Is the temperature too high?"                         |
| Reallocated sectors | "What are reallocated sectors?"                        |
| Pending sectors     | "What does pending sector mean?"                       |
| Health score        | "What is the health percentage?"                       |
| Drive age           | "How old is this drive?"                               |
| General (fallback)  | Any other question — returns a drive status summary    |

The SMART context (including the AI prediction label and reason from a prior `/predict` call if available) is passed along so answers are specific to the current drive.

### Environment Variables Configuration

The system requires an API key to run the Gemini generative model.
1. Copy the example environment file:
   ```bash
   cp ai_service/.env.example ai_service/.env
   ```
2. Edit `ai_service/.env` and replace `your_gemini_api_key_here` with your actual Google Gemini API key.

### Troubleshooting API Issues
- **Empty or missing key**: If `GEMINI_API_KEY` is not present, the Q&A Answer will immediately return an error message prompting you to set the `.env` variable.
- **Model not found / Deprecated**: The system defaults to `gemini-2.5-flash`. If Google deprecates this model, you can safely update the model string parameter in `ai_service/nlp.py`.
- **UI Error Messages**: Because exceptions are handled gracefully in the Python server and returned as HTTP 200 JSON responses, any Gemini API errors (rate limits, service unavailable) will appear directly and safely in the GUI question-answering panel without crashing the application.

---

## Data Flow

```
[SMART scan completes]
        |
        v
Rust: ai_client::predict(disk)
        |
        | HTTP POST /predict  { health_percent, temp_c, power_on_hours, ... }
        v
Python: DriveHealthModel.predict(data)
        |
        | { label, confidence, reason, next_step }
        v
Rust: stores AiResult in HashMap<dev_path, AiResult>
        |
        v
UI: renders AI Health Insight card
        |
[User types a question]
        |
        v
Rust: ai_client::ask(question, disk, ai_result)
        |
        | HTTP POST /ask  { question, smart: { ...fields + ai_label + ai_reason } }
        v
Python: DriveNlpEngine.answer(question, smart)
        |
        | { answer: "..." }
        v
UI: renders answer in the Q&A panel
```

---

## File Reference

| File                          | Purpose                                                  |
|-------------------------------|----------------------------------------------------------|
| `ai_service/model.py`         | DriveHealthModel — classification logic                  |
| `ai_service/nlp.py`           | DriveNlpEngine — question-answering logic                |
| `ai_service/main.py`          | FastAPI app — /predict and /ask routes                   |
| `ai_service/requirements.txt` | Python dependencies (fastapi, uvicorn, pydantic)         |
| `ai_service/start.sh`         | Shell script to install deps and start the service       |
| `ai_service/test_model.py`    | Pytest unit tests for the classifier (29 tests total)    |
| `ai_service/test_nlp.py`      | Pytest unit tests for the NLP engine                     |
| `src/ai_client.rs`            | Rust HTTP client for calling the service                 |
| `src/models/mod.rs`           | AiResult struct (shared between ai_client and app)       |
| `src/gui/app.rs`              | AI panel and Q&A panel UI (render_ai_panel, render_nlp_panel) |

---

## Running on Linux

### Step 1 — Install Python dependencies

Python 3.10 or newer is required.

```bash
cd ai_service
pip install -r requirements.txt
```

### Step 2 — Start the AI service

```bash
bash ai_service/start.sh
```

This installs dependencies and starts the service on `http://127.0.0.1:5001`. Keep this terminal open.

To verify it is running:

```bash
curl http://127.0.0.1:5001/health
# Expected: {"status":"ok"}
```

### Step 3 — Build and run the Rust app

```bash
# Build
cargo build --release

# Run (requires root for smartctl)
sudo ./target/release/ssd_info_cli
```

The app will automatically detect the AI service and display the AI Health Insight panel and Q&A box for each drive. If the service is not running, those panels show a notice and the rest of the app works normally.

### Step 4 — Test the AI service manually

```bash
# Health prediction
curl -X POST http://127.0.0.1:5001/predict \
  -H "Content-Type: application/json" \
  -d '{
    "health_percent": 95,
    "temp_c": 38,
    "power_on_hours": 1200,
    "unsafe_shutdowns": 2,
    "reallocated_sectors": 0,
    "pending_sectors": 0,
    "uncorrectable_sectors": 0
  }'

# NLP question
curl -X POST http://127.0.0.1:5001/ask \
  -H "Content-Type: application/json" \
  -d '{
    "question": "Is this drive safe to use?",
    "smart": {
      "health_percent": 95,
      "temp_c": 38,
      "ai_label": "healthy",
      "model": "Samsung 970 EVO"
    }
  }'
```

### Step 5 — Run the unit tests

```bash
cd ai_service
python -m pytest test_model.py test_nlp.py -v
```

All 29 tests should pass.

### Stopping the service

Press `Ctrl+C` in the terminal where `uvicorn` is running. The Rust app will continue to work — the AI panels will show a "service not running" message.
