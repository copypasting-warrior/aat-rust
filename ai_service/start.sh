#!/usr/bin/env bash
# Start the Drive AI Service
# Usage:  bash start.sh
# The service listens on http://127.0.0.1:5001

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "Installing Python dependencies..."
pip install -r requirements.txt -q

echo "Starting Drive AI Service on http://127.0.0.1:5001 ..."
uvicorn main:app --host 127.0.0.1 --port 5001
