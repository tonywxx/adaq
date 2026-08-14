//! Offline, fictional tutorial panel shared by Factor and Model tests.

use crate::{PythonResearchError, factor::MomentumInputRow, invalid, sha256};
use serde::{Deserialize, Serialize};

pub const TUTORIAL_FIXTURE_ID: &str = "python-tutorial-a-share@1";
pub const TUTORIAL_INSTRUMENT_COUNT: usize = 12;
pub const TUTORIAL_SESSION_COUNT: usize = 180;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyntheticBar {
    pub session: u32,
    pub instrument: String,
    pub close: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureManifest {
    pub fixture_id: String,
    pub synthetic: bool,
    pub instrument_count: usize,
    pub session_count: usize,
    pub calendar_sha256: String,
    pub bars_sha256: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyntheticTutorialFixture {
    pub manifest: FixtureManifest,
    pub instruments: Vec<String>,
    pub sessions: Vec<u32>,
    pub bars: Vec<SyntheticBar>,
}

impl SyntheticTutorialFixture {
    pub fn m12() -> Result<Self, PythonResearchError> {
        let instruments = (1..=TUTORIAL_INSTRUMENT_COUNT)
            .map(|index| format!("SIM{index:02}"))
            .collect::<Vec<_>>();
        let sessions = (1..=TUTORIAL_SESSION_COUNT as u32).collect::<Vec<_>>();
        let bars = sessions
            .iter()
            .flat_map(|session| {
                instruments
                    .iter()
                    .enumerate()
                    .map(move |(index, instrument)| {
                        let trend = (*session as f64) * (index + 1) as f64 * 0.01;
                        let cycle = ((*session as usize + index) % 11) as f64 * 0.001;
                        SyntheticBar {
                            session: *session,
                            instrument: instrument.clone(),
                            close: 100.0 + index as f64 * 2.0 + trend + cycle,
                        }
                    })
            })
            .collect::<Vec<_>>();
        let calendar_sha256 =
            sha256(&serde_json::to_vec(&sessions).map_err(|error| invalid(error.to_string()))?);
        let bars_sha256 =
            sha256(&serde_json::to_vec(&bars).map_err(|error| invalid(error.to_string()))?);
        let mut manifest = FixtureManifest {
            fixture_id: TUTORIAL_FIXTURE_ID.into(),
            synthetic: true,
            instrument_count: instruments.len(),
            session_count: sessions.len(),
            calendar_sha256,
            bars_sha256,
            content_sha256: String::new(),
        };
        manifest.content_sha256 = sha256(
            &serde_json::to_vec(&(
                &manifest.fixture_id,
                &manifest.calendar_sha256,
                &manifest.bars_sha256,
            ))
            .map_err(|error| invalid(error.to_string()))?,
        );
        Ok(Self {
            manifest,
            instruments,
            sessions,
            bars,
        })
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.manifest.fixture_id != TUTORIAL_FIXTURE_ID
            || !self.manifest.synthetic
            || self.instruments.len() != TUTORIAL_INSTRUMENT_COUNT
            || self.sessions.len() != TUTORIAL_SESSION_COUNT
            || self.bars.len() != TUTORIAL_INSTRUMENT_COUNT * TUTORIAL_SESSION_COUNT
        {
            return Err(invalid("tutorial-fixture-shape-invalid"));
        }
        let expected = Self::m12()?;
        if self.manifest != expected.manifest
            || self.instruments != expected.instruments
            || self.sessions != expected.sessions
            || self.bars != expected.bars
        {
            return Err(invalid("tutorial-fixture-hash-or-content-invalid"));
        }
        Ok(())
    }

    pub fn momentum_rows(&self) -> Vec<MomentumInputRow> {
        self.bars
            .iter()
            .map(|bar| MomentumInputRow {
                instrument_id: bar.instrument.clone(),
                observation_time_ms: bar.session as i64,
                close: Some(bar.close),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factor::materialize_momentum;

    #[test]
    fn fixture_is_exactly_synthetic_and_offline() {
        let fixture = SyntheticTutorialFixture::m12().unwrap();
        fixture.validate().unwrap();
        assert_eq!(fixture.bars.len(), 2160);
        assert!(fixture.manifest.synthetic);
        let output =
            materialize_momentum(&fixture.momentum_rows(), &fixture.instruments, 20).unwrap();
        assert_eq!(output.len(), 2160);
        assert!(output.iter().any(|row| row.value.is_some()));
    }

    #[test]
    fn committed_manifest_matches_the_host_generator() {
        let expected: FixtureManifest = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/python-tutorial/manifest.json"
        )))
        .unwrap();
        assert_eq!(expected, SyntheticTutorialFixture::m12().unwrap().manifest);
    }
}
