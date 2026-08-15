from adaq import (
    Forecast,
    ModelArtifact,
    ModelContext,
    ModelProject,
    Signal,
    Target,
    Unavailable,
    finite,
)
from adaq_qlib_adapter import FitResult, fit as fit_ridge, predict as predict_ridge


class QlibRidgeReturn:
    kind = "model"
    target = Target(
        id="future-close-return",
        kind="continuous-future-close-return",
        horizon_bars=5,
        value_scale="return",
    )
    signal = Signal(id="forecast", kind="forecast", value_scale="native")

    def fit(self, context: ModelContext) -> ModelArtifact:
        dataset = context.inputs.get("dataset")
        transformation = context.inputs.get("transformation")
        if dataset is None or not isinstance(transformation, dict):
            raise ValueError("Host-fed DatasetH and transformation are required")
        alpha = next(
            float(parameter.value)
            for parameter in context.parameters
            if parameter.id == "alpha"
        )
        result = fit_ridge(dataset, transformation, alpha)
        return ModelArtifact(
            schema="adaq:linear-model:candidate@1",
            payload={
                "alpha": result.alpha,
                "adapter_id": result.adapter_id,
                "artifact_schema": result.artifact_schema,
                "numeric_representation": result.numeric_representation,
                "forecast_contract": result.forecast_contract,
                "input_slots": result.input_slots,
                "coefficients": result.coefficients,
                "intercept": result.intercept,
                "transformation_sha256": result.transformation_sha256,
            },
        )

    def predict(self, context: ModelContext, fitted_model: ModelArtifact):
        dataset = context.inputs.get("dataset")
        transformation = context.inputs.get("transformation")
        if dataset is None or not isinstance(transformation, dict):
            raise ValueError("Host-fed DatasetH and transformation are required")
        payload = fitted_model.payload
        fitted = FitResult(
            alpha=float(payload["alpha"]),
            coefficients=tuple(float(value) for value in payload["coefficients"]),
            intercept=float(payload["intercept"]),
            input_slots=tuple(str(value) for value in payload["input_slots"]),
            transformation_sha256=str(payload["transformation_sha256"]),
        )
        target_window_end = context.inputs.get("targetWindowEnd")
        return [
            Forecast(
                instrument_id=str(row["instrument"]),
                prediction_time_ms=int(row["datetime"]),
                value=(
                    Unavailable(reason="target-window-boundary")
                    if isinstance(target_window_end, int)
                    and int(row["datetime"]) + 5 > target_window_end
                    else finite(float(row["value"]))
                ),
            )
            for row in predict_ridge(dataset, fitted, transformation)
        ]


def create_project() -> ModelProject:
    return QlibRidgeReturn()
