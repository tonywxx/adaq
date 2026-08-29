import { useCallback, useEffect, useRef, useState } from "react";
import {
	formatFactorError,
	readFactorCache,
	writeFactorCache,
} from "./factor-data";
import type { FactorPage } from "./factor-types";

// ponytail: cap client fan-out; add cursor streaming if evidence grows beyond this bound.
const MAX_FACTOR_PAGE_REQUESTS = 1_000;

export const afterPaint = (): Promise<void> =>
	new Promise((resolve) => {
		if (
			typeof requestAnimationFrame === "undefined" ||
			document.visibilityState === "hidden"
		) {
			resolve();
			return;
		}
		requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
		window.setTimeout(resolve, 100);
	});

export function useFactorPage<T>(
	userId: string,
	resource: string,
	loadPage: (userId: string, page: number) => Promise<FactorPage<T>>,
	options: { allPages?: boolean } = {},
) {
	const { allPages = false } = options;
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
				const first = await loadPage(userId, page);
				if (current !== version.current) return;
				if (
					allPages &&
					page === 1 &&
					(!Number.isSafeInteger(first.pageSize) ||
						first.pageSize <= 0 ||
						!Number.isSafeInteger(first.total) ||
						first.total < 0)
				) {
					throw new Error("Factor page metadata is invalid");
				}
				const totalPages =
					allPages && page === 1 ? Math.ceil(first.total / first.pageSize) : 1;
				if (totalPages > MAX_FACTOR_PAGE_REQUESTS) {
					throw new Error("Factor page count exceeds the safe limit");
				}
				const next =
					allPages && page === 1 && totalPages > 1
						? {
								...first,
								items: [
									...first.items,
									...(
										await Promise.all(
											Array.from({ length: totalPages - 1 }, (_, index) =>
												loadPage(userId, index + 2),
											),
										)
									).flatMap((result) => result.items),
								],
							}
						: first;
				if (current !== version.current) return;
				setData(next);
				writeFactorCache(userId, resource, next);
			} catch (loadError) {
				if (current === version.current) setError(formatFactorError(loadError));
			} finally {
				if (current === version.current) setLoading(false);
			}
		},
		[allPages, loadPage, resource, userId],
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
