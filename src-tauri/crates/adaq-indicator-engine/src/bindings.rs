// Generated from TA-Lib v0.7.1 include/ta_common.h, ta_defs.h, and ta_func.h.
// This narrow tracer binding is committed so normal builds need neither bindgen nor libclang.

pub(super) const TA_SUCCESS: i32 = 0;
pub(super) const TA_FUNC_UNST_ALL: i32 = 24;
pub(super) const TA_COMPATIBILITY_DEFAULT: i32 = 0;
pub(super) const TA_ALL_CANDLE_SETTINGS: i32 = 11;

pub(super) type ParamHolder = std::ffi::c_void;

unsafe extern "C" {
    pub(super) fn TA_Initialize() -> i32;
    pub(super) fn TA_SetUnstablePeriod(id: i32, unstable_period: u32) -> i32;
    pub(super) fn TA_SetCompatibility(value: i32) -> i32;
    pub(super) fn TA_RestoreCandleDefaultSettings(setting_type: i32) -> i32;
    pub(super) fn TA_RSI_Lookback(time_period: i32) -> i32;
    pub(super) fn TA_RSI(
        start_index: i32,
        end_index: i32,
        input: *const f64,
        time_period: i32,
        output_begin: *mut i32,
        output_count: *mut i32,
        output: *mut f64,
    ) -> i32;
    pub(super) fn TA_GetFuncHandle(name: *const std::ffi::c_char, handle: *mut *const u32) -> i32;
    pub(super) fn TA_ParamHolderAlloc(handle: *const u32, holder: *mut *mut ParamHolder) -> i32;
    pub(super) fn TA_ParamHolderFree(holder: *mut ParamHolder) -> i32;
    pub(super) fn TA_SetInputParamRealPtr(
        holder: *mut ParamHolder,
        index: u32,
        value: *const f64,
    ) -> i32;
    pub(super) fn TA_SetInputParamPricePtr(
        holder: *mut ParamHolder,
        index: u32,
        open: *const f64,
        high: *const f64,
        low: *const f64,
        close: *const f64,
        volume: *const f64,
        open_interest: *const f64,
    ) -> i32;
    pub(super) fn TA_SetOptInputParamInteger(
        holder: *mut ParamHolder,
        index: u32,
        value: i32,
    ) -> i32;
    pub(super) fn TA_SetOptInputParamReal(holder: *mut ParamHolder, index: u32, value: f64) -> i32;
    pub(super) fn TA_SetOutputParamIntegerPtr(
        holder: *mut ParamHolder,
        index: u32,
        value: *mut i32,
    ) -> i32;
    pub(super) fn TA_SetOutputParamRealPtr(
        holder: *mut ParamHolder,
        index: u32,
        value: *mut f64,
    ) -> i32;
    pub(super) fn TA_GetLookback(holder: *const ParamHolder, lookback: *mut i32) -> i32;
    pub(super) fn TA_CallFunc(
        holder: *const ParamHolder,
        start: i32,
        end: i32,
        begin: *mut i32,
        count: *mut i32,
    ) -> i32;
}
