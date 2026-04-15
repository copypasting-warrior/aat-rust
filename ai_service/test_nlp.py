"""Unit tests for DriveNlpEngine."""

import pytest
from nlp import DriveNlpEngine


@pytest.fixture
def nlp():
    return DriveNlpEngine()


def make_smart(label="healthy", health=96, temp=38, hours=1200,
               realloc=0, pending=0, uncorr=0, unsafe=2):
    return {
        "model": "TestDrive SSD 512GB",
        "health_percent": health,
        "temp_c": temp,
        "power_on_hours": hours,
        "reallocated_sectors": realloc,
        "pending_sectors": pending,
        "uncorrectable_sectors": uncorr,
        "unsafe_shutdowns": unsafe,
        "ai_label": label,
        "ai_confidence": 0.92,
        "ai_reason": "Drive is operating within all normal parameters.",
    }


# ------------------------------------------------------------------ #
# Intent detection & answer content
# ------------------------------------------------------------------ #

def test_safe_question_healthy(nlp):
    ans = nlp.answer("Is this drive safe to use?", make_smart(label="healthy"))
    assert "safe" in ans.lower() or "healthy" in ans.lower()


def test_safe_question_risky(nlp):
    ans = nlp.answer("Is this SSD safe?", make_smart(label="risky"))
    assert "not safe" in ans.lower() or "back up" in ans.lower()


def test_temperature_question_normal(nlp):
    ans = nlp.answer("What is the temperature?", make_smart(temp=38))
    assert "38" in ans


def test_temperature_question_high(nlp):
    ans = nlp.answer("Is it too hot?", make_smart(temp=66))
    assert "high" in ans.lower() or "critical" in ans.lower()


def test_reallocated_question(nlp):
    ans = nlp.answer("What are reallocated sectors?", make_smart(realloc=5))
    assert "realloc" in ans.lower()
    assert "5" in ans


def test_reallocated_none(nlp):
    ans = nlp.answer("Are there reallocated sectors?", make_smart(realloc=0))
    assert "none" in ans.lower() or "no" in ans.lower()


def test_unsafe_shutdown_question(nlp):
    ans = nlp.answer("What does unsafe shutdown mean?", make_smart(unsafe=50))
    assert "power" in ans.lower() or "shutdown" in ans.lower()
    assert "50" in ans


def test_health_score_question(nlp):
    ans = nlp.answer("What is the health percentage?", make_smart(health=87))
    assert "87" in ans


def test_drive_age_question(nlp):
    ans = nlp.answer("How old is this drive?", make_smart(hours=8760))
    assert "8,760" in ans or "8760" in ans


def test_backup_question_risky(nlp):
    ans = nlp.answer("Should I back up my data?", make_smart(label="risky"))
    assert "back" in ans.lower()


def test_replace_question_healthy(nlp):
    ans = nlp.answer("Do I need to replace this drive?", make_smart(label="healthy"))
    assert "no" in ans.lower() or "not" in ans.lower()


def test_risky_reason_question(nlp):
    ans = nlp.answer("Why is this drive risky?",
                     make_smart(label="risky", realloc=200, health=20))
    assert "realloc" in ans.lower() or "health" in ans.lower()


# ------------------------------------------------------------------ #
# Edge cases
# ------------------------------------------------------------------ #

def test_empty_question_returns_string(nlp):
    """Should return a general summary without crashing."""
    ans = nlp.answer("", make_smart())
    assert isinstance(ans, str)
    assert len(ans) > 0


def test_missing_smart_fields(nlp):
    """Handles drives with no SMART data gracefully."""
    ans = nlp.answer("Is it safe?", {})
    assert isinstance(ans, str)


def test_missing_temp_returns_message(nlp):
    smart = make_smart()
    smart["temp_c"] = None
    ans = nlp.answer("What is the temperature?", smart)
    assert "not available" in ans.lower()


def test_missing_hours_returns_message(nlp):
    smart = make_smart()
    smart["power_on_hours"] = None
    ans = nlp.answer("How old is this drive?", smart)
    assert "not available" in ans.lower()
