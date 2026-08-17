import { useCallback, useEffect, useRef, useState } from "react";
import {
	formatFactorError,
	readFactorCache,
	writeFactorCache,
} from "./factor-data";
import type { FactorPage } from "./factor-types";

export const afterPaint = (): Promise<void> =>
	new Promise((resolve) => {
		if (typeof requestAnimationFrame === "undefined") {
			resolve();
			return;
		}
		requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
	});

export function useFactorPage<T>(
	userId: string,
	resource: string,
	loadPage: (userId: string, page: number) => Promise<FactorPage<T>>,
) {
	const [data, setData] = useState(undefined as FactorPage<T> | undefined);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState(undefined as string | undefined);
	const version = useRef(0);
	const load = useCallback(
		async (page = 1) => {
			const current = ++version.current;
			setLoading(true);
			setError(undefined);
			await afterPaint();
			if (current !== version.current) return;
			try {
				const next = await loadPage(userId, page);
				if (current !== version.current) return;
				setData(next);
				writeFactorCache(userId, resource, next);
			} catch (loadError) {
				if (current === version.current) setError(formatFactorError(loadError));
			} finally {
				if (current === version.current) setLoading(false);
			}
		},
		[loadPage, resource, userId],
	);

	useEffect(() => {
		setData(readFactorCache(userId, resource) as FactorPage<T> | undefined);
		void load();
		return () => {
			version.current += 1;
		};
	}, [load, resource, userId]);

	return { data, error, loading, load };
}
