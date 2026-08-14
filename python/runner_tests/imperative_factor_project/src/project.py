from adaq import FactorOutput, FactorOutputBatch, Unavailable


class ImperativeFactor:
    kind = "factor"

    def evaluate(self, _context, batches):
        for batch in batches:
            rows = []
            for row in batch["rows"]:
                close = row["inputs"]["close"]
                value = Unavailable("missing-input") if close is None else close
                rows.append(FactorOutput(row["instrumentId"], row["eventTimeMs"], value))
            yield FactorOutputBatch(tuple(rows), batch["segmentId"])


def create_project():
    return ImperativeFactor()
