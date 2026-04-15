"""
AI Microservice Entry Point
============================
FastAPI app exposing two endpoints for the aat-rust SSD Health Checker:

  POST /predict   —  deep learning health classification
  POST /ask       —  NLP question-answering

Run with:
    uvicorn main:app --host 127.0.0.1 --port 5001

Or use the provided start.sh script.
"""

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from typing import Optional

from model import DriveHealthModel
from nlp import DriveNlpEngine

# ------------------------------------------------------------------ #
# Application setup
# ------------------------------------------------------------------ #

app = FastAPI(
    title="Drive AI Service",
    description="SMART-data health prediction and NLP Q&A for SSD Health Checker",
    version="1.0.0",
)

# Instantiate singletons once at startup (thread-safe for read-only use)
_model = DriveHealthModel()
_nlp   = DriveNlpEngine()


# ------------------------------------------------------------------ #
# Request / response schemas
# ------------------------------------------------------------------ #

class SmartPayload(BaseModel):
    """
    SMART data fields sent by the Rust app after each scan.
    All fields are optional so the service degrades gracefully when
    a particular drive doesn't report a specific attribute.
    """
    # Device identification
    model: Optional[str]  = None
    dev:   Optional[str]  = None

    # Core health indicators
    health_percent:        Optional[float] = None
    temp_c:                Optional[float] = None
    power_on_hours:        Optional[float] = None
    unsafe_shutdowns:      Optional[float] = None
    power_cycles:          Optional[float] = None
    data_written_tb:       Optional[float] = None
    data_read_tb:          Optional[float] = None

    # SMART attribute-derived indicators
    reallocated_sectors:   Optional[float] = None
    pending_sectors:       Optional[float] = None
    uncorrectable_sectors: Optional[float] = None

    # AI results (populated by /predict, forwarded to /ask for context)
    ai_label:      Optional[str]   = None
    ai_confidence: Optional[float] = None
    ai_reason:     Optional[str]   = None


class AskPayload(BaseModel):
    """Payload for the NLP Q&A endpoint."""
    question: str
    smart:    SmartPayload


class PredictResponse(BaseModel):
    """Health prediction result returned to the Rust app."""
    label:      str    # "healthy" | "watchlist" | "risky"
    confidence: float  # 0.0 – 1.0
    reason:     str    # one-sentence explanation
    next_step:  str    # recommended action


class AskResponse(BaseModel):
    """NLP answer returned to the Rust app."""
    answer: str


# ------------------------------------------------------------------ #
# Endpoints
# ------------------------------------------------------------------ #

@app.get("/health")
def health_check():
    """Simple liveness check — returns 200 if the service is running."""
    return {"status": "ok"}


@app.post("/predict", response_model=PredictResponse)
def predict(payload: SmartPayload):
    """
    Classify the drive health based on SMART data.

    Accepts a SmartPayload JSON object and returns a health label,
    confidence score, human-readable reason, and next-step recommendation.
    """
    try:
        # Convert pydantic model to plain dict for the classifier
        data = payload.model_dump()
        result = _model.predict(data)
        return PredictResponse(
            label=result.label,
            confidence=result.confidence,
            reason=result.reason,
            next_step=result.next_step,
        )
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Prediction failed: {str(e)}")


@app.post("/ask", response_model=AskResponse)
def ask(payload: AskPayload):
    """
    Answer a plain-English question about the drive using SMART context.

    The SMART summary (including any AI prediction already attached) is
    used to build a context-aware, short answer.
    """
    if not payload.question.strip():
        raise HTTPException(status_code=400, detail="Question cannot be empty.")

    try:
        smart_dict = payload.smart.model_dump()
        answer = _nlp.answer(payload.question, smart_dict)
        return AskResponse(answer=answer)
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"NLP failed: {str(e)}")
