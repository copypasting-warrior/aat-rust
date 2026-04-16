"""
Drive Health Classification Model
==================================
Phase 1: Rule-based classifier using SMART thresholds from published SSD
health research (Backblaze failure studies, NVMe spec guidance).

Interface is identical to what a trained neural-network model would expose,
so this module can be swapped with a real torch/ONNX model later without
touching any other code.

Labels
------
- "healthy"   : drive is operating within all normal parameters
- "watchlist" : one or more indicators need monitoring
- "risky"     : drive shows signs of imminent or ongoing failure

Confidence
----------
Computed as a float [0.0, 1.0] reflecting how many signals agree.
"""

from dataclasses import dataclass


@dataclass
class PredictionResult:
    """Output from DriveHealthModel.predict()."""
    label: str          # "healthy" | "watchlist" | "risky"
    confidence: float   # 0.0 – 1.0
    reason: str         # short human-readable explanation (one sentence)
    next_step: str      # recommended user action


class DriveHealthModel:
    """
    Rule-based drive health predictor.

    Each rule produces a severity score (0 = ok, 1 = warning, 2 = critical).
    The overall label is determined by the highest severity and the ratio of
    triggered rules.

    To replace with a real model:
        1. Train a classifier (e.g. sklearn RandomForest or torch MLP) on a
           labeled SMART dataset (e.g. Backblaze data).
        2. Save to ONNX or pickle format.
        3. Replace `_compute_scores()` with actual model inference.
        4. Keep `predict()` signature unchanged.
    """

    # ------------------------------------------------------------------ #
    # Thresholds (tuned from Backblaze 2016–2023 failure reports)
    # ------------------------------------------------------------------ #
    TEMP_WARN   = 55    # °C — JEDEC SSD spec max is 70, watchlist at 55
    TEMP_CRIT   = 65    # °C

    HOURS_WARN  = 26_280   # ~3 years continuous use
    HOURS_CRIT  = 43_800   # ~5 years continuous use

    REALLOC_WARN = 1       # any reallocated sector is a warning sign
    REALLOC_CRIT = 100     # large counts indicate imminent failure

    PENDING_WARN = 1       # pending sectors = instability
    PENDING_CRIT = 50

    UNCORR_WARN  = 1       # uncorrectable errors = data-at-risk
    UNCORR_CRIT  = 10

    HEALTH_WARN  = 60      # health % (NVMe % used converted)
    HEALTH_CRIT  = 30

    UNSAFE_WARN  = 20      # unsafe shutdowns (power loss events)
    UNSAFE_CRIT  = 100

    def _get_attr(self, data: dict, key: str, default=None):
        """Safely extract a field, returning default if missing/None/empty."""
        val = data.get(key, default)
        if val is None or val == "" or val == "null":
            return default
        try:
            return float(val)
        except (ValueError, TypeError):
            return default

    def _compute_scores(self, data: dict) -> list[dict]:
        """
        Evaluate each SMART indicator and return a list of rule results.

        Each result is a dict:
          { "name": str, "severity": int (0/1/2), "detail": str }
        """
        scores = []

        # --- Temperature ---
        temp = self._get_attr(data, "temp_c")
        if temp is not None:
            if temp >= self.TEMP_CRIT:
                scores.append({"name": "temperature", "severity": 2,
                                "detail": f"drive temperature {temp:.0f}°C is critically high"})
            elif temp >= self.TEMP_WARN:
                scores.append({"name": "temperature", "severity": 1,
                                "detail": f"drive temperature {temp:.0f}°C is elevated"})
            else:
                scores.append({"name": "temperature", "severity": 0,
                                "detail": f"drive temperature {temp:.0f}°C is normal"})

        # --- Health Percent ---
        health = self._get_attr(data, "health_percent")
        if health is not None:
            if health <= self.HEALTH_CRIT:
                scores.append({"name": "health_percent", "severity": 2,
                                "detail": f"drive health is only {health:.0f}% — nearing end of life"})
            elif health <= self.HEALTH_WARN:
                scores.append({"name": "health_percent", "severity": 1,
                                "detail": f"drive health is {health:.0f}% — starting to age"})
            else:
                scores.append({"name": "health_percent", "severity": 0,
                                "detail": f"drive health is {health:.0f}%"})

        # --- Power-on Hours ---
        hours = self._get_attr(data, "power_on_hours")
        if hours is not None:
            if hours >= self.HOURS_CRIT:
                scores.append({"name": "power_on_hours", "severity": 2,
                                "detail": f"drive has {hours:.0f} power-on hours — beyond typical lifespan"})
            elif hours >= self.HOURS_WARN:
                scores.append({"name": "power_on_hours", "severity": 1,
                                "detail": f"drive has {hours:.0f} power-on hours — aging"})
            else:
                scores.append({"name": "power_on_hours", "severity": 0,
                                "detail": f"drive has {hours:.0f} power-on hours"})

        # --- Reallocated Sectors ---
        realloc = self._get_attr(data, "reallocated_sectors", 0)
        if realloc >= self.REALLOC_CRIT:
            scores.append({"name": "reallocated_sectors", "severity": 2,
                            "detail": f"{realloc:.0f} reallocated sectors detected — severe physical damage"})
        elif realloc >= self.REALLOC_WARN:
            scores.append({"name": "reallocated_sectors", "severity": 1,
                            "detail": f"{realloc:.0f} reallocated sector(s) detected — early wear"})
        else:
            scores.append({"name": "reallocated_sectors", "severity": 0,
                            "detail": "no reallocated sectors"})

        # --- Pending Sectors ---
        pending = self._get_attr(data, "pending_sectors", 0)
        if pending >= self.PENDING_CRIT:
            scores.append({"name": "pending_sectors", "severity": 2,
                            "detail": f"{pending:.0f} pending sectors — drive is unstable"})
        elif pending >= self.PENDING_WARN:
            scores.append({"name": "pending_sectors", "severity": 1,
                            "detail": f"{pending:.0f} pending sector(s) — possible instability"})
        else:
            scores.append({"name": "pending_sectors", "severity": 0,
                            "detail": "no pending sectors"})

        # --- Uncorrectable Errors ---
        uncorr = self._get_attr(data, "uncorrectable_sectors", 0)
        if uncorr >= self.UNCORR_CRIT:
            scores.append({"name": "uncorrectable_errors", "severity": 2,
                            "detail": f"{uncorr:.0f} uncorrectable errors — data may be at risk"})
        elif uncorr >= self.UNCORR_WARN:
            scores.append({"name": "uncorrectable_errors", "severity": 1,
                            "detail": f"{uncorr:.0f} uncorrectable error(s) detected"})
        else:
            scores.append({"name": "uncorrectable_errors", "severity": 0,
                            "detail": "no uncorrectable errors"})

        # --- Unsafe Shutdowns ---
        unsafe = self._get_attr(data, "unsafe_shutdowns", 0)
        if unsafe >= self.UNSAFE_CRIT:
            scores.append({"name": "unsafe_shutdowns", "severity": 2,
                            "detail": f"{unsafe:.0f} unsafe shutdowns — high stress history"})
        elif unsafe >= self.UNSAFE_WARN:
            scores.append({"name": "unsafe_shutdowns", "severity": 1,
                            "detail": f"{unsafe:.0f} unsafe shutdowns recorded"})
        else:
            scores.append({"name": "unsafe_shutdowns", "severity": 0,
                            "detail": f"{unsafe:.0f} unsafe shutdowns"})

        return scores

    def predict(self, data: dict) -> PredictionResult:
        """
        Predict drive health from a SMART data dictionary.

        Parameters
        ----------
        data : dict
            Keys expected:
              temp_c, health_percent, power_on_hours,
              reallocated_sectors, pending_sectors,
              uncorrectable_sectors, unsafe_shutdowns

        Returns
        -------
        PredictionResult
        """
        scores = self._compute_scores(data)

        if not scores:
            return PredictionResult(
                label="watchlist",
                confidence=0.5,
                reason="Not enough SMART data to make a confident prediction.",
                next_step="Check that smartmontools is installed and the drive supports SMART."
            )

        max_severity = max(s["severity"] for s in scores)
        critical_count = sum(1 for s in scores if s["severity"] == 2)
        warning_count  = sum(1 for s in scores if s["severity"] == 1)
        total          = len(scores)

        # --- Determine label ---
        if max_severity == 2:
            label = "risky"
        elif max_severity == 1:
            label = "watchlist"
        else:
            label = "healthy"

        # --- Compute confidence ---
        # Confidence reflects how many rules agree with the final label.
        # For 'risky': fraction of rules that fired at severity >= 1
        # For 'watchlist': weighted blend
        # For 'healthy': fraction of rules that fired at severity 0
        if label == "risky":
            agree = critical_count + warning_count
            confidence = min(0.60 + (agree / total) * 0.35, 0.98)
        elif label == "watchlist":
            agree = warning_count
            confidence = min(0.50 + (agree / total) * 0.30, 0.88)
        else:
            ok_count = sum(1 for s in scores if s["severity"] == 0)
            confidence = min(0.70 + (ok_count / total) * 0.29, 0.99)
        confidence = round(confidence, 2)

        # --- Build reason from worst offenders ---
        worst = sorted(scores, key=lambda s: s["severity"], reverse=True)
        top_detail = worst[0]["detail"]

        if label == "risky":
            reason = f"Drive is at risk: {top_detail}."
            next_step = "Back up your data immediately and consider replacing the drive."
        elif label == "watchlist":
            reason = f"Drive needs monitoring: {top_detail}."
            next_step = "Schedule a backup and monitor this drive weekly."
        else:
            reason = f"Drive appears healthy — {top_detail}."
            next_step = "No action needed. Continue regular backups as good practice."

        return PredictionResult(
            label=label,
            confidence=confidence,
            reason=reason,
            next_step=next_step
        )
