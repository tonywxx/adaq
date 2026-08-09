//! Asset-neutral market identity, Venue time, and trading calendar contracts.
//!
//! Every event and Bar boundary is stored as a unique UTC instant (Unix
//! milliseconds), while Venue rules, Trading Dates, Trading Sessions, Session
//! Phases, and Bar boundaries are evaluated in the Venue's IANA time zone.
//! China A-share rules use `Asia/Shanghai`, United States equity rules use
//! `America/New_York` through time-zone database rules, and continuously
//! traded crypto stays on its recorded UTC grid.

use std::fmt;

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::BarInterval;

/// The asset-neutral instrument class represented by a Venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VenueKind {
    CryptoSpot,
    ChinaAShareEquity,
    UsEquity,
}

impl VenueKind {
    /// The V1 IANA time zone mandated for this Venue class.
    pub const fn time_zone_name(self) -> &'static str {
        match self {
            Self::CryptoSpot => "UTC",
            Self::ChinaAShareEquity => "Asia/Shanghai",
            Self::UsEquity => "America/New_York",
        }
    }
}

/// A stable, provider-independent Venue identity.
///
/// A Venue is the exchange or market (for example `okx`, `sse`, `nasdaq`),
/// never the Market Data Provider that delivers observations about it.
/// Deserialization re-validates the exact class-to-time-zone mapping, so a
/// payload can never construct an invalid Venue.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "VenueRaw", rename_all = "camelCase")]
pub struct Venue {
    pub id: String,
    pub kind: VenueKind,
    pub time_zone: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VenueRaw {
    id: String,
    kind: VenueKind,
    time_zone: String,
}

impl TryFrom<VenueRaw> for Venue {
    type Error = CalendarError;

    fn try_from(raw: VenueRaw) -> Result<Self, Self::Error> {
        if raw.time_zone.parse::<Tz>().is_err() {
            return Err(CalendarError::InvalidTimeZone(raw.time_zone));
        }
        if raw.time_zone != raw.kind.time_zone_name() {
            return Err(CalendarError::InvalidTimeZone(format!(
                "{:?} venues must use time zone {}, found {}",
                raw.kind,
                raw.kind.time_zone_name(),
                raw.time_zone
            )));
        }
        let venue = Self::new(raw.id, raw.kind)?;
        Ok(venue)
    }
}

impl Venue {
    /// Builds a Venue whose IANA time zone matches its instrument class.
    ///
    /// The mapping is exact: Crypto Spot requires `UTC`, China A-share
    /// requires `Asia/Shanghai`, and U.S. Equity requires `America/New_York`
    /// through time-zone database rules; a fixed UTC offset is never accepted.
    pub fn new(id: impl Into<String>, kind: VenueKind) -> Result<Self, CalendarError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(CalendarError::InvalidVenue("Venue id must be non-empty"));
        }
        let time_zone = kind.time_zone_name();
        time_zone
            .parse::<Tz>()
            .map_err(|_| CalendarError::InvalidTimeZone(time_zone.to_owned()))?;
        Ok(Self {
            id,
            kind,
            time_zone: time_zone.to_owned(),
        })
    }

    pub fn crypto_spot(id: impl Into<String>) -> Result<Self, CalendarError> {
        Self::new(id, VenueKind::CryptoSpot)
    }

    pub fn china_a_share(id: impl Into<String>) -> Result<Self, CalendarError> {
        Self::new(id, VenueKind::ChinaAShareEquity)
    }

    pub fn us_equity(id: impl Into<String>) -> Result<Self, CalendarError> {
        Self::new(id, VenueKind::UsEquity)
    }

    /// The parsed IANA time zone rule. Infallible because construction
    /// validated the name against the time-zone database.
    pub fn tz(&self) -> Tz {
        self.time_zone
            .parse()
            .expect("venue time zone was validated at construction")
    }

    /// Converts a UTC instant to this Venue's local wall-clock time.
    pub fn local_time(&self, utc_ms: i64) -> Result<NaiveDateTime, CalendarError> {
        let utc = utc_datetime(utc_ms)?;
        Ok(self.tz().from_utc_datetime(&utc.naive_utc()).naive_local())
    }

    /// Converts a Venue-local wall-clock time to a UTC instant.
    ///
    /// Nonexistent local times (spring-forward gaps) are always rejected;
    /// ambiguous local times (fall-back duplicates) are rejected unless an
    /// explicit disambiguation is supplied.
    pub fn resolve_local_time(
        &self,
        local: NaiveDateTime,
        disambiguation: LocalTimeDisambiguation,
    ) -> Result<i64, CalendarError> {
        match self.tz().from_local_datetime(&local) {
            chrono::LocalResult::Single(value) => Ok(value.timestamp_millis()),
            chrono::LocalResult::Ambiguous(earlier, later) => match disambiguation {
                LocalTimeDisambiguation::Reject => Err(CalendarError::AmbiguousLocalTime(
                    local.to_string(),
                    self.time_zone.clone(),
                )),
                LocalTimeDisambiguation::Earlier => Ok(earlier.timestamp_millis()),
                LocalTimeDisambiguation::Later => Ok(later.timestamp_millis()),
            },
            chrono::LocalResult::None => Err(CalendarError::NonexistentLocalTime(
                local.to_string(),
                self.time_zone.clone(),
            )),
        }
    }
}

/// How an ambiguous Venue-local wall-clock time (DST fall-back duplicate) is
/// resolved. Nonexistent times are rejected regardless of this choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalTimeDisambiguation {
    Reject,
    Earlier,
    Later,
}

/// The ADAQ-wide Instrument identity: a Venue plus that Venue's native
/// Instrument code. It is provider-independent because the Venue is never the
/// Market Data Provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentId {
    pub venue: Venue,
    pub code: String,
}

impl InstrumentId {
    pub fn new(venue: Venue, code: impl Into<String>) -> Result<Self, CalendarError> {
        let code = code.into();
        if code.trim().is_empty() {
            return Err(CalendarError::InvalidInstrument(
                "Instrument code must be non-empty",
            ));
        }
        Ok(Self { venue, code })
    }
}

/// Provenance of the provider-native symbol mapped onto one Instrument ID.
///
/// The provider-native symbol is retained verbatim so a provider payload can
/// always be traced back to its Instrument identity and acquisition evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentSourceMapping {
    pub instrument: InstrumentId,
    pub provider: String,
    pub provider_symbol: String,
    pub connector_version: String,
    pub captured_at_ms: i64,
}

/// A Venue-local calendar date to which session-based market observations
/// belong under one Trading Calendar Snapshot. It is an explicit identity and
/// is never inferred from the UTC calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl TradingDate {
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self, CalendarError> {
        NaiveDate::from_ymd_opt(year, month.into(), day.into())
            .map(|_| Self { year, month, day })
            .ok_or(CalendarError::InvalidTradingDate(year, month, day))
    }

    pub fn from_naive_date(date: NaiveDate) -> Self {
        Self {
            year: date.year(),
            month: date.month() as u8,
            day: date.day() as u8,
        }
    }

    pub fn to_naive_date(self) -> Result<NaiveDate, CalendarError> {
        NaiveDate::from_ymd_opt(self.year, self.month.into(), self.day.into()).ok_or(
            CalendarError::InvalidTradingDate(self.year, self.month, self.day),
        )
    }

    /// The Venue-local calendar date of a UTC instant under the Venue rule.
    pub fn from_utc_ms(venue: &Venue, utc_ms: i64) -> Result<Self, CalendarError> {
        Ok(Self::from_naive_date(venue.local_time(utc_ms)?.date()))
    }
}

impl fmt::Display for TradingDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

/// The Trading Calendar classification of an observation or order-eligibility
/// window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    PreOpen,
    Auction,
    Continuous,
    Break,
    ExtendedHours,
    Closed,
}

/// One ordered Venue-local window of a Trading Session with its Session Phase.
/// Windows may repeat a phase (A-share morning and afternoon sessions are both
/// Continuous) and a scheduled break is its own window rather than a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingSession {
    pub phase: SessionPhase,
    pub start_local: NaiveTime,
    pub end_local: NaiveTime,
}

/// A scheduled non-trading period recorded as calendar evidence, never as a
/// Bar Gap.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledClosure {
    pub kind: ScheduledClosureKind,
    pub start_ms: i64,
    pub end_ms: i64,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledClosureKind {
    Holiday,
    EarlyClose,
    SpecialClosure,
    Maintenance,
}

/// The classification of one Venue-local calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DayKind {
    TradingDay,
    Holiday,
    Weekend,
    SpecialClosure,
}

/// Evidence about one Venue-local calendar date in a Trading Calendar
/// Snapshot. Whole-day non-trading is `day_kind`; intra-day closures are
/// `closures`; `session_override` replaces the snapshot default sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayEvidence {
    pub date: TradingDate,
    pub day_kind: DayKind,
    #[serde(default)]
    pub session_override: Option<Vec<TradingSession>>,
    #[serde(default)]
    pub closures: Vec<ScheduledClosure>,
}

impl DayEvidence {
    pub fn trading_day(date: TradingDate) -> Self {
        Self {
            date,
            day_kind: DayKind::TradingDay,
            session_override: None,
            closures: Vec::new(),
        }
    }

    pub fn closed(date: TradingDate, day_kind: DayKind) -> Self {
        Self {
            date,
            day_kind,
            session_override: None,
            closures: Vec::new(),
        }
    }
}

/// An immutable Venue calendar revision defining the Venue Time Zone, default
/// Trading Sessions, Trading Dates, holidays, early closes, and special
/// closures for an exact effective UTC range. Deserialization re-runs `new`'s
/// validation, so a payload can never construct an invalid Snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TradingCalendarSnapshotRaw", rename_all = "camelCase")]
pub struct TradingCalendarSnapshot {
    pub snapshot_id: String,
    pub venue: Venue,
    pub effective_from_ms: i64,
    pub effective_to_ms: i64,
    /// The weekday session template; empty for continuously traded Venues.
    #[serde(default)]
    pub default_sessions: Vec<TradingSession>,
    #[serde(default)]
    pub days: Vec<DayEvidence>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TradingCalendarSnapshotRaw {
    snapshot_id: String,
    venue: Venue,
    effective_from_ms: i64,
    effective_to_ms: i64,
    #[serde(default)]
    default_sessions: Vec<TradingSession>,
    #[serde(default)]
    days: Vec<DayEvidence>,
}

impl TryFrom<TradingCalendarSnapshotRaw> for TradingCalendarSnapshot {
    type Error = CalendarError;

    fn try_from(raw: TradingCalendarSnapshotRaw) -> Result<Self, Self::Error> {
        Self::new(
            raw.snapshot_id,
            raw.venue,
            raw.effective_from_ms,
            raw.effective_to_ms,
            raw.default_sessions,
            raw.days,
        )
    }
}

impl TradingCalendarSnapshot {
    pub fn new(
        snapshot_id: impl Into<String>,
        venue: Venue,
        effective_from_ms: i64,
        effective_to_ms: i64,
        default_sessions: Vec<TradingSession>,
        days: Vec<DayEvidence>,
    ) -> Result<Self, CalendarError> {
        let snapshot_id = snapshot_id.into();
        if snapshot_id.trim().is_empty() {
            return Err(CalendarError::InvalidCalendar(
                "snapshot id must be non-empty",
            ));
        }
        if effective_from_ms >= effective_to_ms {
            return Err(CalendarError::InvalidCalendar(
                "calendar effective range must be increasing",
            ));
        }
        validate_sessions(&default_sessions)?;
        let mut sorted = days;
        sorted.sort_by_key(|day| day.date);
        for adjacent in sorted.windows(2) {
            if adjacent[0].date == adjacent[1].date {
                return Err(CalendarError::InvalidCalendar(
                    "duplicate calendar day evidence",
                ));
            }
        }
        for day in &sorted {
            if let Some(sessions) = &day.session_override {
                validate_sessions(sessions)?;
            }
            for closure in &day.closures {
                if closure.start_ms >= closure.end_ms {
                    return Err(CalendarError::InvalidCalendar(
                        "scheduled closure must be increasing",
                    ));
                }
            }
        }
        Ok(Self {
            snapshot_id,
            venue,
            effective_from_ms,
            effective_to_ms,
            default_sessions,
            days: sorted,
        })
    }

    pub fn contains(&self, utc_ms: i64) -> bool {
        utc_ms >= self.effective_from_ms && utc_ms < self.effective_to_ms
    }

    pub fn day(&self, date: TradingDate) -> Option<&DayEvidence> {
        self.days
            .binary_search_by_key(&date, |day| day.date)
            .ok()
            .map(|index| &self.days[index])
    }

    /// Whether one Venue-local date is a Trading Date. Weekends are
    /// structurally closed; holidays and special closures require recorded
    /// DayEvidence. Unrecorded weekdays follow the default session template.
    pub fn is_trading_day(&self, date: TradingDate) -> Result<bool, CalendarError> {
        let weekday = date.to_naive_date()?.weekday();
        if matches!(weekday, Weekday::Sat | Weekday::Sun) {
            return Ok(false);
        }
        Ok(self
            .day(date)
            .map(|day| day.day_kind == DayKind::TradingDay)
            .unwrap_or(true))
    }

    /// The sessions scheduled for one date: the day override when present,
    /// otherwise the snapshot default template. A non-trading day has none.
    pub fn sessions_for(&self, date: TradingDate) -> Result<&[TradingSession], CalendarError> {
        if !self.is_trading_day(date)? {
            return Ok(&[]);
        }
        Ok(self
            .day(date)
            .and_then(|day| day.session_override.as_deref())
            .unwrap_or(&self.default_sessions))
    }

    /// The Venue-local Trading Date of a UTC instant. Errors when the instant
    /// lies outside the snapshot's effective range.
    pub fn trading_date_of(&self, utc_ms: i64) -> Result<TradingDate, CalendarError> {
        if !self.contains(utc_ms) {
            return Err(CalendarError::OutsideCalendarRange(utc_ms));
        }
        TradingDate::from_utc_ms(&self.venue, utc_ms)
    }

    /// The Session Phase classification of a UTC instant.
    pub fn session_phase_at(&self, utc_ms: i64) -> Result<SessionPhase, CalendarError> {
        Ok(self.session_context_at(utc_ms)?.phase)
    }

    /// Binds one session-based market observation to its full context: the
    /// Trading Calendar Snapshot, Venue, IANA time zone, Venue-local Trading
    /// Date, and Session Phase.
    pub fn session_context_at(&self, utc_ms: i64) -> Result<SessionContext, CalendarError> {
        if !self.contains(utc_ms) {
            return Err(CalendarError::OutsideCalendarRange(utc_ms));
        }
        let date = self.trading_date_of(utc_ms)?;
        let phase = if !self.is_trading_day(date)? {
            SessionPhase::Closed
        } else {
            let sessions = self.sessions_for(date)?;
            let local = self.venue.local_time(utc_ms)?.time();
            if sessions.is_empty() {
                SessionPhase::Closed
            } else if local < sessions[0].start_local {
                SessionPhase::PreOpen
            } else if let Some(session) = sessions
                .iter()
                .find(|session| local >= session.start_local && local < session.end_local)
            {
                session.phase
            } else {
                SessionPhase::Closed
            }
        };
        Ok(SessionContext {
            snapshot_id: self.snapshot_id.clone(),
            venue: self.venue.clone(),
            time_zone: self.venue.time_zone.clone(),
            trading_date: date,
            phase,
        })
    }

    /// Whether a UTC instant is a scheduled non-trading period under this
    /// calendar: a holiday, weekend, special closure, scheduled break,
    /// early close, maintenance window, or outside every Trading Session.
    /// Missing Bars in such a period are calendar state, never Bar Gaps.
    pub fn is_scheduled_non_trading(&self, utc_ms: i64) -> Result<bool, CalendarError> {
        if !self.contains(utc_ms) {
            return Err(CalendarError::OutsideCalendarRange(utc_ms));
        }
        let date = self.trading_date_of(utc_ms)?;
        if !self.is_trading_day(date)? {
            return Ok(true);
        }
        if let Some(day) = self.day(date) {
            if day
                .closures
                .iter()
                .any(|closure| utc_ms >= closure.start_ms && utc_ms < closure.end_ms)
            {
                return Ok(true);
            }
        }
        Ok(!matches!(
            self.session_phase_at(utc_ms)?,
            SessionPhase::Continuous | SessionPhase::Auction
        ))
    }

    /// Resolves one date's sessions to UTC instants under the Venue rule.
    pub fn session_windows_utc(
        &self,
        date: TradingDate,
    ) -> Result<Vec<SessionWindowUtc>, CalendarError> {
        let sessions = self.sessions_for(date)?;
        if sessions.is_empty() {
            return Err(CalendarError::InvalidCalendar(
                "sessions requested for a non-trading day",
            ));
        }
        sessions
            .iter()
            .map(|session| {
                let start_ms = self.venue.resolve_local_time(
                    date.to_naive_date()?.and_time(session.start_local),
                    LocalTimeDisambiguation::Reject,
                )?;
                let end_ms = self.venue.resolve_local_time(
                    date.to_naive_date()?.and_time(session.end_local),
                    LocalTimeDisambiguation::Reject,
                )?;
                Ok(SessionWindowUtc {
                    phase: session.phase,
                    start_ms,
                    end_ms,
                })
            })
            .collect()
    }

    /// The UTC window of the scheduled session containing a UTC instant.
    pub fn session_window_containing(
        &self,
        utc_ms: i64,
    ) -> Result<Option<SessionWindowUtc>, CalendarError> {
        let date = self.trading_date_of(utc_ms)?;
        Ok(self
            .session_windows_utc(date)?
            .into_iter()
            .find(|window| utc_ms >= window.start_ms && utc_ms < window.end_ms))
    }

    /// The UTC instant at which the daily (or longer) Bar for one Trading
    /// Date opens: the first scheduled session start on that trading day.
    pub fn daily_boundary_open_ms(&self, date: TradingDate) -> Result<i64, CalendarError> {
        let windows = self.session_windows_utc(date)?;
        windows
            .first()
            .map(|window| window.start_ms)
            .ok_or(CalendarError::InvalidCalendar(
                "daily boundary requested for a non-trading day",
            ))
    }

    /// The next Trading Date after `date`, skipping weekends, holidays, and
    /// special closures.
    pub fn next_trading_date(&self, date: TradingDate) -> Result<TradingDate, CalendarError> {
        let mut candidate = date.to_naive_date()?;
        loop {
            candidate = candidate
                .succ_opt()
                .ok_or(CalendarError::InvalidTradingDate(
                    date.year, date.month, date.day,
                ))?;
            let next = TradingDate::from_naive_date(candidate);
            if self.is_trading_day(next)? {
                return Ok(next);
            }
        }
    }

    /// The Trading Date `count` trading dates after `anchor`, counting
    /// consecutive Trading Dates for multi-day Bar anchors.
    pub fn trading_date_offset(
        &self,
        anchor: TradingDate,
        count: u32,
    ) -> Result<TradingDate, CalendarError> {
        let mut date = anchor;
        for _ in 0..count {
            date = self.next_trading_date(date)?;
        }
        Ok(date)
    }
}

/// A resolved Trading Session window expressed as UTC instants while keeping
/// its Venue-local phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWindowUtc {
    pub phase: SessionPhase,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// The full calendar binding of one session-based market observation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContext {
    pub snapshot_id: String,
    pub venue: Venue,
    pub time_zone: String,
    pub trading_date: TradingDate,
    pub phase: SessionPhase,
}

/// A Venue-local presentation of a UTC instant that never changes the
/// canonical UTC identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VenueLocalTime {
    pub utc_ms: i64,
    pub local: NaiveDateTime,
}

impl VenueLocalTime {
    pub fn from_utc(venue: &Venue, utc_ms: i64) -> Result<Self, CalendarError> {
        Ok(Self {
            utc_ms,
            local: venue.local_time(utc_ms)?,
        })
    }

    pub fn format_iso8601(&self) -> String {
        self.local.format("%Y-%m-%dT%H:%M:%S").to_string()
    }

    pub fn format_date(&self) -> String {
        self.local.date().format("%Y-%m-%d").to_string()
    }

    pub fn format_time(&self) -> String {
        self.local.time().format("%H:%M:%S").to_string()
    }
}

/// The unique identity of one Bar: Instrument ID, Bar Interval, and Bar Open
/// Time as a UTC instant. Identical duplicates collapse; conflicting
/// duplicates are invalid provider data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarIdentity {
    pub instrument: InstrumentId,
    pub interval: BarInterval,
    pub open_time_ms: i64,
}

impl BarIdentity {
    /// Explicit mapping from the existing provider-keyed crypto identity
    /// `(src, code)` to the Venue-keyed contract, preserving the recorded UTC
    /// grid semantics of Snapshot and Backtest consumers.
    pub fn from_legacy(
        src: &str,
        code: &str,
        interval: BarInterval,
        open_time_ms: i64,
    ) -> Result<Self, CalendarError> {
        let venue = match src {
            crate::OKX_SRC => Venue::crypto_spot("okx")?,
            _ => return Err(CalendarError::UnsupportedSource(src.to_owned())),
        };
        Ok(Self {
            instrument: InstrumentId::new(venue, code)?,
            interval,
            open_time_ms,
        })
    }
}

/// The Venue-local Trading Date of a continuously traded crypto observation:
/// the UTC calendar date of its recorded grid.
pub fn crypto_trading_date(utc_ms: i64) -> Result<TradingDate, CalendarError> {
    Ok(TradingDate::from_naive_date(
        utc_datetime(utc_ms)?.date_naive(),
    ))
}

/// The Session Phase of a continuously traded crypto observation: Continuous
/// on the recorded UTC grid unless it falls inside retained provider
/// maintenance or outage evidence.
pub fn crypto_session_phase(
    utc_ms: i64,
    closures: &[ScheduledClosure],
) -> Result<SessionPhase, CalendarError> {
    if closures
        .iter()
        .any(|closure| utc_ms >= closure.start_ms && utc_ms < closure.end_ms)
    {
        return Ok(SessionPhase::Closed);
    }
    Ok(SessionPhase::Continuous)
}

fn utc_datetime(utc_ms: i64) -> Result<DateTime<Utc>, CalendarError> {
    DateTime::<Utc>::from_timestamp_millis(utc_ms).ok_or(CalendarError::InvalidUtcInstant(utc_ms))
}

fn validate_sessions(sessions: &[TradingSession]) -> Result<(), CalendarError> {
    let mut previous_end = None;
    for session in sessions {
        if session.start_local >= session.end_local {
            return Err(CalendarError::InvalidCalendar(
                "trading session must be increasing",
            ));
        }
        if previous_end.is_some_and(|end| end > session.start_local) {
            return Err(CalendarError::InvalidCalendar(
                "trading sessions must not overlap",
            ));
        }
        previous_end = Some(session.end_local);
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CalendarError {
    #[error("invalid IANA time zone: {0}")]
    InvalidTimeZone(String),
    #[error("invalid Venue: {0}")]
    InvalidVenue(&'static str),
    #[error("invalid Instrument: {0}")]
    InvalidInstrument(&'static str),
    #[error("invalid calendar: {0}")]
    InvalidCalendar(&'static str),
    #[error("invalid Trading Date {0:04}-{1:02}-{2:02}")]
    InvalidTradingDate(i32, u8, u8),
    #[error("invalid UTC instant: {0}")]
    InvalidUtcInstant(i64),
    #[error("UTC instant {0} is outside the calendar effective range")]
    OutsideCalendarRange(i64),
    #[error("local time {0} does not exist in {1}")]
    NonexistentLocalTime(String, String),
    #[error("local time {0} is ambiguous in {1}; supply Earlier or Later")]
    AmbiguousLocalTime(String, String),
    #[error("no calendar mapping for provider source: {0}")]
    UnsupportedSource(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 86_400_000;

    fn utc_ms(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> i64 {
        NaiveDate::from_ymd_opt(year, month.into(), day.into())
            .unwrap()
            .and_hms_opt(hour.into(), minute.into(), 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    fn local(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month.into(), day.into())
            .unwrap()
            .and_hms_opt(hour.into(), minute.into(), 0)
            .unwrap()
    }

    fn session(phase: SessionPhase, start: (u8, u8), end: (u8, u8)) -> TradingSession {
        TradingSession {
            phase,
            start_local: NaiveTime::from_hms_opt(start.0.into(), start.1.into(), 0).unwrap(),
            end_local: NaiveTime::from_hms_opt(end.0.into(), end.1.into(), 0).unwrap(),
        }
    }

    fn a_share_sessions() -> Vec<TradingSession> {
        vec![
            session(SessionPhase::Continuous, (9, 30), (11, 30)),
            session(SessionPhase::Break, (11, 30), (13, 0)),
            session(SessionPhase::Continuous, (13, 0), (15, 0)),
        ]
    }

    fn us_sessions() -> Vec<TradingSession> {
        vec![session(SessionPhase::Continuous, (9, 30), (16, 0))]
    }

    fn a_share_calendar() -> TradingCalendarSnapshot {
        TradingCalendarSnapshot::new(
            "a-share-test-2024",
            Venue::china_a_share("sse").unwrap(),
            utc_ms(2024, 1, 1, 0, 0),
            utc_ms(2025, 1, 1, 0, 0),
            a_share_sessions(),
            vec![
                DayEvidence::closed(TradingDate::new(2024, 10, 1).unwrap(), DayKind::Holiday),
                DayEvidence::closed(
                    TradingDate::new(2024, 5, 13).unwrap(),
                    DayKind::SpecialClosure,
                ),
            ],
        )
        .unwrap()
    }

    fn us_calendar() -> TradingCalendarSnapshot {
        let new_year = DayEvidence::closed(TradingDate::new(2024, 1, 1).unwrap(), DayKind::Holiday);
        let independence_day =
            DayEvidence::closed(TradingDate::new(2024, 7, 4).unwrap(), DayKind::Holiday);
        let early_close = DayEvidence {
            date: TradingDate::new(2024, 7, 3).unwrap(),
            day_kind: DayKind::TradingDay,
            session_override: Some(vec![session(SessionPhase::Continuous, (9, 30), (13, 0))]),
            closures: vec![ScheduledClosure {
                kind: ScheduledClosureKind::EarlyClose,
                start_ms: utc_ms(2024, 7, 3, 17, 0),
                end_ms: utc_ms(2024, 7, 3, 21, 0),
                reason: Some("Independence Day eve early close".to_owned()),
            }],
        };
        let mlk_day = DayEvidence::closed(TradingDate::new(2024, 1, 15).unwrap(), DayKind::Holiday);
        TradingCalendarSnapshot::new(
            "us-test-2024",
            Venue::us_equity("nasdaq").unwrap(),
            utc_ms(2024, 1, 1, 0, 0),
            utc_ms(2025, 1, 1, 0, 0),
            us_sessions(),
            vec![new_year, independence_day, early_close, mlk_day],
        )
        .unwrap()
    }

    fn weekend_calendar() -> TradingCalendarSnapshot {
        TradingCalendarSnapshot::new(
            "us-weekend-test",
            Venue::us_equity("nyse").unwrap(),
            utc_ms(2024, 1, 1, 0, 0),
            utc_ms(2025, 1, 1, 0, 0),
            us_sessions(),
            vec![
                DayEvidence::closed(TradingDate::new(2024, 1, 6).unwrap(), DayKind::Weekend),
                DayEvidence::closed(TradingDate::new(2024, 1, 7).unwrap(), DayKind::Weekend),
            ],
        )
        .unwrap()
    }

    #[test]
    fn a_share_session_boundaries_and_midday_break_are_venue_local() {
        let calendar = a_share_calendar();
        let date = TradingDate::new(2024, 3, 11).unwrap();

        let windows = calendar.session_windows_utc(date).unwrap();
        assert_eq!(
            windows,
            vec![
                SessionWindowUtc {
                    phase: SessionPhase::Continuous,
                    start_ms: utc_ms(2024, 3, 11, 1, 30),
                    end_ms: utc_ms(2024, 3, 11, 3, 30),
                },
                SessionWindowUtc {
                    phase: SessionPhase::Break,
                    start_ms: utc_ms(2024, 3, 11, 3, 30),
                    end_ms: utc_ms(2024, 3, 11, 5, 0),
                },
                SessionWindowUtc {
                    phase: SessionPhase::Continuous,
                    start_ms: utc_ms(2024, 3, 11, 5, 0),
                    end_ms: utc_ms(2024, 3, 11, 7, 0),
                },
            ]
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 3, 11, 0, 0))
                .unwrap(),
            SessionPhase::PreOpen
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 3, 11, 1, 30))
                .unwrap(),
            SessionPhase::Continuous
        );
        // 03:30 UTC is 11:30 in Shanghai: the scheduled midday break.
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 3, 11, 3, 30))
                .unwrap(),
            SessionPhase::Break
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 3, 11, 6, 59))
                .unwrap(),
            SessionPhase::Continuous
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 3, 11, 7, 0))
                .unwrap(),
            SessionPhase::Closed
        );
        assert!(
            calendar
                .is_scheduled_non_trading(utc_ms(2024, 3, 11, 3, 30))
                .unwrap()
        );
        assert!(
            !calendar
                .is_scheduled_non_trading(utc_ms(2024, 3, 11, 2, 0))
                .unwrap()
        );
    }

    #[test]
    fn trading_date_is_venue_local_not_utc() {
        let calendar = a_share_calendar();
        // 2024-03-11 20:30 UTC is 2024-03-12 04:30 in Shanghai.
        assert_eq!(
            calendar
                .trading_date_of(utc_ms(2024, 3, 11, 20, 30))
                .unwrap(),
            TradingDate::new(2024, 3, 12).unwrap()
        );
        // 2024-03-11 03:00 UTC is 2024-03-10 23:00 in New York (EDT).
        let us = us_calendar();
        assert_eq!(
            us.trading_date_of(utc_ms(2024, 3, 11, 3, 0)).unwrap(),
            TradingDate::new(2024, 3, 10).unwrap()
        );
    }

    #[test]
    fn session_context_binds_snapshot_venue_time_zone_date_and_phase() {
        let calendar = a_share_calendar();
        let context = calendar
            .session_context_at(utc_ms(2024, 3, 11, 3, 30))
            .unwrap();
        assert_eq!(context.snapshot_id, "a-share-test-2024");
        assert_eq!(context.venue, Venue::china_a_share("sse").unwrap());
        assert_eq!(context.time_zone, "Asia/Shanghai");
        assert_eq!(context.trading_date, TradingDate::new(2024, 3, 11).unwrap());
        assert_eq!(context.phase, SessionPhase::Break);
        assert!(matches!(
            calendar
                .session_context_at(utc_ms(2025, 1, 1, 0, 0))
                .unwrap_err(),
            CalendarError::OutsideCalendarRange(_)
        ));
    }

    #[test]
    fn a_share_holidays_and_special_closures_are_calendar_evidence() {
        let calendar = a_share_calendar();
        // National Day holiday: 10:00 Shanghai on 2024-10-01.
        let holiday = utc_ms(2024, 10, 1, 2, 0);
        assert_eq!(
            calendar.session_phase_at(holiday).unwrap(),
            SessionPhase::Closed
        );
        assert!(calendar.is_scheduled_non_trading(holiday).unwrap());
        // Known special closure recorded as evidence: 10:00 Shanghai.
        let special = utc_ms(2024, 5, 13, 2, 0);
        assert_eq!(
            calendar.session_phase_at(special).unwrap(),
            SessionPhase::Closed
        );
        assert!(calendar.is_scheduled_non_trading(special).unwrap());
    }

    #[test]
    fn us_regular_session_shifts_across_daylight_saving() {
        let calendar = us_calendar();
        let standard = TradingDate::new(2024, 1, 8).unwrap();
        assert_eq!(
            calendar.daily_boundary_open_ms(standard).unwrap(),
            utc_ms(2024, 1, 8, 14, 30)
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 1, 8, 14, 30))
                .unwrap(),
            SessionPhase::Continuous
        );
        let daylight = TradingDate::new(2024, 7, 8).unwrap();
        assert_eq!(
            calendar.daily_boundary_open_ms(daylight).unwrap(),
            utc_ms(2024, 7, 8, 13, 30)
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 7, 8, 13, 30))
                .unwrap(),
            SessionPhase::Continuous
        );
        // 21:00 UTC ends the standard-day session, 20:00 UTC the DST session.
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 1, 8, 21, 0))
                .unwrap(),
            SessionPhase::Closed
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 7, 8, 20, 0))
                .unwrap(),
            SessionPhase::Closed
        );
    }

    #[test]
    fn us_early_close_is_calendar_evidence_not_a_gap() {
        let calendar = us_calendar();
        let early_close = TradingDate::new(2024, 7, 3).unwrap();
        assert_eq!(
            calendar.daily_boundary_open_ms(early_close).unwrap(),
            utc_ms(2024, 7, 3, 13, 30)
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 7, 3, 16, 0))
                .unwrap(),
            SessionPhase::Continuous
        );
        assert_eq!(
            calendar
                .session_phase_at(utc_ms(2024, 7, 3, 17, 30))
                .unwrap(),
            SessionPhase::Closed
        );
        assert!(
            calendar
                .is_scheduled_non_trading(utc_ms(2024, 7, 3, 18, 0))
                .unwrap()
        );
        assert_eq!(
            calendar.next_trading_date(early_close).unwrap(),
            TradingDate::new(2024, 7, 5).unwrap()
        );
    }

    #[test]
    fn us_fall_back_ambiguous_local_time_is_rejected_or_disambiguated() {
        let venue = Venue::us_equity("nasdaq").unwrap();
        let ambiguous = local(2024, 11, 3, 1, 30);
        let error = venue
            .resolve_local_time(ambiguous, LocalTimeDisambiguation::Reject)
            .unwrap_err();
        assert!(matches!(error, CalendarError::AmbiguousLocalTime(_, _)));
        assert_eq!(
            venue
                .resolve_local_time(ambiguous, LocalTimeDisambiguation::Earlier)
                .unwrap(),
            utc_ms(2024, 11, 3, 5, 30)
        );
        assert_eq!(
            venue
                .resolve_local_time(ambiguous, LocalTimeDisambiguation::Later)
                .unwrap(),
            utc_ms(2024, 11, 3, 6, 30)
        );
    }

    #[test]
    fn us_spring_forward_nonexistent_local_time_is_rejected() {
        let venue = Venue::us_equity("nasdaq").unwrap();
        let nonexistent = local(2024, 3, 10, 2, 30);
        for disambiguation in [
            LocalTimeDisambiguation::Reject,
            LocalTimeDisambiguation::Earlier,
            LocalTimeDisambiguation::Later,
        ] {
            let error = venue
                .resolve_local_time(nonexistent, disambiguation)
                .unwrap_err();
            assert!(matches!(error, CalendarError::NonexistentLocalTime(_, _)));
        }
    }

    #[test]
    fn crypto_utc_boundaries_and_maintenance_are_separate_evidence() {
        let open = utc_ms(2024, 1, 1, 0, 0);
        assert_eq!(
            crypto_trading_date(open).unwrap(),
            TradingDate::new(2024, 1, 1).unwrap()
        );
        let maintenance = vec![ScheduledClosure {
            kind: ScheduledClosureKind::Maintenance,
            start_ms: open,
            end_ms: open + DAY_MS,
            reason: Some("scheduled upgrade".to_owned()),
        }];
        assert_eq!(
            crypto_session_phase(open + 1, &maintenance).unwrap(),
            SessionPhase::Closed
        );
        assert_eq!(
            crypto_session_phase(open + DAY_MS, &maintenance).unwrap(),
            SessionPhase::Continuous
        );
        assert_eq!(
            crypto_session_phase(open, &[]).unwrap(),
            SessionPhase::Continuous
        );
    }

    #[test]
    fn utc_local_round_trips_preserve_the_local_identity() {
        for (venue, instant) in [
            (
                Venue::china_a_share("sse").unwrap(),
                utc_ms(2024, 3, 11, 2, 45),
            ),
            (
                Venue::us_equity("nasdaq").unwrap(),
                utc_ms(2024, 7, 8, 15, 45),
            ),
        ] {
            let local = venue.local_time(instant).unwrap();
            assert_eq!(
                venue
                    .resolve_local_time(local, LocalTimeDisambiguation::Reject)
                    .unwrap(),
                instant
            );
        }
        assert_eq!(
            VenueLocalTime::from_utc(
                &Venue::us_equity("nasdaq").unwrap(),
                utc_ms(2024, 7, 8, 13, 30)
            )
            .unwrap()
            .format_iso8601(),
            "2024-07-08T09:30:00"
        );
    }

    #[test]
    fn daily_bar_identity_uses_the_venue_trading_date() {
        let calendar = us_calendar();
        let winter = calendar
            .trading_date_of(utc_ms(2024, 1, 8, 14, 30))
            .unwrap();
        assert_eq!(winter, TradingDate::new(2024, 1, 8).unwrap());
        assert_eq!(
            calendar.daily_boundary_open_ms(winter).unwrap(),
            utc_ms(2024, 1, 8, 14, 30)
        );
        let summer = calendar
            .trading_date_of(utc_ms(2024, 7, 8, 13, 30))
            .unwrap();
        assert_eq!(summer, TradingDate::new(2024, 7, 8).unwrap());
        assert_eq!(
            calendar.daily_boundary_open_ms(summer).unwrap(),
            utc_ms(2024, 7, 8, 13, 30)
        );
        // A daily open resolves back to the same Venue-local Trading Date.
        for (date, open_ms) in [
            (
                TradingDate::new(2024, 1, 8).unwrap(),
                utc_ms(2024, 1, 8, 14, 30),
            ),
            (
                TradingDate::new(2024, 7, 8).unwrap(),
                utc_ms(2024, 7, 8, 13, 30),
            ),
        ] {
            assert_eq!(calendar.daily_boundary_open_ms(date).unwrap(), open_ms);
            assert_eq!(calendar.trading_date_of(open_ms).unwrap(), date);
        }
        // Crypto daily Bars keep their recorded UTC-grid identity: the open
        // instant maps to the UTC calendar date, not a session anchor.
        let identity =
            BarIdentity::from_legacy("okx", "BTC-USDT", BarInterval::OneDay, 1_704_067_200_000)
                .unwrap();
        assert_eq!(identity.open_time_ms, 1_704_067_200_000);
        assert_eq!(
            crypto_trading_date(identity.open_time_ms).unwrap(),
            TradingDate::new(2024, 1, 1).unwrap()
        );
    }

    #[test]
    fn scheduled_closures_are_not_bar_gaps_but_missing_session_bars_are() {
        let calendar = us_calendar();
        assert!(
            calendar
                .is_scheduled_non_trading(utc_ms(2024, 1, 1, 15, 0))
                .unwrap()
        );
        assert!(
            calendar
                .is_scheduled_non_trading(utc_ms(2024, 1, 6, 15, 0))
                .unwrap()
        );
        assert!(
            calendar
                .is_scheduled_non_trading(utc_ms(2024, 1, 8, 14, 0))
                .unwrap()
        );
        // Inside Continuous on a trading day a missing Bar is a genuine gap:
        // 14:30 UTC (session open) and 15:00 UTC (mid-session) are scheduled
        // trading windows, so an absent Bar there is missing data, not
        // calendar state.
        assert!(
            !calendar
                .is_scheduled_non_trading(utc_ms(2024, 1, 8, 14, 30))
                .unwrap()
        );
        assert!(
            !calendar
                .is_scheduled_non_trading(utc_ms(2024, 1, 8, 15, 0))
                .unwrap()
        );
        assert!(
            calendar
                .is_scheduled_non_trading(utc_ms(2024, 7, 3, 18, 0))
                .unwrap()
        );
        let weekend = weekend_calendar();
        assert!(
            weekend
                .is_scheduled_non_trading(utc_ms(2024, 1, 6, 15, 0))
                .unwrap()
        );
    }

    #[test]
    fn multi_day_intervals_count_consecutive_trading_dates() {
        let calendar = us_calendar();
        assert_eq!(
            calendar
                .trading_date_offset(TradingDate::new(2024, 1, 8).unwrap(), 1)
                .unwrap(),
            TradingDate::new(2024, 1, 9).unwrap()
        );
        assert_eq!(
            calendar
                .trading_date_offset(TradingDate::new(2024, 1, 5).unwrap(), 1)
                .unwrap(),
            TradingDate::new(2024, 1, 8).unwrap()
        );
        assert_eq!(
            calendar
                .trading_date_offset(TradingDate::new(2024, 1, 12).unwrap(), 1)
                .unwrap(),
            TradingDate::new(2024, 1, 16).unwrap()
        );
    }

    #[test]
    fn deserialization_revalidates_venue_and_calendar_invariants() {
        // A Venue whose payload time zone contradicts its class is rejected.
        let mismatched = r#"{"id":"bad","kind":"cryptoSpot","timeZone":"Asia/Shanghai"}"#;
        assert!(serde_json::from_str::<Venue>(mismatched).is_err());
        let unknown = r#"{"id":"bad","kind":"cryptoSpot","timeZone":"Not/AZone"}"#;
        assert!(serde_json::from_str::<Venue>(unknown).is_err());
        // A valid Venue round-trips through its canonical JSON shape.
        let venue = Venue::us_equity("nasdaq").unwrap();
        let json = serde_json::to_string(&venue).unwrap();
        assert_eq!(serde_json::from_str::<Venue>(&json).unwrap(), venue);

        // A Snapshot round-trips, and duplicate day evidence is rejected.
        let calendar = us_calendar();
        let json = serde_json::to_string(&calendar).unwrap();
        assert_eq!(
            serde_json::from_str::<TradingCalendarSnapshot>(&json).unwrap(),
            calendar
        );
        let day = DayEvidence::trading_day(TradingDate::new(2024, 3, 11).unwrap());
        let duplicate_days = serde_json::json!({
            "snapshotId": "dup",
            "venue": serde_json::to_value(Venue::us_equity("nasdaq").unwrap()).unwrap(),
            "effectiveFromMs": 0,
            "effectiveToMs": DAY_MS * 400,
            "defaultSessions": [],
            "days": [serde_json::to_value(&day).unwrap(), serde_json::to_value(&day).unwrap()]
        });
        assert!(serde_json::from_value::<TradingCalendarSnapshot>(duplicate_days).is_err());
    }

    #[test]
    fn invalid_mappings_are_rejected() {
        assert!(matches!(
            Venue::us_equity("").unwrap_err(),
            CalendarError::InvalidVenue(_)
        ));
        assert!(matches!(
            InstrumentId::new(Venue::crypto_spot("okx").unwrap(), " ").unwrap_err(),
            CalendarError::InvalidInstrument(_)
        ));
        assert!(matches!(
            BarIdentity::from_legacy("alpaca", "AAPL", BarInterval::OneDay, 0).unwrap_err(),
            CalendarError::UnsupportedSource(_)
        ));
        assert_eq!(
            BarIdentity::from_legacy("okx", "BTC-USDT", BarInterval::OneDay, 1_704_067_200_000)
                .unwrap()
                .instrument,
            InstrumentId::new(Venue::crypto_spot("okx").unwrap(), "BTC-USDT").unwrap()
        );
        assert!(matches!(
            calendar_error_from_duplicate_days(),
            CalendarError::InvalidCalendar(_)
        ));
    }

    fn calendar_error_from_duplicate_days() -> CalendarError {
        let day = DayEvidence::trading_day(TradingDate::new(2024, 3, 11).unwrap());
        TradingCalendarSnapshot::new(
            "dup",
            Venue::us_equity("nasdaq").unwrap(),
            0,
            DAY_MS * 400,
            us_sessions(),
            vec![day.clone(), day],
        )
        .unwrap_err()
    }
}
