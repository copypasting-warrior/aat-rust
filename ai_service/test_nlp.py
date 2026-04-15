"""Unit tests for DriveNlpEngine."""

import pytest
from unittest.mock import patch, MagicMock
import os
from nlp import DriveNlpEngine

@pytest.fixture
def mock_env():
    with patch.dict(os.environ, {"GEMINI_API_KEY": "test_api_key"}):
        yield

@pytest.fixture
def mock_genai():
    with patch('nlp.genai.GenerativeModel') as MockModel:
        mock_instance = MockModel.return_value
        mock_response = MagicMock()
        mock_response.text = "Mocked answer from Gemini."
        mock_instance.generate_content.return_value = mock_response
        yield mock_instance

@pytest.fixture
def mock_genai_error():
    with patch('nlp.genai.GenerativeModel') as MockModel:
        mock_instance = MockModel.return_value
        mock_instance.generate_content.side_effect = Exception("API rate limit exceeded")
        yield mock_instance

@pytest.fixture
def nlp(mock_env, mock_genai):
    return DriveNlpEngine()

def make_smart(label="healthy", health=96, temp=38, hours=1200):
    return {
        "model": "TestDrive SSD 512GB",
        "health_percent": health,
        "temp_c": temp,
        "power_on_hours": hours,
        "ai_label": label,
        "ai_confidence": 0.92,
        "ai_reason": "Drive is operating within all normal parameters.",
    }

def test_missing_api_key():
    with patch.dict(os.environ, {}, clear=True):
        engine = DriveNlpEngine()
        ans = engine.answer("Is it safe?", make_smart())
        assert "GEMINI_API_KEY is not set" in ans

def test_answer_delegates_to_gemini(nlp, mock_genai):
    smart = make_smart()
    ans = nlp.answer("Is this drive safe?", smart)
    assert ans == "Mocked answer from Gemini."
    mock_genai.generate_content.assert_called_once()
    
    # Ensure context contains the question and smart info
    args, kwargs = mock_genai.generate_content.call_args
    context = args[0]
    assert "Is this drive safe?" in context
    assert "TestDrive SSD 512GB" in context

def test_answer_handles_api_error(mock_env, mock_genai_error):
    engine = DriveNlpEngine()
    ans = engine.answer("Is this safe?", make_smart())
    assert "API rate limit exceeded" in ans
    assert "error occurred" in ans.lower()
