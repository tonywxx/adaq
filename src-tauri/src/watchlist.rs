use std::{path::Path, sync::Mutex};

use adaq_data_core::{
    BarInterval,
    market::{Venue, VenueKind},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const DEFAULT_SRC: &str = "okx";
const DEFAULT_ACTIVE_CODE: &str = "BTC-USDT";
const DEFAULT_CRYPTO_CODES: [&str; 3] = ["BTC-USDT", "ETH-USDT", "SOL-USDT"];
const DEFAULT_A_SHARE_CODES: [(&str, &str); 6] = [
    ("600519", "sse"),
    ("601318", "sse"),
    ("510500", "sse"),
    ("000333", "szse"),
    ("588000", "sse"),
    ("688981", "sse"),
];
const DEFAULT_US_CODES: [&str; 7] = ["NVDA", "TSLA", "AAPL", "GOOGL", "MSFT", "AMZN", "META"];
const WATCHLIST_DEFAULTS_VERSION: i64 = 1;
const WATCHLIST_LIMIT: i64 = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentRef {
    pub src: String,
    pub code: String,
    #[serde(default)]
    pub venue: Option<Venue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistState {
    pub items: Vec<InstrumentRef>,
    pub active_instrument: InstrumentRef,
    pub mini_chart_interval: BarInterval,
    pub limit: i64,
}

pub struct WatchlistDb(Mutex<Connection>);

pub fn validate_provider_venue(instrument: &InstrumentRef) -> Result<(), String> {
    let expected_kind = match instrument.src.as_str() {
        "okx" => VenueKind::CryptoSpot,
        "akshare-rs" => VenueKind::ChinaAShareEquity,
        "alpaca" => VenueKind::UsEquity,
        _ => return Err("unsupported Market Data Provider for Watchlist Instrument".to_owned()),
    };
    let Some(venue) = instrument.venue.as_ref() else {
        return (instrument.src == DEFAULT_SRC)
            .then_some(())
            .ok_or_else(|| "a canonical Venue is required for this Instrument".to_owned());
    };
    if venue.kind != expected_kind {
        return Err("Watchlist provider and Venue do not match".to_owned());
    }
    Ok(())
}

impl WatchlistDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, String> {
        connection
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                CREATE TABLE IF NOT EXISTS watchlist_settings (
                    user_id TEXT PRIMARY KEY,
                    active_src TEXT NOT NULL,
                    active_code TEXT NOT NULL,
                    active_venue_json TEXT,
                    mini_chart_interval TEXT NOT NULL,
                    watchlist_defaults_version INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS watchlist_items (
                    user_id TEXT NOT NULL,
                    src TEXT NOT NULL,
                    code TEXT NOT NULL,
                    venue_json TEXT,
                    position INTEGER NOT NULL,
                    FOREIGN KEY (user_id) REFERENCES watchlist_settings(user_id)
                        ON DELETE CASCADE
                );
                ",
            )
            .map_err(|error| error.to_string())?;
        ensure_column(
            &connection,
            "watchlist_settings",
            "active_venue_json",
            "TEXT",
        )?;
        ensure_column(
            &connection,
            "watchlist_settings",
            "watchlist_defaults_version",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(&connection, "watchlist_items", "venue_json", "TEXT")?;
        migrate_legacy_item_identity(&connection)?;
        connection
            .execute(
                "CREATE INDEX IF NOT EXISTS watchlist_items_order
                 ON watchlist_items(user_id, position)",
                [],
            )
            .map_err(|error| error.to_string())?;
        let default_venue_json =
            serde_json::to_string(&default_venue()).map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE watchlist_settings
                 SET active_venue_json = ?1
                 WHERE active_venue_json IS NULL AND active_src = 'okx'",
                [&default_venue_json],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE watchlist_items
                 SET venue_json = ?1
                 WHERE venue_json IS NULL AND src = 'okx'",
                [&default_venue_json],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS watchlist_items_identity
                 ON watchlist_items(user_id, venue_json, code)
                 WHERE venue_json IS NOT NULL",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(Self(Mutex::new(connection)))
    }

    pub fn get(&self, user_id: &str) -> Result<WatchlistState, String> {
        validate_user_id(user_id)?;
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        ensure_account(&mut connection, user_id)?;
        load_state(&connection, user_id)
    }

    pub fn add(&self, user_id: &str, instrument: &InstrumentRef) -> Result<WatchlistState, String> {
        validate_user_id(user_id)?;
        let instrument = canonicalize_instrument(instrument)?;
        let venue_json = serde_json::to_string(
            instrument
                .venue
                .as_ref()
                .expect("canonical instruments always have a Venue"),
        )
        .map_err(|error| error.to_string())?;
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        ensure_account(&mut connection, user_id)?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM watchlist_items WHERE user_id = ?1",
                [user_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if count >= WATCHLIST_LIMIT {
            return Err(format!(
                "Watchlist is limited to {WATCHLIST_LIMIT} instruments"
            ));
        }
        let position: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(position) + 1, 0)
                 FROM watchlist_items WHERE user_id = ?1",
                [user_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO watchlist_items(user_id, src, code, venue_json, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    user_id,
                    instrument.src,
                    instrument.code,
                    venue_json,
                    position
                ],
            )
            .map_err(|error| {
                if error.to_string().contains("UNIQUE constraint failed") {
                    "Instrument is already in the Watchlist".to_owned()
                } else {
                    error.to_string()
                }
            })?;
        load_state(&connection, user_id)
    }

    pub fn remove(
        &self,
        user_id: &str,
        instrument: &InstrumentRef,
    ) -> Result<WatchlistState, String> {
        validate_user_id(user_id)?;
        let instrument = canonicalize_instrument(instrument)?;
        let venue_json = serde_json::to_string(
            instrument
                .venue
                .as_ref()
                .expect("canonical instruments always have a Venue"),
        )
        .map_err(|error| error.to_string())?;
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        ensure_account(&mut connection, user_id)?;
        connection
            .execute(
                "DELETE FROM watchlist_items
                 WHERE user_id = ?1 AND venue_json = ?2 AND code = ?3",
                params![user_id, venue_json, instrument.code],
            )
            .map_err(|error| error.to_string())?;
        let (active_src, active_code, active_venue_json): (String, String, Option<String>) =
            connection
                .query_row(
                    "SELECT active_src, active_code, active_venue_json
                 FROM watchlist_settings WHERE user_id = ?1",
                    [user_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(|error| error.to_string())?;
        if active_venue_json.as_deref() == Some(&venue_json)
            || (active_venue_json.is_none()
                && active_src == instrument.src
                && active_code == instrument.code)
        {
            let next = first_item(&connection, user_id)?.unwrap_or_else(default_active);
            let next_venue_json = serde_json::to_string(
                next.venue
                    .as_ref()
                    .expect("canonical instruments always have a Venue"),
            )
            .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE watchlist_settings
                     SET active_src = ?2, active_code = ?3, active_venue_json = ?4
                     WHERE user_id = ?1",
                    params![user_id, next.src, next.code, next_venue_json],
                )
                .map_err(|error| error.to_string())?;
        }
        load_state(&connection, user_id)
    }

    pub fn set_active(
        &self,
        user_id: &str,
        instrument: &InstrumentRef,
    ) -> Result<WatchlistState, String> {
        validate_user_id(user_id)?;
        let instrument = canonicalize_instrument(instrument)?;
        let venue_json = serde_json::to_string(
            instrument
                .venue
                .as_ref()
                .expect("canonical instruments always have a Venue"),
        )
        .map_err(|error| error.to_string())?;
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        ensure_account(&mut connection, user_id)?;
        connection
            .execute(
                "UPDATE watchlist_settings
                 SET active_src = ?2, active_code = ?3, active_venue_json = ?4
                 WHERE user_id = ?1",
                params![user_id, instrument.src, instrument.code, venue_json],
            )
            .map_err(|error| error.to_string())?;
        load_state(&connection, user_id)
    }

    pub fn set_interval(
        &self,
        user_id: &str,
        interval: BarInterval,
    ) -> Result<WatchlistState, String> {
        validate_user_id(user_id)?;
        if !matches!(
            interval,
            BarInterval::OneMinute
                | BarInterval::FiveMinutes
                | BarInterval::FifteenMinutes
                | BarInterval::OneHour
                | BarInterval::FourHours
                | BarInterval::OneDay
        ) {
            return Err("unsupported Watchlist Mini-chart interval".to_owned());
        }
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        ensure_account(&mut connection, user_id)?;
        connection
            .execute(
                "UPDATE watchlist_settings
                 SET mini_chart_interval = ?2 WHERE user_id = ?1",
                params![user_id, interval.as_str()],
            )
            .map_err(|error| error.to_string())?;
        load_state(&connection, user_id)
    }
}

pub(crate) fn insert_default_watchlist(
    connection: &Connection,
    user_id: &str,
) -> Result<(), String> {
    validate_user_id(user_id)?;
    let default_venue_json =
        serde_json::to_string(&default_venue()).map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT INTO watchlist_settings(
                user_id, active_src, active_code, active_venue_json,
                mini_chart_interval, watchlist_defaults_version
             ) VALUES (?1, ?2, ?3, ?4, '1m', ?5)",
            params![
                user_id,
                DEFAULT_SRC,
                DEFAULT_ACTIVE_CODE,
                default_venue_json,
                WATCHLIST_DEFAULTS_VERSION,
            ],
        )
        .map_err(|error| error.to_string())?;
    seed_default_items(connection, user_id)?;
    Ok(())
}

fn ensure_account(connection: &mut Connection, user_id: &str) -> Result<(), String> {
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let default_venue_json =
        serde_json::to_string(&default_venue()).map_err(|error| error.to_string())?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO watchlist_settings(
                user_id, active_src, active_code, active_venue_json,
                mini_chart_interval, watchlist_defaults_version
             ) VALUES (?1, ?2, ?3, ?4, '1m', 0)",
            params![
                user_id,
                DEFAULT_SRC,
                DEFAULT_ACTIVE_CODE,
                default_venue_json
            ],
        )
        .map_err(|error| error.to_string())?;
    let defaults_version: i64 = transaction
        .query_row(
            "SELECT watchlist_defaults_version FROM watchlist_settings WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if inserted == 1 || defaults_version < WATCHLIST_DEFAULTS_VERSION {
        seed_default_items(&transaction, user_id)?;
        transaction
            .execute(
                "UPDATE watchlist_settings
                 SET watchlist_defaults_version = ?2 WHERE user_id = ?1",
                params![user_id, WATCHLIST_DEFAULTS_VERSION],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn seed_default_items(connection: &Connection, user_id: &str) -> Result<(), String> {
    let mut count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM watchlist_items WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut position: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(position) + 1, 0)
             FROM watchlist_items WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    for instrument in default_watchlist_items() {
        if count >= WATCHLIST_LIMIT {
            break;
        }
        let venue_json = serde_json::to_string(
            instrument
                .venue
                .as_ref()
                .expect("default instruments always have a Venue"),
        )
        .map_err(|error| error.to_string())?;
        let inserted = connection
            .execute(
                "INSERT OR IGNORE INTO watchlist_items(
                    user_id, src, code, venue_json, position
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    user_id,
                    instrument.src,
                    instrument.code,
                    venue_json,
                    position,
                ],
            )
            .map_err(|error| error.to_string())?;
        if inserted == 1 {
            count += 1;
            position += 1;
        }
    }
    Ok(())
}

fn load_state(connection: &Connection, user_id: &str) -> Result<WatchlistState, String> {
    let (active_src, active_code, active_venue_json, interval): (
        String,
        String,
        Option<String>,
        String,
    ) = connection
        .query_row(
            "SELECT active_src, active_code, active_venue_json, mini_chart_interval
             FROM watchlist_settings WHERE user_id = ?1",
            [user_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    let active_instrument = from_storage(active_src, active_code, active_venue_json)?;
    let mut statement = connection
        .prepare(
            "SELECT src, code, venue_json FROM watchlist_items
             WHERE user_id = ?1 ORDER BY position, rowid",
        )
        .map_err(|error| error.to_string())?;
    let items = statement
        .query_map([user_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let items = items
        .into_iter()
        .map(|(src, code, venue_json)| from_storage(src, code, venue_json))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WatchlistState {
        items,
        active_instrument,
        mini_chart_interval: parse_interval(&interval).unwrap_or(BarInterval::OneMinute),
        limit: WATCHLIST_LIMIT,
    })
}

fn first_item(connection: &Connection, user_id: &str) -> Result<Option<InstrumentRef>, String> {
    let row = connection
        .query_row(
            "SELECT src, code, venue_json FROM watchlist_items
             WHERE user_id = ?1 ORDER BY position, rowid LIMIT 1",
            [user_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    row.map(|(src, code, venue_json)| from_storage(src, code, venue_json))
        .transpose()
}

fn parse_interval(value: &str) -> Option<BarInterval> {
    BarInterval::ALL
        .into_iter()
        .find(|interval| interval.as_str() == value)
}

fn default_active() -> InstrumentRef {
    InstrumentRef {
        src: DEFAULT_SRC.to_owned(),
        code: DEFAULT_ACTIVE_CODE.to_owned(),
        venue: Some(default_venue()),
    }
}

fn default_watchlist_items() -> Vec<InstrumentRef> {
    let mut items = Vec::with_capacity(
        DEFAULT_CRYPTO_CODES.len() + DEFAULT_A_SHARE_CODES.len() + DEFAULT_US_CODES.len(),
    );
    items.extend(
        DEFAULT_CRYPTO_CODES
            .iter()
            .map(|code| default_instrument(DEFAULT_SRC, code, "okx", VenueKind::CryptoSpot)),
    );
    items.extend(DEFAULT_A_SHARE_CODES.iter().map(|(code, venue)| {
        default_instrument("akshare-rs", code, venue, VenueKind::ChinaAShareEquity)
    }));
    items.extend(
        DEFAULT_US_CODES
            .iter()
            .map(|code| default_instrument("alpaca", code, "nasdaq", VenueKind::UsEquity)),
    );
    items
}

fn default_instrument(src: &str, code: &str, venue_id: &str, kind: VenueKind) -> InstrumentRef {
    InstrumentRef {
        src: src.to_owned(),
        code: code.to_owned(),
        venue: Some(Venue::new(venue_id, kind).expect("default Venue is valid")),
    }
}

fn default_venue() -> Venue {
    Venue::new(DEFAULT_SRC, VenueKind::CryptoSpot).expect("default OKX Venue is valid")
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if columns.iter().any(|value| value == column) {
        return Ok(());
    }
    connection
        .execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn migrate_legacy_item_identity(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(watchlist_items)")
        .map_err(|error| error.to_string())?;
    let mut primary_key_columns = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    primary_key_columns.sort_by_key(|(_, position)| *position);
    let primary_key_columns = primary_key_columns
        .into_iter()
        .filter(|(_, position)| *position > 0)
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    if primary_key_columns != ["user_id", "src", "code"] {
        return Ok(());
    }

    connection
        .execute_batch(
            "
            CREATE TABLE watchlist_items_identity_migration (
                user_id TEXT NOT NULL,
                src TEXT NOT NULL,
                code TEXT NOT NULL,
                venue_json TEXT,
                position INTEGER NOT NULL,
                FOREIGN KEY (user_id) REFERENCES watchlist_settings(user_id)
                    ON DELETE CASCADE
            );
            INSERT INTO watchlist_items_identity_migration(
                user_id, src, code, venue_json, position
            )
            SELECT user_id, src, code, venue_json, position
            FROM watchlist_items;
            DROP TABLE watchlist_items;
            ALTER TABLE watchlist_items_identity_migration RENAME TO watchlist_items;
            ",
        )
        .map_err(|error| error.to_string())
}

fn canonicalize_instrument(instrument: &InstrumentRef) -> Result<InstrumentRef, String> {
    if !matches!(instrument.src.as_str(), "okx" | "akshare-rs" | "alpaca")
        || instrument.code.trim().is_empty()
    {
        return Err("unsupported or empty Instrument identity".to_owned());
    }
    let venue = instrument
        .venue
        .clone()
        .or_else(|| (instrument.src == DEFAULT_SRC).then(default_venue))
        .ok_or_else(|| "a canonical Venue is required for this Instrument".to_owned())?;
    let canonical = Venue::new(venue.id.clone(), venue.kind).map_err(|error| error.to_string())?;
    if canonical.time_zone != venue.time_zone {
        return Err("Instrument Venue time zone does not match its Venue kind".to_owned());
    }
    Ok(InstrumentRef {
        src: instrument.src.clone(),
        code: instrument.code.clone(),
        venue: Some(canonical),
    })
}

fn from_storage(
    src: String,
    code: String,
    venue_json: Option<String>,
) -> Result<InstrumentRef, String> {
    let venue = venue_json
        .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .transpose()?;
    canonicalize_instrument(&InstrumentRef { src, code, venue })
}

fn validate_user_id(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() || user_id.len() > 128 {
        Err("user ID must contain 1 to 128 characters".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use adaq_data_core::market::{Venue, VenueKind};
    use rusqlite::Connection;

    use super::{InstrumentRef, WATCHLIST_LIMIT, WatchlistDb, validate_provider_venue};

    fn database() -> WatchlistDb {
        WatchlistDb::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn instrument(code: &str) -> InstrumentRef {
        InstrumentRef {
            src: "okx".to_owned(),
            code: code.to_owned(),
            venue: None,
        }
    }

    fn instrument_at(src: &str, venue_id: &str, kind: VenueKind, code: &str) -> InstrumentRef {
        InstrumentRef {
            src: src.to_owned(),
            code: code.to_owned(),
            venue: Some(Venue::new(venue_id, kind).unwrap()),
        }
    }

    #[test]
    fn initializes_defaults_once_and_allows_an_empty_watchlist() {
        let database = database();
        let initial = database.get("user-1").unwrap();
        assert_eq!(initial.items.len(), 16);
        assert_eq!(initial.limit, WATCHLIST_LIMIT);
        assert_eq!(
            initial
                .items
                .iter()
                .map(|item| item.code.as_str())
                .collect::<Vec<_>>(),
            vec![
                "BTC-USDT", "ETH-USDT", "SOL-USDT", "600519", "601318", "510500", "000333",
                "588000", "688981", "NVDA", "TSLA", "AAPL", "GOOGL", "MSFT", "AMZN", "META",
            ]
        );
        assert_eq!(initial.items[5].venue.as_ref().unwrap().id, "sse");
        assert_eq!(initial.items[6].venue.as_ref().unwrap().id, "szse");

        for item in initial.items {
            database.remove("user-1", &item).unwrap();
        }

        let state = database.get("user-1").unwrap();
        assert!(state.items.is_empty());
        assert_eq!(state.active_instrument.code, "BTC-USDT");
    }

    #[test]
    fn removing_the_active_instrument_selects_the_first_remaining_item() {
        let database = database();
        database
            .set_active("user-1", &instrument("ETH-USDT"))
            .unwrap();

        let state = database.remove("user-1", &instrument("ETH-USDT")).unwrap();

        assert_eq!(state.active_instrument.code, "BTC-USDT");
    }

    #[test]
    fn keeps_identity_provider_independent_but_venue_scoped() {
        let database = database();
        let sse = instrument_at("akshare-rs", "sse", VenueKind::ChinaAShareEquity, "600000");
        database.add("user-1", &sse).unwrap();

        let same_instrument_from_another_provider =
            instrument_at("alpaca", "sse", VenueKind::ChinaAShareEquity, "600000");
        assert!(
            database
                .add("user-1", &same_instrument_from_another_provider)
                .unwrap_err()
                .contains("already")
        );

        let same_code_at_another_venue =
            instrument_at("akshare-rs", "szse", VenueKind::ChinaAShareEquity, "600000");
        assert_eq!(
            database
                .add("user-1", &same_code_at_another_venue)
                .unwrap()
                .items
                .len(),
            18
        );
    }

    #[test]
    fn migrates_the_legacy_provider_key_before_loading_items() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                CREATE TABLE watchlist_settings (
                    user_id TEXT PRIMARY KEY,
                    active_src TEXT NOT NULL,
                    active_code TEXT NOT NULL,
                    mini_chart_interval TEXT NOT NULL
                );
                CREATE TABLE watchlist_items (
                    user_id TEXT NOT NULL,
                    src TEXT NOT NULL,
                    code TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    PRIMARY KEY (user_id, src, code),
                    FOREIGN KEY (user_id) REFERENCES watchlist_settings(user_id)
                        ON DELETE CASCADE
                );
                INSERT INTO watchlist_settings(user_id, active_src, active_code, mini_chart_interval)
                VALUES ('user-1', 'okx', 'BTC-USDT', '1m');
                INSERT INTO watchlist_items(user_id, src, code, position)
                VALUES ('user-1', 'okx', 'BTC-USDT', 0);
                ",
            )
            .unwrap();

        let database = WatchlistDb::from_connection(connection).unwrap();
        let state = database.get("user-1").unwrap();
        assert_eq!(state.items[0].venue.as_ref().unwrap().id, "okx");
        assert_eq!(state.items.len(), 16);

        let seeded = state
            .items
            .iter()
            .find(|item| item.code == "600519")
            .cloned()
            .unwrap();
        database.remove("user-1", &seeded).unwrap();
        assert!(
            database
                .get("user-1")
                .unwrap()
                .items
                .iter()
                .all(|item| item.code != "600519")
        );

        let same_code_at_another_venue = instrument_at(
            "akshare-rs",
            "sse",
            VenueKind::ChinaAShareEquity,
            "BTC-USDT",
        );
        assert_eq!(
            database
                .add("user-1", &same_code_at_another_venue)
                .unwrap()
                .items
                .len(),
            16
        );
    }

    #[test]
    fn keeps_watchlist_items_isolated_by_user() {
        let database = database();
        let instrument = instrument_at("akshare-rs", "sse", VenueKind::ChinaAShareEquity, "600000");

        database.add("user-1", &instrument).unwrap();

        assert!(
            database
                .get("user-2")
                .unwrap()
                .items
                .iter()
                .all(|value| value.code != "600000")
        );
    }

    #[test]
    fn rejects_provider_and_venue_mismatches_at_the_command_boundary() {
        let instrument = instrument_at("alpaca", "sse", VenueKind::ChinaAShareEquity, "600000");

        assert!(
            validate_provider_venue(&instrument)
                .unwrap_err()
                .contains("do not match")
        );
    }
}
