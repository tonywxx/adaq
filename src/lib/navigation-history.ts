import { useCallback, useEffect, useState } from "react";
import { useRouter, useRouterState } from "@tanstack/react-router";

const tabStateKey = "__adaqTab";

type TabState = { scope: string; value: string; owner?: string };

export function historyTabValue(
	state: unknown,
	scope: string,
	fallback: string,
	owner?: string,
) {
	if (!state || typeof state !== "object") return fallback;
	const tab = (state as Record<string, unknown>)[tabStateKey];
	if (!tab || typeof tab !== "object") return fallback;
	const { owner: storedOwner, scope: storedScope, value } = tab as TabState;
	return storedScope === scope &&
		storedOwner === owner &&
		typeof value === "string"
		? value
		: fallback;
}

export function useHistoryTab(scope: string, fallback: string, owner?: string) {
	const router = useRouter();
	const state = useRouterState({
		select: (routerState) => routerState.location.state,
	});
	const value = historyTabValue(state, scope, fallback, owner);
	const setValue = useCallback(
		(nextValue: string) => {
			if (nextValue === value) return;
			router.history.push(router.history.location.href, {
				...router.history.location.state,
				[tabStateKey]: { owner, scope, value: nextValue },
			});
		},
		[owner, router, scope, value],
	);

	return [value, setValue] as const;
}

export function useHistoryControls() {
	const router = useRouter();
	const index = useRouterState({
		select: (state) => state.location.state.__TSR_index,
	});
	const [length, setLength] = useState(() => router.history.length);

	useEffect(
		() => router.history.subscribe(() => setLength(router.history.length)),
		[router],
	);

	return {
		canGoBack: index > 0,
		canGoForward: index < length - 1,
		back: () => router.history.back(),
		forward: () => router.history.forward(),
	};
}
