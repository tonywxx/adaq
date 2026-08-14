from adaq import ModelArtifact, ModelContext, ModelProject, Signal, Target


class QlibRidgeReturn:
    kind = "model"
    target = Target(
        id="future-close-return",
        kind="continuous-future-close-return",
        horizon_bars=5,
        value_scale="return",
    )
    signal = Signal(id="forecast", kind="forecast", value_scale="native")

    def fit(self, _context: ModelContext) -> ModelArtifact:
        raise RuntimeError("The registered Host-fed Qlib Ridge Adapter owns fitting")

    def predict(self, _context: ModelContext, _fitted_model: ModelArtifact):
        raise RuntimeError("The registered Host-fed Qlib Ridge Adapter owns prediction")


def create_project() -> ModelProject:
    return QlibRidgeReturn()
