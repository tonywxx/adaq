use ada_data_core::{
    BarInterval, BarSeries, DataError, HistoricalBarRange, OkxClient, SpotInstrument,
};
use std::sync::Mutex;
use tauri::{
    Emitter, Manager, State,
    menu::{AboutMetadata, MenuBuilder, SubmenuBuilder},
};
use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const FACTOR_COMPONENT_PATH: &str = "/Users/tony/Downloads/factor-ema5-10.adaq";

wasmtime::component::bindgen!({
    path: "/Users/tony/private/adaq-factor-ema5-10/wit",
    world: "ema-5-10",
});

const CHECK_FOR_UPDATES_MENU_ID: &str = "check_for_updates";
const CHECK_FOR_UPDATES_EVENT: &str = "adaq-check-for-updates";

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FactorMetaInfo {
    uuid: String,
    name: String,
    category: String,
    component_type: String,
    description: String,
    version: String,
    price: String,
    currency: String,
    usage: String,
    latest_update_date: String,
    recommended_timeframe: String,
    minimum_closes: u32,
}

struct LoadedFactor {
    store: Store<WasiState>,
    bindings: Ema510,
}

struct WasiState {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl WasiView for WasiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

struct WasmLoader {
    factor: Mutex<LoadedFactor>,
}

impl WasmLoader {
    fn load(path: &str) -> Result<Self, String> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config).map_err(|error| error.to_string())?;
        let component = Component::from_file(&engine, path).map_err(|error| error.to_string())?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| error.to_string())?;
        let mut store = Store::new(
            &engine,
            WasiState {
                ctx: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
            },
        );
        let bindings = Ema510::instantiate(&mut store, &component, &linker)
            .map_err(|error| error.to_string())?;

        Ok(Self {
            factor: Mutex::new(LoadedFactor { store, bindings }),
        })
    }

    fn get_meta_info(&self) -> Result<FactorMetaInfo, String> {
        let mut factor = self.factor.lock().map_err(|error| error.to_string())?;
        let LoadedFactor { store, bindings } = &mut *factor;
        let meta = bindings
            .adaq_factor_api()
            .call_get_meta_info(store)
            .map_err(|error| error.to_string())?;

        Ok(FactorMetaInfo {
            uuid: meta.uuid,
            name: meta.name,
            category: match meta.category {
                exports::adaq::factor::api::ComponentCategory::Factor => "factor",
                exports::adaq::factor::api::ComponentCategory::Strategy => "strategy",
            }
            .to_owned(),
            component_type: meta.component_type,
            description: meta.description,
            version: meta.version,
            price: meta.price,
            currency: meta.currency,
            usage: meta.usage,
            latest_update_date: meta.latest_update_date,
            recommended_timeframe: meta.recommended_timeframe,
            minimum_closes: meta.minimum_closes,
        })
    }
}

#[tauri::command]
fn get_factor_meta_info(loader: State<'_, WasmLoader>) -> Result<FactorMetaInfo, String> {
    loader.get_meta_info()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketSourceRequest {
    src: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketGetBarSeriesRequest {
    src: String,
    code: String,
    interval: BarInterval,
    start_time_ms: i64,
    end_time_ms: i64,
}

fn require_okx(src: &str) -> Result<(), DataError> {
    if src == "okx" {
        Ok(())
    } else {
        Err(DataError::new(
            src,
            "unsupported_src",
            format!("unsupported market data source: {src}"),
        ))
    }
}

#[tauri::command]
async fn market_list_spot_instruments(
    request: MarketSourceRequest,
    client: State<'_, OkxClient>,
) -> Result<Vec<SpotInstrument>, DataError> {
    require_okx(&request.src)?;
    client.list_spot_instruments().await
}

#[tauri::command]
async fn market_get_bar_series(
    request: MarketGetBarSeriesRequest,
    client: State<'_, OkxClient>,
) -> Result<BarSeries, DataError> {
    require_okx(&request.src)?;
    client
        .get_bar_series_range(
            &request.code,
            request.interval,
            HistoricalBarRange {
                start_time_ms: request.start_time_ms,
                end_time_ms: request.end_time_ms,
            },
        )
        .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_http::init())
        .setup(|app| {
            app.manage(WasmLoader::load(FACTOR_COMPONENT_PATH).map_err(std::io::Error::other)?);
            app.manage(OkxClient::default());
            let handle = app.handle();
            let app_menu = SubmenuBuilder::new(handle, "adaq")
                .about(Some(AboutMetadata {
                    name: Some("adaq".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    authors: Some(vec!["TONy.W".into()]),
                    comments: Some("AI Quant Trading".into()),
                    ..Default::default()
                }))
                .text(CHECK_FOR_UPDATES_MENU_ID, "Check for Updates...")
                .separator()
                .services()
                .separator()
                .hide()
                .hide_others()
                .show_all()
                .separator()
                .quit()
                .build()?;
            let edit_menu = SubmenuBuilder::new(handle, "Edit")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;
            let window_menu = SubmenuBuilder::new(handle, "Window")
                .minimize()
                .fullscreen()
                .separator()
                .close_window()
                .build()?;
            let menu = MenuBuilder::new(handle)
                .item(&app_menu)
                .item(&edit_menu)
                .item(&window_menu)
                .build()?;

            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                if event.id() == CHECK_FOR_UPDATES_MENU_ID {
                    if let Err(error) = app.emit_to("main", CHECK_FOR_UPDATES_EVENT, ()) {
                        eprintln!("failed to emit update check event: {error}");
                    }
                }
            });

            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_factor_meta_info,
            market_list_spot_instruments,
            market_get_bar_series
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{FACTOR_COMPONENT_PATH, WasmLoader};

    #[test]
    fn loads_and_reuses_factor_component() {
        let loader = WasmLoader::load(FACTOR_COMPONENT_PATH).unwrap();
        let first = loader.get_meta_info().unwrap();
        let second = loader.get_meta_info().unwrap();

        assert_eq!(first.name, "EMA 5/10 Crossover");
        assert_eq!(first.uuid, second.uuid);
    }
}
