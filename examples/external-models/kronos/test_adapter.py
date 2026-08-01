import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

import adapter


FIXTURES = pathlib.Path(__file__).with_name("fixtures")


class AdapterTest(unittest.TestCase):
    def test_generated_paths_become_aligned_expected_close_returns(self):
        snapshot = json.loads((FIXTURES / "snapshot.json").read_text())
        paths = json.loads((FIXTURES / "generated-paths.json").read_text())

        rows = adapter.transform_paths(snapshot, paths, horizons=[1, 2], lookback=2)

        self.assertEqual(rows[0]["status"], "unavailable")
        self.assertEqual(rows[0]["unavailableReason"], "warmup")
        self.assertEqual(rows[1]["predictionTimeMs"], 7_200_000)
        self.assertEqual(rows[1]["availableAtMs"], 7_200_000)
        self.assertEqual(rows[1]["values"], [0.01, 0.02])
        self.assertEqual(rows[2]["values"], [-0.01, 0.05])
        self.assertEqual(rows[3]["values"], [0.0, 0.02])
        self.assertTrue(all(row["instrumentId"] == "okx:BTC-USDT" for row in rows))

    @unittest.skipUnless(importlib.util.find_spec("pyarrow"), "requires pinned Adapter environment")
    def test_fixture_archive_matches_the_imported_golden_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "fixture.adaq-signals"
            subprocess.run(
                [
                    sys.executable,
                    str(pathlib.Path(adapter.__file__)),
                    "--snapshot-json",
                    str(FIXTURES / "snapshot.json"),
                    "--fixture-paths",
                    str(FIXTURES / "generated-paths.json"),
                    "--lookback",
                    "2",
                    "--horizons",
                    "1,2",
                    "--seed",
                    "7",
                    "--output",
                    str(output),
                ],
                check=True,
            )
            self.assertEqual(
                output.read_bytes(),
                (FIXTURES / "kronos-fixture.adaq-signals").read_bytes(),
            )


if __name__ == "__main__":
    unittest.main()
