use std::{path::Path, sync::Mutex};

use adaq_data_core::BarInterval;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const DEFAULT_SRC: &str = "okx";
const DEFAULT_ACTIVE_CODE: &str = "BTC-USDT";
const DEFAULT_CODES: [&str; 3] = ["BTC-USDT", "ETH-USDT", "SOL-USDT"];
const WATCHLIST_LIMIT: i64 = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentRef {
    pub src: String,
    pub code: String,
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
                    mini_chart_interval TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS watchlist_items (
                    user_id TEXT NOT NULL,
                    src TEXT NOT NULL,
                    code TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    PRIMARY KEY (user_id, src, code),
                    FOREIGN KEY (user_id) REFERENCES watchlist_settings(user_id)
                        ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS watchlist_items_order
                    ON watchlist_items(user_id, position);
                ",
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
        validate_instrument(instrument)?;
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
                "INSERT INTO watchlist_items(user_id, src, code, position)
                 VALUES (?1, ?2, ?3, ?4)",
                params![user_id, instrument.src, instrument.code, position],
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
        validate_instrument(instrument)?;
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        ensure_account(&mut connection, user_id)?;
        connection
            .execute(
                "DELETE FROM watchlist_items
                 WHERE user_id = ?1 AND src = ?2 AND code = ?3",
                params![user_id, instrument.src, instrument.code],
            )
            .map_err(|error| error.to_string())?;
        let (active_src, active_code): (String, String) = connection
            .query_row(
                "SELECT active_src, active_code FROM watchlist_settings WHERE user_id = ?1",
                [user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        if active_src == instrument.src && active_code == instrument.code {
            let next = first_item(&connection, user_id)?.unwrap_or_else(default_active);
            connection
                .execute(
                    "UPDATE watchlist_settings
                     SET active_src = ?2, active_code = ?3 WHERE user_id = ?1",
                    params![user_id, next.src, next.code],
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
        validate_instrument(instrument)?;
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        ensure_account(&mut connection, user_id)?;
        connection
            .execute(
                "UPDATE watchlist_settings
                 SET active_src = ?2, active_code = ?3 WHERE user_id = ?1",
                params![user_id, instrument.src, instrument.code],
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

fn ensure_account(connection: &mut Connection, user_id: &str) -> Result<(), String> {
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO watchlist_settings(
                user_id, active_src, active_code, mini_chart_interval
             ) VALUES (?1, ?2, ?3, '1m')",
            params![user_id, DEFAULT_SRC, DEFAULT_ACTIVE_CODE],
        )
        .map_err(|error| error.to_string())?;
    if inserted == 1 {
        for (position, code) in DEFAULT_CODES.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO watchlist_items(user_id, src, code, position)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![user_id, DEFAULT_SRC, code, position as i64],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn load_state(connection: &Connection, user_id: &str) -> Result<WatchlistState, String> {
    let (active_src, active_code, interval): (String, String, String) = connection
        .query_row(
            "SELECT active_src, active_code, mini_chart_interval
             FROM watchlist_settings WHERE user_id = ?1",
            [user_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT src, code FROM watchlist_items
             WHERE user_id = ?1 ORDER BY position, rowid",
        )
        .map_err(|error| error.to_string())?;
    let items = statement
        .query_map([user_id], |row| {
            Ok(InstrumentRef {
                src: row.get(0)?,
                code: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(WatchlistState {
        items,
        active_instrument: InstrumentRef {
            src: active_src,
            code: active_code,
        },
        mini_chart_interval: parse_interval(&interval).unwrap_or(BarInterval::OneMinute),
        limit: WATCHLIST_LIMIT,
    })
}

fn first_item(connection: &Connection, user_id: &str) -> Result<Option<InstrumentRef>, String> {
    connection
        .query_row(
            "SELECT src, code FROM watchlist_items
             WHERE user_id = ?1 ORDER BY position, rowid LIMIT 1",
            [user_id],
            |row| {
                Ok(InstrumentRef {
                    src: row.get(0)?,
                    code: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
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
    }
}

fn validate_user_id(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() || user_id.len() > 128 {
        Err("user ID must contain 1 to 128 characters".to_owned())
    } else {
        Ok(())
    }
}

fn validate_instrument(instrument: &InstrumentRef) -> Result<(), String> {
    if instrument.src != DEFAULT_SRC || instrument.code.trim().is_empty() {
        Err("only non-empty OKX Instrument IDs are supported".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{InstrumentRef, WATCHLIST_LIMIT, WatchlistDb};

    fn database() -> WatchlistDb {
        WatchlistDb::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn instrument(code: &str) -> InstrumentRef {
        InstrumentRef {
            src: "okx".to_owned(),
            code: code.to_owned(),
        }
    }

    #[test]
    fn initializes_defaults_once_and_allows_an_empty_watchlist() {
        let database = database();
        let initial = database.get("user-1").unwrap();
        assert_eq!(initial.items.len(), 3);
        assert_eq!(initial.limit, WATCHLIST_LIMIT);

        for code in ["BTC-USDT", "ETH-USDT", "SOL-USDT"] {
            database.remove("user-1", &instrument(code)).unwrap();
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
}
