from adaq import FactorDefinition, FactorProject, Scope, create_factor_definition


class CrossSectionalMomentum:
    kind = "factor"

    def define(self, _context) -> FactorDefinition:
        return create_factor_definition(
            Scope.CROSS_SECTIONAL,
            [
                {"op": "market-close", "id": "close"},
                {"op": "backward-simple-return", "input": "close", "parameter": "lookback"},
                {"op": "cross-sectional-percentile", "input": "return"},
                {"op": "rename", "input": "percentile", "output": "momentum-score"},
            ],
            ["momentum-score"],
        )


def create_project() -> FactorProject:
    return CrossSectionalMomentum()
