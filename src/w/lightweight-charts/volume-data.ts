export const MAX_CHART_VALUE = Number.MAX_SAFE_INTEGER / 100;

export function volumeScale(data: readonly { volume?: number }[]) {
	const maxVolume = data.reduce(
		(max, { volume = 0 }) =>
			Number.isFinite(volume) ? Math.max(max, Math.abs(volume)) : max,
		0,
	);
	return Math.max(1, Math.ceil(maxVolume / MAX_CHART_VALUE));
}

export function scaleVolume(volume: number | undefined, scale: number) {
	return typeof volume === "number" && Number.isFinite(volume)
		? volume / Math.max(1, scale)
		: 0;
}
