# Keep Gate 7 candidate artifacts explicit

Status: accepted

Gate 7 retains one immutable Candidate Model Artifact for each completed, repeatability-verified Model Trial and records its successful Attempt and artifact identity directly on the Trial; failed or cancelled Attempts never create a candidate pointer. Gate 8 owns the User Parameter Selection Decision, binds exactly one Candidate Model Artifact as the Selected Model Artifact, and only then accepts downstream Forecast Signal Dataset and Final Evaluation evidence. This keeps training lineage explicit and prevents trial-level forecasts from becoming downstream outputs before User selection.
