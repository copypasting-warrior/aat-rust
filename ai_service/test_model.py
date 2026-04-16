"""Unit tests for DriveHealthModel."""

import pytest
from model import DriveHealthModel, PredictionResult


@pytest.fixture
def model():
    return DriveHealthModel()


def healthy_drive():
    return {
        "temp_c": 38,
        "health_percent": 97,
        "power_on_hours": 1200,
        "reallocated_sectors": 0,
        "pending_sectors": 0,
        "uncorrectable_sectors": 0,
        "unsafe_shutdowns": 3,
    }


def watchlist_drive():
    return {
        "temp_c": 57,           # elevated temperature
        "health_percent": 72,   # moderate health
        "power_on_hours": 28000, # 3+ years
        "reallocated_sectors": 3,
        "pending_sectors": 1,
        "uncorrectable_sectors": 0,
        "unsafe_shutdowns": 25,
    }


def risky_drive():
    return {
        "temp_c": 66,          # critical temperature
        "health_percent": 22,  # near end of life
        "power_on_hours": 45000,
        "reallocated_sectors": 150,
        "pending_sectors": 60,
        "uncorrectable_sectors": 12,
        "unsafe_shutdowns": 120,
    }


# ------------------------------------------------------------------ #
# Label tests
# ------------------------------------------------------------------ #

def test_healthy_label(model):
    result = model.predict(healthy_drive())
    assert result.label == "healthy"


def test_watchlist_label(model):
    result = model.predict(watchlist_drive())
    assert result.label == "watchlist"


def test_risky_label(model):
    result = model.predict(risky_drive())
    assert result.label == "risky"


# ------------------------------------------------------------------ #
# Confidence tests
# ------------------------------------------------------------------ #

def test_confidence_range(model):
    for data in [healthy_drive(), watchlist_drive(), risky_drive()]:
        result = model.predict(data)
        assert 0.0 <= result.confidence <= 1.0, (
            f"Confidence out of range: {result.confidence}"
        )


def test_healthy_high_confidence(model):
    result = model.predict(healthy_drive())
    assert result.confidence >= 0.70, (
        f"Expected >= 0.70 for healthy drive, got {result.confidence}"
    )


def test_risky_high_confidence(model):
    result = model.predict(risky_drive())
    assert result.confidence >= 0.80, (
        f"Expected >= 0.80 for risky drive, got {result.confidence}"
    )


# ------------------------------------------------------------------ #
# Output content tests
# ------------------------------------------------------------------ #

def test_reason_and_next_step_not_empty(model):
    for data in [healthy_drive(), watchlist_drive(), risky_drive()]:
        result = model.predict(data)
        assert result.reason.strip(), "reason should not be empty"
        assert result.next_step.strip(), "next_step should not be empty"


def test_risky_next_step_mentions_backup(model):
    result = model.predict(risky_drive())
    assert "back" in result.next_step.lower() or "replac" in result.next_step.lower()


# ------------------------------------------------------------------ #
# Edge case tests
# ------------------------------------------------------------------ #

def test_empty_payload(model):
    """Should not raise — return watchlist with low confidence."""
    result = model.predict({})
    assert isinstance(result, PredictionResult)
    assert result.label in ("healthy", "watchlist", "risky")


def test_partial_payload(model):
    """Only temperature provided — should still return a result."""
    result = model.predict({"temp_c": 45})
    assert result.label in ("healthy", "watchlist", "risky")


def test_none_values(model):
    """None values should be treated like missing data."""
    result = model.predict({
        "temp_c": None,
        "health_percent": None,
        "reallocated_sectors": None,
    })
    assert result.label in ("healthy", "watchlist", "risky")


def test_zero_health_is_risky(model):
    data = healthy_drive()
    data["health_percent"] = 0
    result = model.predict(data)
    assert result.label == "risky"


def test_high_realloc_overrides_good_temp(model):
    """Even a cool drive is risky if reallocated sectors are high."""
    data = healthy_drive()
    data["reallocated_sectors"] = 200
    result = model.predict(data)
    assert result.label == "risky"
