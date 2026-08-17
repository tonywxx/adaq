/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { clearSessionCache } from "@/lib/session-cache";
import { i18n } from "@/lib/i18n";
import { CandidatesWorkspace } from "./candidates-workspace";
import type { FactorAdapter } from "./factor-adapter";
import type {
	FactorAttemptView,
	FactorCandidateView,
	FactorPage,
} from "./factor-types";

jest.mock("@/components/ui/badge", () => ({
	Badge: ({
		children,
		...props
	}: {
		children?: unknown;
		[key: string]: unknown;
	}) => require("react").createElement("span", props, children),
}));

jest.mock("@/components/ui/button", () => ({
	Button: ({
		children,
		loading,
		loadingText,
		variant: _variant,
		size: _size,
		effect: _effect,
		asChild: _asChild,
		icon: _icon,
		iconPlacement: _iconPlacement,
		loadingIconPlacement: _loadingIconPlacement,
		...props
	}: {
		children?: unknown;
		loading?: boolean;
		loadingText?: string;
		[key: string]: unknown;
	}) =>
		require("react").createElement(
			"button",
			{ ...props, disabled: Boolean(props.disabled) || loading },
			loading && loadingText ? loadingText : children,
		),
}));

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

type Deferred<T> = {
	promise: Promise<T>;
	resolve: (value: T) => void;
	reject: (reason?: unknown) => void;
};

function deferred<T>(): Deferred<T> {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise: Promise<T> = new Promise(
		(
			resolvePromise: (value: T | PromiseLike<T>) => void,
			rejectPromise: (reason?: unknown) => void,
		) => {
			resolve = resolvePromise;
			reject = rejectPromise;
		},
	);
	return { promise, resolve, reject };
}

function page<T>(items: T[]): FactorPage<T> {
	return { items, page: 1, pageSize: 50, total: items.length };
}

function candidate(name: string): FactorCandidateView {
	return {
		candidate: {
			candidateHash: name,
			revision: 1,
			scope: "time-series",
			source: { kind: "declarative" },
		},
		presentation: { name },
		lockedBy: [],
		createdAtMs: 0,
	};
}

function makeAdapter(
	overrides: Partial<{
		listCandidates: FactorAdapter["listCandidates"];
		listAttempts: FactorAdapter["listAttempts"];
		publishCandidate: FactorAdapter["publishCandidate"];
	}> = {},
): FactorAdapter {
	return {
		listCandidates: async () => page([] as FactorCandidateView[]),
		listAttempts: async () => page([] as FactorAttemptView[]),
		publishCandidate: async () => candidate("published"),
		buildCandidate: async () => ({
			attemptId: "attempt-1",
			userId: "user-1",
			kind: "candidate-build",
			requestHash: "request-hash",
			status: "pending",
			completedUnits: 0,
			progressTotal: 1,
			createdAtMs: 0,
			updatedAtMs: 0,
		}),
		...overrides,
	} as unknown as FactorAdapter;
}

function mount(adapter: FactorAdapter, userId = "user-1") {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	return { container, root, userId, adapter };
}

async function unmount(root: Root, container: HTMLDivElement) {
	await act(async () => root.unmount());
	container.remove();
}

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await Promise.resolve();
	});
}

beforeEach(() => {
	clearSessionCache();
	Object.defineProperty(globalThis, "requestAnimationFrame", {
		configurable: true,
		value: undefined,
	});
});

test("renders loading while candidates load and surfaces a list error", async () => {
	const pending = deferred();
	const adapter = makeAdapter({
		listCandidates: async () =>
			pending.promise as Promise<FactorPage<FactorCandidateView>>,
	});
	const mounted = mount(adapter);

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace userId={mounted.userId} adapter={mounted.adapter} />,
		);
	});
	await settle();
	expect(mounted.container.querySelector('[aria-busy="true"]')).not.toBeNull();

	await act(async () => {
		pending.reject(new Error("candidate list failed"));
	});
	await settle();
	expect(
		mounted.container.querySelector('[role="alert"]')?.textContent,
	).toContain("candidate list failed");

	await unmount(mounted.root, mounted.container);
});

test("ignores a stale candidate response after the user changes", async () => {
	const first = deferred();
	const second = deferred();
	const adapter = makeAdapter({
		listCandidates: async (userId) =>
			(userId === "user-a" ? first.promise : second.promise) as Promise<
				FactorPage<FactorCandidateView>
			>,
	});
	const mounted = mount(adapter, "user-a");

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace userId={mounted.userId} adapter={mounted.adapter} />,
		);
	});
	await settle();

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace userId="user-b" adapter={mounted.adapter} />,
		);
	});
	await settle();

	await act(async () => {
		second.resolve(page([candidate("Beta")]));
	});
	await settle();
	expect(mounted.container.textContent).toContain("Beta");

	await act(async () => {
		first.resolve(page([candidate("Alpha")]));
	});
	await settle();
	expect(mounted.container.textContent).toContain("Beta");
	expect(mounted.container.textContent).not.toContain("Alpha");

	await unmount(mounted.root, mounted.container);
});

test("renders publish failures through the workspace seam", async () => {
	const publishCandidate = jest
		.fn()
		.mockRejectedValue(new Error("publish failed"));
	const mounted = mount(makeAdapter({ publishCandidate }));

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace userId={mounted.userId} adapter={mounted.adapter} />,
		);
	});
	await settle();

	const nameInput = mounted.container.querySelector(
		"input",
	) as HTMLInputElement | null;
	if (!nameInput) throw new Error("candidate name input not found");
	const setValue = Object.getOwnPropertyDescriptor(
		HTMLInputElement.prototype,
		"value",
	)?.set;
	if (!setValue) throw new Error("input value setter not found");
	await act(async () => {
		setValue.call(nameInput, "Demo");
		nameInput.dispatchEvent(new Event("input", { bubbles: true }));
	});

	const publishButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) =>
		button.textContent?.includes(i18n.t("factors.candidates.publish")),
	);
	if (!publishButton) throw new Error("publish button not found");
	await act(async () => {
		publishButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
	});
	await settle();

	expect(publishCandidate).toHaveBeenCalled();
	expect(
		mounted.container.querySelector('[role="alert"]')?.textContent,
	).toContain("publish failed");

	await unmount(mounted.root, mounted.container);
});
