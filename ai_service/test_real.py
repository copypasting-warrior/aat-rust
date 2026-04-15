import os
from nlp import DriveNlpEngine

engine = DriveNlpEngine()

smart = {
    "model": "Test NVMe SSD 1TB",
    "health_percent": 95,
    "temp_c": 45,
    "power_on_hours": 1200,
    "unsafe_shutdowns": 5,
    "ai_label": "healthy",
    "ai_reason": "Everything is fine."
}

print("Testing Gemini integration...\n")

q1 = "Is my drive safe to use?"
ans1 = engine.answer(q1, smart)
print(f"Q: {q1}")
print(f"A: {ans1}\n")

q2 = "What is the recipe for chocolate cake?"
ans2 = engine.answer(q2, smart)
print(f"Q: {q2}")
print(f"A: {ans2}\n")

q3 = "What does unsafe shutdown mean for this drive?"
ans3 = engine.answer(q3, smart)
print(f"Q: {q3}")
print(f"A: {ans3}\n")
