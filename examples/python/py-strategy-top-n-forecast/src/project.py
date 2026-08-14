from adaq import StrategyContext, StrategyProject


class TopNForecast:
    kind = "strategy"

    def build_target(self, _context: StrategyContext):
        raise RuntimeError("Strategy execution is an M13 continuation")


def create_project() -> StrategyProject:
    return TopNForecast()
