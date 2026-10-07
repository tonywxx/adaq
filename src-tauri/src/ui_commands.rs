use std::sync::Arc;

use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::Value;
use tauri::{Runtime, ipc::Invoke};

// Dispatch, argument decoding, synchronous commands and response serialization must
// all leave the window thread. Async commands keep their own background scheduling.
pub(crate) fn background_handler<R: Runtime>(
    handler: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    let handler = Arc::new(handler);
    move |invoke| {
        // Revocation is a bounded in-memory update and must precede later IPC,
        // including a new session bind after signing out.
        if invoke.message.command() == "auth_clear_session" {
            return handler(invoke);
        }
        let handler = Arc::clone(&handler);
        let resolver = invoke.resolver.clone();
        let command = invoke.message.command().to_owned();
        tauri::async_runtime::spawn(async move {
            match tauri::async_runtime::spawn_blocking(move || handler(invoke)).await {
                Ok(true) => {}
                Ok(false) => resolver.reject(format!("Command {command} not found")),
                Err(error) => resolver.reject(format!("Command {command} failed: {error}")),
            }
        });
        true
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordPage {
    pub items: Vec<Value>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
}

impl RecordPage {
    // SQL fragments come exclusively from Host-owned queries, never the Webview.
    pub(crate) fn read(
        database: &Connection,
        source: &str,
        projection: &str,
        order: &str,
        user_id: &str,
        entity_id: Option<&str>,
        requested_page: usize,
    ) -> Result<Self, String> {
        if user_id.trim().is_empty() || requested_page == 0 {
            return Err("A user and a page starting at 1 are required.".into());
        }
        let total = database
            .query_row(
                &format!("SELECT COUNT(*) FROM {source}"),
                params![user_id, entity_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?;
        let total = usize::try_from(total).map_err(|error| error.to_string())?;
        let page_size = 10;
        let page = requested_page.min(total.div_ceil(page_size).max(1));
        let mut statement = database
            .prepare(&format!(
                "SELECT {projection} FROM {source} ORDER BY {order} LIMIT ?3 OFFSET ?4"
            ))
            .map_err(|error| error.to_string())?;
        let items = statement
            .query_map(
                params![
                    user_id,
                    entity_id,
                    page_size as i64,
                    ((page - 1) * page_size) as i64
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())?
            .map(|row| {
                let value = row.map_err(|error| error.to_string())?;
                serde_json::from_str(&value).map_err(|error| error.to_string())
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            items,
            page,
            page_size,
            total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;
    use tauri::{State, WebviewWindow, test::MockRuntime};

    struct BlockingCommand {
        started: mpsc::Sender<std::thread::ThreadId>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    #[tauri::command]
    fn slow_read(state: State<'_, BlockingCommand>) -> &'static str {
        state.started.send(std::thread::current().id()).unwrap();
        state
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        "loaded"
    }

    #[tauri::command]
    fn quick_read() -> &'static str {
        "responsive"
    }

    fn request(
        command: &str,
        window: &WebviewWindow<MockRuntime>,
    ) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: window.url().unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_owned(),
        }
    }

    #[test]
    fn synchronous_ipc_leaves_dispatch_thread_and_allows_other_commands() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let app = tauri::test::mock_builder()
            .manage(BlockingCommand {
                started: started_tx,
                release: Mutex::new(release_rx),
            })
            .invoke_handler(background_handler(tauri::generate_handler![
                slow_read, quick_read
            ]))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let slow_window = window.clone();
        let client = std::thread::spawn(move || {
            tauri::test::get_ipc_response(&slow_window, request("slow_read", &slow_window))
        });
        let worker = started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_ne!(worker, client.thread().id());
        assert!(!client.is_finished());
        let quick = tauri::test::get_ipc_response(&window, request("quick_read", &window)).unwrap();
        assert_eq!(quick.deserialize::<String>().unwrap(), "responsive");
        assert!(
            tauri::test::get_ipc_response(&window, request("unknown_command", &window)).is_err()
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            client
                .join()
                .unwrap()
                .unwrap()
                .deserialize::<String>()
                .unwrap(),
            "loaded"
        );
    }

    #[test]
    fn bot_pages_skip_old_attempts_and_bound_current_audit_for_each_user() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = crate::bot_operations::BotStore::open(Arc::clone(&database)).unwrap();
        {
            let conn = database.lock().unwrap();
            for user in ["alice", "bob"] {
                for index in 0..27 {
                    let bot = format!("{user}-{index:03}");
                    let bundle = serde_json::json!({"identity": bot, "accountId": "demo", "schedule": {"type": "ema-double-cross", "instrumentId": "okx:BTC-USDT"}});
                    conn.execute("INSERT INTO bots VALUES (?1, ?2, ?3, '\"faulted\"', 'current', '[]', 1, ?4)", params![bot, user, bundle.to_string(), index]).unwrap();
                    // An old attempt must never be decoded to render the current card.
                    conn.execute("INSERT INTO bot_runtime_attempts VALUES (?1, ?2, 'old', 0, 'invalid legacy payload', 1)", params![user, bot]).unwrap();
                    let attempt = serde_json::json!({"attemptId": "current", "evidence": (0..27).map(|n| serde_json::json!({"code": n})).collect::<Vec<_>>(), "orders": vec!["large"; 1_000], "events": [], "decisions": []});
                    conn.execute(
                        "INSERT INTO bot_runtime_attempts VALUES (?1, ?2, 'current', 1, ?3, 1)",
                        params![user, bot, attempt.to_string()],
                    )
                    .unwrap();
                }
            }
        }
        let mut ids = std::collections::HashSet::new();
        for page in 1..=3 {
            let result = store.page("alice", page).unwrap();
            assert_eq!(result.total, 27);
            assert!(result.items.len() <= 10);
            for item in result.items {
                assert!(item["botId"].as_str().unwrap().starts_with("alice-"));
                assert!(ids.insert(item["botId"].clone().to_string()));
                assert_eq!(item["attempts"].as_array().unwrap().len(), 1);
                assert_eq!(item["attempts"][0]["evidence"].as_array().unwrap().len(), 3);
                assert!(item["attempts"][0]["orders"].as_array().unwrap().is_empty());
            }
        }
        assert_eq!(ids.len(), 27);
        assert_eq!(store.page("alice", usize::MAX).unwrap().page, 3);
        assert!(store.page("alice", 0).is_err());
        let audit = store
            .audit_page(
                "alice",
                "alice-026",
                "current",
                crate::bot_operations::BotAuditSection::Evidence,
                2,
            )
            .unwrap();
        assert_eq!(audit.items.len(), 10);
        assert_eq!(audit.items[0]["code"], 16);
        assert!(
            store
                .audit_page(
                    "bob",
                    "alice-026",
                    "current",
                    crate::bot_operations::BotAuditSection::Evidence,
                    1
                )
                .is_err()
        );
        assert!(
            store
                .audit_page(
                    "alice",
                    "alice-026",
                    "old",
                    crate::bot_operations::BotAuditSection::Evidence,
                    1
                )
                .is_err()
        );
    }

    #[test]
    fn feedback_pages_remove_snapshot_payload_and_keep_off_page_lens_availability() {
        use crate::paper_feedback::{FeedbackSection, PaperFeedbackStore};
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperFeedbackStore::open(Arc::clone(&database)).unwrap();
        {
            let conn = database.lock().unwrap();
            for user in ["alice", "bob"] {
                for index in 0..27 {
                    let id = format!("{user}-{index:03}");
                    let payload = serde_json::json!({"botId": "bot", "evidence": vec!["large immutable evidence"; 1_000]});
                    conn.execute(
                        "INSERT INTO paper_feedback_snapshots VALUES (?1, ?2, ?3, 'ready', ?4)",
                        params![id, user, payload.to_string(), index],
                    )
                    .unwrap();
                    let report =
                        serde_json::json!({"snapshotId": id, "lens": "factor", "metrics": {}});
                    conn.execute(
                        "INSERT INTO paper_feedback_reports VALUES (?1, ?2, ?1, ?3, 'ready', ?4)",
                        params![id, user, report.to_string(), 27 - index],
                    )
                    .unwrap();
                    conn.execute(
                        "INSERT INTO research_review_decisions VALUES (?1, ?2, '{}', ?3)",
                        params![id, user, index],
                    )
                    .unwrap();
                }
            }
        }
        for section in [
            FeedbackSection::Snapshots,
            FeedbackSection::Reports,
            FeedbackSection::Decisions,
        ] {
            let page = store.page("alice", section, 1).unwrap();
            assert_eq!(page.total, 27);
            assert_eq!(page.items.len(), 10);
            assert_eq!(page.page_size, 10);
        }
        let snapshots = store.page("alice", FeedbackSection::Snapshots, 1).unwrap();
        let reports = store.page("alice", FeedbackSection::Reports, 1).unwrap();
        assert!(
            reports
                .items
                .iter()
                .all(|report| report["evidenceState"] == "ready")
        );
        for snapshot in snapshots.items {
            assert!(
                snapshot["snapshotId"]
                    .as_str()
                    .unwrap()
                    .starts_with("alice-")
            );
            assert!(snapshot["input"].get("evidence").is_none());
            assert_eq!(snapshot["evidenceState"], "ready");
            assert_eq!(snapshot["existingLenses"], serde_json::json!(["factor"]));
            assert!(
                !reports
                    .items
                    .iter()
                    .any(|report| report["input"]["snapshotId"] == snapshot["snapshotId"])
            );
        }
        assert_eq!(
            store
                .page("alice", FeedbackSection::Snapshots, 3)
                .unwrap()
                .items
                .len(),
            7
        );
        assert!(store.page("alice", FeedbackSection::Snapshots, 0).is_err());
        assert_eq!(
            store
                .page("unknown", FeedbackSection::Reports, 9)
                .unwrap()
                .total,
            0
        );
    }
}
