import { MAX_CHART_VALUE, scaleVolume, volumeScale } from "./volume-data";

test("scales a PEPE weekly volume below Lightweight Charts' value limit", () => {
	const value = 727_886_377_162_213;
	const scale = volumeScale([{ volume: value }]);

	expect(scale).toBe(9);
	expect(scaleVolume(value, scale)).toBeLessThanOrEqual(MAX_CHART_VALUE);
});
