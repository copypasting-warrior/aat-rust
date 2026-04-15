"""
Drive NLP Question-Answer Engine
=================================
Answers plain-English questions about the current drive's SMART data.

Design
------
- Fully offline — no API calls, no model download
- Context-aware: builds a fact sheet from the SMART summary and uses it
  to answer question-specific intents
- Easy to upgrade: replace `answer()` body with an LLM API call while
  keeping the same signature

Supported question intents
--------------------------
  risky / fail     — why is the drive risky / will it fail
  safe / good      — is the drive safe / is it ok to use
  temperature      — what is the temperature / is it too hot
  reallocated      — what are reallocated sectors
  pending          — what are pending sectors
  shutdown         — what does unsafe shutdown mean
  health           — what is the health score / percentage
  hours / age      — how old is the drive
  backup           — should I back up / when to back up
  replace          — when should I replace the drive
  <fallback>       — generic summary of the drive status
"""


class DriveNlpEngine:
    """
    Offline NLP engine that answers drive health questions using the
    current SMART summary as context.
    """

    # Maps of keywords to intent labels.
    # Order matters — first match wins.
    # More specific / multi-word phrases MUST come before shorter overlapping ones.
    INTENT_MAP: list[tuple[list[str], str]] = [
        (["unsafe shutdown", "power loss", "sudden", "unclean", "unsafe shut"],  "unsafe_shutdown"),
        (["backup", "back up", "copy", "save data", "data loss"],                "backup"),
        (["replac", "new drive", "buy", "swap", "when should"],                  "replace"),
        (["risky", "fail", "dying", "broken", "damage", "bad"],                  "risky_reason"),
        (["safe", "good", "ok", "fine", "healthy", "trust"],                     "safe_status"),
        (["temp", "hot", "heat", "celsius", "degree"],                           "temperature"),
        (["realloc", "remapp", "relocated", "sector"],                           "reallocated"),
        (["pending", "instab", "waiting"],                                       "pending"),
        (["health", "percent", "score", "condition", "life"],                    "health_score"),
        (["hour", "old", "age", "how long", "uptime"],                           "drive_age"),
    ]


    def _detect_intent(self, question: str) -> str:
        q = question.lower()
        for keywords, intent in self.INTENT_MAP:
            if any(k in q for k in keywords):
                return intent
        return "general"

    def _build_fact_sheet(self, smart: dict) -> dict:
        """Extract and normalize the key facts from the SMART summary."""
        def get(key, default=None):
            v = smart.get(key)
            if v is None or v == "" or str(v) in ("null", "None"):
                return default
            return v

        label      = get("ai_label", "unknown")
        confidence = get("ai_confidence", None)
        health     = get("health_percent", None)
        temp       = get("temp_c", None)
        hours      = get("power_on_hours", None)
        realloc    = get("reallocated_sectors", 0)
        pending    = get("pending_sectors", 0)
        uncorr     = get("uncorrectable_sectors", 0)
        unsafe     = get("unsafe_shutdowns", 0)
        model      = get("model", "this drive")
        reason     = get("ai_reason", "")

        return {
            "label": label,
            "confidence": confidence,
            "health": health,
            "temp": temp,
            "hours": hours,
            "realloc": int(realloc) if realloc is not None else 0,
            "pending": int(pending) if pending is not None else 0,
            "uncorr": int(uncorr) if uncorr is not None else 0,
            "unsafe": int(unsafe) if unsafe is not None else 0,
            "model": model,
            "reason": reason,
        }

    def _answer_risky_reason(self, f: dict) -> str:
        issues = []
        if f["realloc"] > 0:
            issues.append(f"{f['realloc']} reallocated sector(s)")
        if f["pending"] > 0:
            issues.append(f"{f['pending']} pending sector(s)")
        if f["uncorr"] > 0:
            issues.append(f"{f['uncorr']} uncorrectable error(s)")
        if f["health"] is not None and f["health"] < 60:
            issues.append(f"health at only {f['health']}%")
        if f["temp"] is not None and f["temp"] >= 55:
            issues.append(f"high temperature ({f['temp']}°C)")

        if issues:
            return (f"{f['model']} is flagged as {f['label']} because it has "
                    f"{', '.join(issues)}. " + f["reason"])
        return (f"{f['model']} shows a {f['label']} status. " + (f["reason"] or
                "Multiple SMART indicators suggest elevated risk."))

    def _answer_safe_status(self, f: dict) -> str:
        label = f["label"]
        health = f["health"]
        h_str = f" Its health is {health}%." if health is not None else ""
        if label == "healthy":
            return (f"{f['model']} is safe to use.{h_str} "
                    "All monitored SMART indicators are within normal limits.")
        elif label == "watchlist":
            return (f"{f['model']} can still be used but should be monitored carefully.{h_str} "
                    "Consider running a backup soon.")
        else:
            return (f"{f['model']} is NOT safe to rely on.{h_str} "
                    "Back up your data immediately and plan a replacement.")

    def _answer_temperature(self, f: dict) -> str:
        temp = f["temp"]
        if temp is None:
            return "Temperature data is not available for this drive."
        if temp >= 65:
            return (f"The drive temperature is {temp}°C — critically high. "
                    "Shut down the system and check cooling immediately.")
        if temp >= 55:
            return (f"The drive temperature is {temp}°C — elevated. "
                    "Ensure adequate airflow and check case ventilation.")
        return (f"The drive temperature is {temp}°C — within normal range "
                "(safe operating limit is typically 0–70°C).")

    def _answer_reallocated(self, f: dict) -> str:
        n = f["realloc"]
        base = ("Reallocated sectors are damaged areas of the disk that the drive "
                "firmware has moved to a reserved spare area. ")
        if n == 0:
            return base + "This drive has none — a good sign."
        if n < 10:
            return base + f"This drive has {n}, which is an early warning sign worth monitoring."
        return base + f"This drive has {n} — a significant number indicating physical wear."

    def _answer_pending(self, f: dict) -> str:
        n = f["pending"]
        base = ("Pending sectors are disk areas that could not be read successfully "
                "and are waiting to be reallocated or recovered. ")
        if n == 0:
            return base + "This drive has none — good."
        return base + f"This drive has {n} pending sector(s), which indicates instability."

    def _answer_unsafe_shutdown(self, f: dict) -> str:
        n = f["unsafe"]
        base = ("An unsafe shutdown happens when the drive loses power suddenly "
                "without a clean OS shutdown — for example during a power cut or system crash. ")
        if n == 0:
            return base + "This drive has zero recorded. That is ideal."
        if n < 20:
            return base + f"This drive has {n} recorded — a moderate amount."
        return base + (f"This drive has {n} recorded — a high count that increases "
                       "the risk of file system corruption.")

    def _answer_health_score(self, f: dict) -> str:
        h = f["health"]
        if h is None:
            return "Health percentage data is not available for this drive (common for SATA/HDD)."
        if h > 84:
            return (f"The drive health score is {h}% — excellent. "
                    "This reflects how much of the drive's rated endurance remains.")
        if h > 60:
            return (f"The drive health score is {h}% — acceptable but aging. "
                    "Plan for replacement in the coming year.")
        return (f"The drive health score is {h}% — low. "
                "The drive is nearing the end of its rated lifespan.")

    def _answer_drive_age(self, f: dict) -> str:
        hours = f["hours"]
        if hours is None:
            return "Power-on hours data is not available for this drive."
        years = hours / 8760
        if years < 1:
            return f"The drive has been powered on for {hours} hours — less than one year of use."
        return (f"The drive has {hours:,} power-on hours, equivalent to about "
                f"{years:.1f} years of continuous use.")

    def _answer_backup(self, f: dict) -> str:
        label = f["label"]
        if label == "risky":
            return ("Back up NOW. The drive shows signs of failure — every hour of delay is a risk.")
        if label == "watchlist":
            return ("You should back up soon. The drive is not critical yet, but "
                    "running a backup this week is strongly recommended.")
        return ("Regular backups are always good practice. This drive currently "
                "shows no urgent warning signs.")

    def _answer_replace(self, f: dict) -> str:
        label = f["label"]
        health = f["health"]
        h_str = f" (health: {health}%)" if health is not None else ""
        if label == "risky":
            return (f"Replace this drive as soon as possible{h_str}. "
                    "Current SMART data indicates it is at high risk of failure.")
        if label == "watchlist":
            return (f"Plan to replace this drive within the next 6–12 months{h_str}. "
                    "It is still functional but showing early wear.")
        return (f"No immediate replacement needed{h_str}. "
                "Continue monitoring every few months.")

    def _answer_general(self, f: dict) -> str:
        health_str = f"{f['health']}%" if f["health"] is not None else "unknown"
        temp_str   = f"{f['temp']}°C" if f["temp"] is not None else "unknown"
        hours_str  = f"{f['hours']:,} hours" if f["hours"] is not None else "unknown"
        return (f"{f['model']} — status: {f['label']}. "
                f"Health: {health_str}, temperature: {temp_str}, "
                f"power-on hours: {hours_str}. "
                f"{f['reason']}")

    def answer(self, question: str, smart: dict) -> str:
        """
        Answer a plain-English question about the drive.

        Parameters
        ----------
        question : str
            User's question (any length, any wording).
        smart : dict
            SMART summary dict — same keys as the /predict payload, plus
            optional ai_label / ai_confidence / ai_reason from the predict step.

        Returns
        -------
        str
            Short, plain-English answer (1–3 sentences).
        """
        intent = self._detect_intent(question)
        f = self._build_fact_sheet(smart)

        handlers = {
            "risky_reason":    self._answer_risky_reason,
            "safe_status":     self._answer_safe_status,
            "temperature":     self._answer_temperature,
            "reallocated":     self._answer_reallocated,
            "pending":         self._answer_pending,
            "unsafe_shutdown": self._answer_unsafe_shutdown,
            "health_score":    self._answer_health_score,
            "drive_age":       self._answer_drive_age,
            "backup":          self._answer_backup,
            "replace":         self._answer_replace,
            "general":         self._answer_general,
        }

        handler = handlers.get(intent, self._answer_general)
        return handler(f)
