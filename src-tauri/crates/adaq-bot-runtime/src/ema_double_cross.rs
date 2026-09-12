use rust_decimal::Decimal;
use std::{
    collections::{HashSet, VecDeque},
    fmt,
};

const MAX_EVIDENCE_IDS: usize = 256;
const MAX_TRADE_IDS: usize = 100_000;

pub const EMA_FAST_PERIOD: u32 = 5;
pub const EMA_SLOW_PERIOD: u32 = 10;
pub const EMA_BAR_INTERVAL_MS: i64 = 900_000;
pub const EMA_CONFIRMATION_MS: i64 = 60_000;
pub const EMA_DECISION_MODE: &str = "ema-double-cross-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmaDoubleCrossConfig {
    pub fast_period: u32,
    pub slow_period: u32,
    pub bar_interval_ms: i64,
    pub confirmation_ms: i64,
}

impl EmaDoubleCrossConfig {
    fn validate(&self) -> Result<(), EmaError> {
        if self.fast_period == 0
            || self.slow_period <= self.fast_period
            || self.bar_interval_ms <= 0
            || self.confirmation_ms <= 0
        {
            return Err(EmaError::InvalidConfig);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmaTrade {
    pub trade_id: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub observed_at_ms: i64,
    pub available_at_ms: i64,
    pub received_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmedBar {
    pub bar_open_time_ms: i64,
    pub close: Decimal,
    pub observed_at_ms: i64,
    pub available_at_ms: i64,
    pub evidence_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmaEvent {
    Connected { at_ms: i64 },
    Disconnected { at_ms: i64 },
    BarClosed(ConfirmedBar),
    Trade(EmaTrade),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmaRelation {
    Below,
    Equal,
    Above,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmaSignal {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmaBlockReason {
    Disconnected,
    Warmup,
    Gap,
    Late,
    OutOfOrder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmaIgnoreReason {
    DuplicateTrade,
    DuplicateBar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmaObservation {
    pub trade_id: String,
    pub price: Decimal,
    pub observed_at_ms: i64,
    pub fast_ema: Decimal,
    pub slow_ema: Decimal,
    pub relation: EmaRelation,
    pub signal: Option<EmaSignal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmaProcessOutcome {
    Connected,
    Disconnected,
    BarAccepted {
        bar_open_time_ms: i64,
        fast_ema: Option<Decimal>,
        slow_ema: Option<Decimal>,
    },
    Observation(EmaObservation),
    Ignored(EmaIgnoreReason),
    Blocked {
        reason: EmaBlockReason,
        detail: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmaError {
    InvalidConfig,
    InvalidEvent(&'static str),
    WrongInstrument,
}

impl fmt::Display for EmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => write!(f, "EMA double-cross configuration is invalid"),
            Self::InvalidEvent(detail) => write!(f, "EMA double-cross event is invalid: {detail}"),
            Self::WrongInstrument => write!(f, "EMA double-cross event instrument is invalid"),
        }
    }
}

impl std::error::Error for EmaError {}

#[derive(Clone, Debug)]
struct EmaAccumulator {
    period: u32,
    seed: Vec<Decimal>,
    last_confirmed: Option<Decimal>,
}

impl EmaAccumulator {
    fn new(period: u32) -> Self {
        Self {
            period,
            seed: Vec::new(),
            last_confirmed: None,
        }
    }

    fn reset(&mut self) {
        self.seed.clear();
        self.last_confirmed = None;
    }

    fn push_confirmed(&mut self, close: Decimal) -> Decimal {
        if self.last_confirmed.is_none() {
            self.seed.push(close);
            if self.seed.len() < self.period as usize {
                return close;
            }
            let sum = self
                .seed
                .iter()
                .copied()
                .fold(Decimal::ZERO, |sum, value| sum + value);
            let seeded = sum / Decimal::from(self.period);
            self.last_confirmed = Some(seeded);
            return seeded;
        }

        let previous = self.last_confirmed.expect("checked above");
        let smoothing = Decimal::from(2) / Decimal::from(self.period + 1);
        let next = previous + (close - previous) * smoothing;
        self.last_confirmed = Some(next);
        next
    }

    fn provisional(&self, price: Decimal) -> Option<Decimal> {
        self.last_confirmed.map(|previous| {
            let smoothing = Decimal::from(2) / Decimal::from(self.period + 1);
            previous + (price - previous) * smoothing
        })
    }
}

#[derive(Clone, Debug)]
enum BuyPhase {
    AwaitingFirstCross,
    ConfirmingFirst {
        start_ms: i64,
        reference_high: Decimal,
    },
    WaitingPullback {
        reference_high: Decimal,
    },
    AwaitingSecondBreak {
        reference_high: Decimal,
    },
    ConfirmingSecond {
        start_ms: i64,
        reference_high: Decimal,
    },
}

#[derive(Clone, Debug)]
enum SellPhase {
    AwaitingFirstCross,
    ConfirmingFirst {
        start_ms: i64,
        reference_low: Decimal,
    },
    WaitingRally {
        reference_low: Decimal,
    },
    AwaitingSecondBreak {
        reference_low: Decimal,
    },
    ConfirmingSecond {
        start_ms: i64,
        reference_low: Decimal,
    },
}

pub struct EmaDoubleCrossEngine {
    config: EmaDoubleCrossConfig,
    instrument_id: String,
    fast: EmaAccumulator,
    slow: EmaAccumulator,
    connected: bool,
    owned_position: bool,
    current_bar_open_ms: Option<i64>,
    last_trade_observed_ms: Option<i64>,
    last_relation: Option<EmaRelation>,
    buy_phase: BuyPhase,
    sell_phase: SellPhase,
    seen_trade_ids: HashSet<String>,
    trade_id_order: VecDeque<String>,
    seen_evidence_ids: HashSet<String>,
    evidence_id_order: VecDeque<String>,
}

impl EmaDoubleCrossEngine {
    pub fn new(
        config: EmaDoubleCrossConfig,
        instrument_id: impl Into<String>,
    ) -> Result<Self, EmaError> {
        config.validate()?;
        let instrument_id = instrument_id.into();
        if instrument_id.trim().is_empty() || instrument_id.len() > 128 {
            return Err(EmaError::WrongInstrument);
        }
        Ok(Self {
            fast: EmaAccumulator::new(config.fast_period),
            slow: EmaAccumulator::new(config.slow_period),
            config,
            instrument_id,
            connected: true,
            owned_position: false,
            current_bar_open_ms: None,
            last_trade_observed_ms: None,
            last_relation: None,
            buy_phase: BuyPhase::AwaitingFirstCross,
            sell_phase: SellPhase::AwaitingFirstCross,
            seen_trade_ids: HashSet::new(),
            trade_id_order: VecDeque::new(),
            seen_evidence_ids: HashSet::new(),
            evidence_id_order: VecDeque::new(),
        })
    }

    pub fn set_owned_position(&mut self, owned: bool) {
        if self.owned_position != owned {
            self.owned_position = owned;
            self.reset_signal_state();
        }
    }

    pub fn owned_position(&self) -> bool {
        self.owned_position
    }

    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub fn current_bar_open_time_ms(&self) -> Option<i64> {
        self.current_bar_open_ms
    }

    pub fn process(&mut self, event: EmaEvent) -> Result<EmaProcessOutcome, EmaError> {
        match event {
            EmaEvent::Connected { at_ms } => {
                if at_ms < 0 {
                    return Err(EmaError::InvalidEvent("connection time is negative"));
                }
                self.connected = true;
                self.reset_analysis();
                Ok(EmaProcessOutcome::Connected)
            }
            EmaEvent::Disconnected { at_ms } => {
                if at_ms < 0 {
                    return Err(EmaError::InvalidEvent("disconnection time is negative"));
                }
                self.connected = false;
                self.reset_analysis();
                Ok(EmaProcessOutcome::Disconnected)
            }
            EmaEvent::BarClosed(bar) => self.process_bar(bar),
            EmaEvent::Trade(trade) => self.process_trade(trade),
        }
    }

    fn process_bar(&mut self, bar: ConfirmedBar) -> Result<EmaProcessOutcome, EmaError> {
        if !self.connected {
            return Ok(EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Disconnected,
                detail: "trade stream is disconnected",
            });
        }
        if bar.bar_open_time_ms < 0
            || bar.bar_open_time_ms % self.config.bar_interval_ms != 0
            || bar.close <= Decimal::ZERO
            || bar.evidence_id.trim().is_empty()
            || bar.evidence_id.len() > 256
            || bar.observed_at_ms < 0
            || bar.available_at_ms < bar.observed_at_ms
        {
            return Err(EmaError::InvalidEvent("confirmed bar metadata is invalid"));
        }
        let bar_close_ms = bar
            .bar_open_time_ms
            .checked_add(self.config.bar_interval_ms)
            .ok_or(EmaError::InvalidEvent("bar close time overflowed"))?;
        if bar.observed_at_ms < bar_close_ms {
            return Err(EmaError::InvalidEvent(
                "confirmed bar was observed before its close",
            ));
        }
        if self.seen_evidence_ids.contains(&bar.evidence_id) {
            return Ok(EmaProcessOutcome::Ignored(EmaIgnoreReason::DuplicateBar));
        }

        let mut gap = false;
        if let Some(expected_open_ms) = self.current_bar_open_ms {
            if bar.bar_open_time_ms < expected_open_ms {
                self.reset_analysis();
                return Ok(EmaProcessOutcome::Blocked {
                    reason: EmaBlockReason::Late,
                    detail: "confirmed bar arrived before the active bar",
                });
            }
            if bar.bar_open_time_ms > expected_open_ms {
                gap = true;
                self.reset_analysis();
            }
        }

        self.fast.push_confirmed(bar.close);
        self.slow.push_confirmed(bar.close);
        self.current_bar_open_ms = Some(bar_close_ms);
        self.remember_evidence_id(bar.evidence_id);
        let outcome = EmaProcessOutcome::BarAccepted {
            bar_open_time_ms: bar.bar_open_time_ms,
            fast_ema: self.fast.last_confirmed,
            slow_ema: self.slow.last_confirmed,
        };
        if gap {
            Ok(EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Gap,
                detail: "confirmed bar continuity was broken; fresh warmup started",
            })
        } else {
            Ok(outcome)
        }
    }

    fn process_trade(&mut self, trade: EmaTrade) -> Result<EmaProcessOutcome, EmaError> {
        if !self.connected {
            return Ok(EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Disconnected,
                detail: "trade stream is disconnected",
            });
        }
        if trade.trade_id.trim().is_empty()
            || trade.trade_id.len() > 256
            || trade.price <= Decimal::ZERO
            || trade.quantity <= Decimal::ZERO
            || trade.observed_at_ms < 0
            || trade.available_at_ms < trade.observed_at_ms
            || trade.received_at_ms < trade.available_at_ms
        {
            return Err(EmaError::InvalidEvent("trade metadata is invalid"));
        }
        if self.seen_trade_ids.contains(&trade.trade_id) {
            return Ok(EmaProcessOutcome::Ignored(EmaIgnoreReason::DuplicateTrade));
        }
        if self
            .last_trade_observed_ms
            .is_some_and(|last| trade.observed_at_ms < last)
        {
            self.reset_analysis();
            return Ok(EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::OutOfOrder,
                detail: "trade observation time moved backwards",
            });
        }
        let Some(current_bar_open_ms) = self.current_bar_open_ms else {
            return Ok(EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Warmup,
                detail: "no confirmed bar is available for the trade",
            });
        };
        let current_bar_close_ms = current_bar_open_ms
            .checked_add(self.config.bar_interval_ms)
            .ok_or(EmaError::InvalidEvent("active bar close time overflowed"))?;
        if trade.observed_at_ms < current_bar_open_ms {
            self.reset_analysis();
            return Ok(EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Late,
                detail: "trade arrived before the active bar",
            });
        }
        if trade.observed_at_ms >= current_bar_close_ms {
            self.reset_analysis();
            return Ok(EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Gap,
                detail: "trade arrived after an unobserved bar close",
            });
        }

        self.remember_trade_id(trade.trade_id.clone());
        self.last_trade_observed_ms = Some(trade.observed_at_ms);
        let Some(fast_ema) = self.fast.provisional(trade.price) else {
            return Ok(EmaProcessOutcome::Observation(EmaObservation {
                trade_id: trade.trade_id,
                price: trade.price,
                observed_at_ms: trade.observed_at_ms,
                fast_ema: Decimal::ZERO,
                slow_ema: Decimal::ZERO,
                relation: EmaRelation::Equal,
                signal: None,
            }));
        };
        let Some(slow_ema) = self.slow.provisional(trade.price) else {
            return Ok(EmaProcessOutcome::Observation(EmaObservation {
                trade_id: trade.trade_id,
                price: trade.price,
                observed_at_ms: trade.observed_at_ms,
                fast_ema,
                slow_ema: Decimal::ZERO,
                relation: EmaRelation::Equal,
                signal: None,
            }));
        };
        let relation = relation(fast_ema, slow_ema);
        let signal = if self.owned_position {
            self.advance_sell(relation, trade.price, trade.observed_at_ms)
        } else {
            self.advance_buy(relation, trade.price, trade.observed_at_ms)
        };
        self.last_relation = Some(relation);
        Ok(EmaProcessOutcome::Observation(EmaObservation {
            trade_id: trade.trade_id,
            price: trade.price,
            observed_at_ms: trade.observed_at_ms,
            fast_ema,
            slow_ema,
            relation,
            signal,
        }))
    }

    fn advance_buy(
        &mut self,
        current: EmaRelation,
        price: Decimal,
        at_ms: i64,
    ) -> Option<EmaSignal> {
        let previous = self.last_relation;
        let crossed_up = crosses_up(previous, current);
        let phase = self.buy_phase.clone();
        match phase {
            BuyPhase::AwaitingFirstCross => {
                if crossed_up {
                    self.buy_phase = BuyPhase::ConfirmingFirst {
                        start_ms: at_ms,
                        reference_high: price,
                    };
                }
            }
            BuyPhase::ConfirmingFirst {
                start_ms,
                reference_high,
            } => {
                if current != EmaRelation::Above {
                    self.buy_phase = BuyPhase::AwaitingFirstCross;
                } else {
                    let reference_high = reference_high.max(price);
                    if at_ms.saturating_sub(start_ms) >= self.config.confirmation_ms {
                        self.buy_phase = BuyPhase::WaitingPullback { reference_high };
                    } else {
                        self.buy_phase = BuyPhase::ConfirmingFirst {
                            start_ms,
                            reference_high,
                        };
                    }
                }
            }
            BuyPhase::WaitingPullback { reference_high } => {
                if crosses_down(previous, current) {
                    self.buy_phase = BuyPhase::AwaitingSecondBreak { reference_high };
                }
            }
            BuyPhase::AwaitingSecondBreak { reference_high } => {
                if current == EmaRelation::Above && price > reference_high {
                    self.buy_phase = BuyPhase::ConfirmingSecond {
                        start_ms: at_ms,
                        reference_high,
                    };
                }
            }
            BuyPhase::ConfirmingSecond {
                start_ms,
                reference_high,
            } => {
                if current != EmaRelation::Above {
                    self.buy_phase = BuyPhase::AwaitingFirstCross;
                } else if price <= reference_high {
                    self.buy_phase = BuyPhase::AwaitingSecondBreak { reference_high };
                } else if at_ms.saturating_sub(start_ms) >= self.config.confirmation_ms {
                    self.buy_phase = BuyPhase::AwaitingFirstCross;
                    return Some(EmaSignal::Buy);
                }
            }
        }
        None
    }

    fn advance_sell(
        &mut self,
        current: EmaRelation,
        price: Decimal,
        at_ms: i64,
    ) -> Option<EmaSignal> {
        let previous = self.last_relation;
        let crossed_down = crosses_down(previous, current);
        let phase = self.sell_phase.clone();
        match phase {
            SellPhase::AwaitingFirstCross => {
                if crossed_down {
                    self.sell_phase = SellPhase::ConfirmingFirst {
                        start_ms: at_ms,
                        reference_low: price,
                    };
                }
            }
            SellPhase::ConfirmingFirst {
                start_ms,
                reference_low,
            } => {
                if current != EmaRelation::Below {
                    self.sell_phase = SellPhase::AwaitingFirstCross;
                } else {
                    let reference_low = reference_low.min(price);
                    if at_ms.saturating_sub(start_ms) >= self.config.confirmation_ms {
                        self.sell_phase = SellPhase::WaitingRally { reference_low };
                    } else {
                        self.sell_phase = SellPhase::ConfirmingFirst {
                            start_ms,
                            reference_low,
                        };
                    }
                }
            }
            SellPhase::WaitingRally { reference_low } => {
                if crosses_up(previous, current) {
                    self.sell_phase = SellPhase::AwaitingSecondBreak { reference_low };
                }
            }
            SellPhase::AwaitingSecondBreak { reference_low } => {
                if current == EmaRelation::Below && price < reference_low {
                    self.sell_phase = SellPhase::ConfirmingSecond {
                        start_ms: at_ms,
                        reference_low,
                    };
                }
            }
            SellPhase::ConfirmingSecond {
                start_ms,
                reference_low,
            } => {
                if current != EmaRelation::Below {
                    self.sell_phase = SellPhase::AwaitingFirstCross;
                } else if price >= reference_low {
                    self.sell_phase = SellPhase::AwaitingSecondBreak { reference_low };
                } else if at_ms.saturating_sub(start_ms) >= self.config.confirmation_ms {
                    self.sell_phase = SellPhase::AwaitingFirstCross;
                    return Some(EmaSignal::Sell);
                }
            }
        }
        None
    }

    fn reset_signal_state(&mut self) {
        self.last_relation = None;
        self.buy_phase = BuyPhase::AwaitingFirstCross;
        self.sell_phase = SellPhase::AwaitingFirstCross;
    }

    fn reset_analysis(&mut self) {
        self.fast.reset();
        self.slow.reset();
        self.current_bar_open_ms = None;
        self.last_trade_observed_ms = None;
        self.reset_signal_state();
    }

    fn remember_trade_id(&mut self, trade_id: String) {
        self.seen_trade_ids.insert(trade_id.clone());
        self.trade_id_order.push_back(trade_id);
        while self.trade_id_order.len() > MAX_TRADE_IDS {
            if let Some(oldest) = self.trade_id_order.pop_front() {
                self.seen_trade_ids.remove(&oldest);
            }
        }
    }

    fn remember_evidence_id(&mut self, evidence_id: String) {
        self.seen_evidence_ids.insert(evidence_id.clone());
        self.evidence_id_order.push_back(evidence_id);
        while self.evidence_id_order.len() > MAX_EVIDENCE_IDS {
            if let Some(oldest) = self.evidence_id_order.pop_front() {
                self.seen_evidence_ids.remove(&oldest);
            }
        }
    }
}

fn relation(fast: Decimal, slow: Decimal) -> EmaRelation {
    match fast.cmp(&slow) {
        std::cmp::Ordering::Less => EmaRelation::Below,
        std::cmp::Ordering::Equal => EmaRelation::Equal,
        std::cmp::Ordering::Greater => EmaRelation::Above,
    }
}

fn crosses_up(previous: Option<EmaRelation>, current: EmaRelation) -> bool {
    matches!(
        (previous, current),
        (
            Some(EmaRelation::Below | EmaRelation::Equal),
            EmaRelation::Above
        )
    )
}

fn crosses_down(previous: Option<EmaRelation>, current: EmaRelation) -> bool {
    matches!(
        (previous, current),
        (
            Some(EmaRelation::Above | EmaRelation::Equal),
            EmaRelation::Below
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> EmaDoubleCrossConfig {
        EmaDoubleCrossConfig {
            fast_period: 1,
            slow_period: 2,
            bar_interval_ms: 900_000,
            confirmation_ms: 60_000,
        }
    }

    fn trade(id: &str, price: i64, at_ms: i64) -> EmaEvent {
        EmaEvent::Trade(EmaTrade {
            trade_id: id.into(),
            price: Decimal::from(price),
            quantity: Decimal::ONE,
            observed_at_ms: at_ms,
            available_at_ms: at_ms,
            received_at_ms: at_ms,
        })
    }

    fn seeded_engine() -> EmaDoubleCrossEngine {
        let mut engine = EmaDoubleCrossEngine::new(config(), "BTC-USDT").unwrap();
        for (index, close) in [10, 10].into_iter().enumerate() {
            engine
                .process(EmaEvent::BarClosed(ConfirmedBar {
                    bar_open_time_ms: index as i64 * 900_000,
                    close: Decimal::from(close),
                    observed_at_ms: (index as i64 + 1) * 900_000,
                    available_at_ms: (index as i64 + 1) * 900_000,
                    evidence_id: format!("bar-{index}"),
                }))
                .unwrap();
        }
        engine
    }

    fn observation(
        engine: &mut EmaDoubleCrossEngine,
        id: &str,
        price: i64,
        at_ms: i64,
    ) -> EmaObservation {
        match engine.process(trade(id, price, at_ms)).unwrap() {
            EmaProcessOutcome::Observation(observation) => observation,
            other => panic!("expected observation, got {other:?}"),
        }
    }

    #[test]
    fn provisional_ema_uses_the_last_confirmed_bar_for_each_trade() {
        let mut engine = seeded_engine();
        let first = observation(&mut engine, "trade-1", 20, 1_800_001);
        let second = observation(&mut engine, "trade-2", 30, 1_800_002);

        assert_eq!(first.fast_ema, Decimal::from(20));
        assert_eq!(first.slow_ema, Decimal::from(50) / Decimal::from(3));
        assert_eq!(second.fast_ema, Decimal::from(30));
        assert_eq!(
            second.slow_ema,
            Decimal::from(10)
                + (Decimal::from(30) - Decimal::from(10)) * (Decimal::from(2) / Decimal::from(3))
        );
    }

    #[test]
    fn buy_requires_two_strict_confirmations_and_reference_breakout() {
        let mut engine = seeded_engine();
        assert_eq!(
            observation(&mut engine, "below", 9, 1_800_001).relation,
            EmaRelation::Below
        );
        assert_eq!(
            observation(&mut engine, "cross-up", 11, 1_800_002).signal,
            None
        );
        assert_eq!(
            observation(&mut engine, "first-end", 12, 1_860_002).signal,
            None
        );
        assert_eq!(
            observation(&mut engine, "cross-down", 8, 1_860_003).relation,
            EmaRelation::Below
        );
        assert_eq!(
            observation(&mut engine, "second-start", 13, 1_860_004).signal,
            None
        );
        assert_eq!(
            observation(&mut engine, "second-end", 14, 1_920_004).signal,
            Some(EmaSignal::Buy)
        );
    }

    #[test]
    fn sell_mirrors_the_buy_path_for_an_owned_position() {
        let mut engine = seeded_engine();
        engine.set_owned_position(true);
        observation(&mut engine, "above", 11, 1_800_001);
        assert_eq!(
            observation(&mut engine, "cross-down", 9, 1_800_002).signal,
            None
        );
        observation(&mut engine, "first-end", 8, 1_860_002);
        observation(&mut engine, "cross-up", 12, 1_860_003);
        observation(&mut engine, "second-start", 7, 1_860_004);
        assert_eq!(
            observation(&mut engine, "second-end", 6, 1_920_004).signal,
            Some(EmaSignal::Sell)
        );
    }

    #[test]
    fn intrabar_order_changes_provisional_relations_with_identical_bar_ohlc() {
        let mut first = seeded_engine();
        let mut second = seeded_engine();
        let first_relations = [
            observation(&mut first, "first-low", 9, 1_800_001).relation,
            observation(&mut first, "first-high", 11, 1_800_002).relation,
        ];
        let second_relations = [
            observation(&mut second, "second-high", 11, 1_800_001).relation,
            observation(&mut second, "second-low", 9, 1_800_002).relation,
        ];

        assert_eq!(first_relations, [EmaRelation::Below, EmaRelation::Above]);
        assert_eq!(second_relations, [EmaRelation::Above, EmaRelation::Below]);
    }

    #[test]
    fn equality_breaks_a_confirmation_and_duplicate_or_late_data_is_blocked() {
        let mut engine = seeded_engine();
        observation(&mut engine, "cross-up", 11, 1_800_001);
        let equal = observation(&mut engine, "equal", 10, 1_830_001);
        assert_eq!(equal.relation, EmaRelation::Equal);
        assert_eq!(
            engine.process(trade("equal", 10, 1_830_002)).unwrap(),
            EmaProcessOutcome::Ignored(EmaIgnoreReason::DuplicateTrade)
        );
        assert!(matches!(
            engine.process(trade("late", 9, 1_800_000)).unwrap(),
            EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::OutOfOrder,
                ..
            }
        ));
    }

    #[test]
    fn a_gap_or_disconnect_requires_fresh_warmup() {
        let mut engine = seeded_engine();
        let gap = engine
            .process(EmaEvent::BarClosed(ConfirmedBar {
                bar_open_time_ms: 2_700_000,
                close: Decimal::from(10),
                observed_at_ms: 3_600_000,
                available_at_ms: 3_600_000,
                evidence_id: "bar-gap".into(),
            }))
            .unwrap();
        assert!(matches!(
            gap,
            EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Gap,
                ..
            }
        ));
        assert!(matches!(
            engine
                .process(trade("trade-during-fresh-warmup", 11, 3_600_001))
                .unwrap(),
            EmaProcessOutcome::Observation(_)
        ));
        engine
            .process(EmaEvent::BarClosed(ConfirmedBar {
                bar_open_time_ms: 3_600_000,
                close: Decimal::from(10),
                observed_at_ms: 4_500_000,
                available_at_ms: 4_500_000,
                evidence_id: "bar-recovery".into(),
            }))
            .unwrap();
        assert!(matches!(
            engine
                .process(trade("trade-after-gap", 11, 4_500_001))
                .unwrap(),
            EmaProcessOutcome::Observation(_)
        ));
        engine
            .process(EmaEvent::Disconnected { at_ms: 2_700_002 })
            .unwrap();
        assert!(matches!(
            engine
                .process(trade("disconnected", 11, 2_700_003))
                .unwrap(),
            EmaProcessOutcome::Blocked {
                reason: EmaBlockReason::Disconnected,
                ..
            }
        ));
    }

    #[test]
    fn confirmed_bars_must_start_on_the_utc_grid() {
        let mut engine = EmaDoubleCrossEngine::new(config(), "BTC-USDT").unwrap();
        let error = engine
            .process(EmaEvent::BarClosed(ConfirmedBar {
                bar_open_time_ms: 1,
                close: Decimal::ONE,
                observed_at_ms: 900_001,
                available_at_ms: 900_001,
                evidence_id: "off-grid".into(),
            }))
            .unwrap_err();
        assert_eq!(
            error,
            EmaError::InvalidEvent("confirmed bar metadata is invalid")
        );
    }
}
