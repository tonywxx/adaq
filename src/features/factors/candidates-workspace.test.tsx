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
import type { ResearchEvidenceProjection } from "@/features/research/research-context-preflight";

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

const candidateFeatureDataset = {
	datasetId: "feature-dataset-1",
	requestHash: "a".repeat(64),
	featurePlanHash: "b".repeat(64),
	contentSha256: "e".repeat(64),
	outputNames: ["close", "return"],
};

const candidateContext: ResearchEvidenceProjection = {
	contextRevision: 2,
	contextHash: "c".repeat(64),
	market: "crypto",
	venue: "okx",
	rangeStartMs: 1,
	rangeEndMs: 2,
	snapshotId: "snapshot-1",
	universeId: "universe-1",
	evidence: [
		{
			id: "feature-dataset-1",
			lineageHash: "d".repeat(64),
			userId: "user-1",
			market: "crypto",
			venue: "okx",
			snapshotId: "snapshot-1",
			universeId: "universe-1",
			featureId: "feature-dataset-1",
			grade: "provider-graded",
			accessible: true,
			complete: true,
			fresh: true,
		},
	],
	featureDataset: candidateFeatureDataset,
};

const candidatePredecessor: NonNullable<FactorCandidateView["predecessor"]> = {
	...candidateContext,
	userId: "user-1",
	featureDataset: candidateFeatureDataset,
};

function candidate(
	name: string,
	predecessor?: FactorCandidateView["predecessor"],
): FactorCandidateView {
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
		predecessor,
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

function mount(
	adapter: FactorAdapter,
	userId = "user-1",
	context: ResearchEvidenceProjection | null = candidateContext,
) {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	return { container, root, userId, adapter, context };
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

function setInputValue(
	element: HTMLInputElement | HTMLTextAreaElement,
	value: string,
) {
	const prototype =
		element instanceof HTMLTextAreaElement
			? HTMLTextAreaElement.prototype
			: HTMLInputElement.prototype;
	const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
	if (!setter) throw new Error("form value setter not found");
	setter.call(element, value);
	element.dispatchEvent(new Event("input", { bubbles: true }));
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
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
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
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
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
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
		);
	});
	await settle();

	const nameInput = mounted.container.querySelector(
		"input",
	) as HTMLInputElement | null;
	if (!nameInput) throw new Error("candidate name input not found");
	await act(async () => {
		setInputValue(nameInput, "Demo");
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

test("rejects references outside the handed-off Feature outputs", async () => {
	const publishCandidate = jest.fn();
	const mounted = mount(makeAdapter({ publishCandidate }));

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
		);
	});
	await settle();

	const nameInput = mounted.container.querySelector(
		"input",
	) as HTMLInputElement | null;
	const slots = mounted.container.querySelector(
		"textarea",
	) as HTMLTextAreaElement | null;
	if (!nameInput || !slots) throw new Error("candidate form not found");
	await act(async () => {
		setInputValue(nameInput, "Demo");
		setInputValue(slots, "not-in-dataset");
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

	expect(publishCandidate).not.toHaveBeenCalled();
	expect(mounted.container.textContent).toContain(
		i18n.t("factors.candidates.missingFeatureOutput", {
			name: "not-in-dataset",
		}),
	);

	await unmount(mounted.root, mounted.container);
});

test("shows the retained predecessor during Candidate inspection", async () => {
	const mounted = mount(
		makeAdapter({
			listCandidates: async () =>
				page([candidate("Inspectable", candidatePredecessor)]),
		}),
	);

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
		);
	});
	await settle();

	expect(mounted.container.textContent).toContain("feature-dataset-1");
	expect(mounted.container.textContent).toContain("r2");
	expect(mounted.container.textContent).toContain("Handed-off context");

	await unmount(mounted.root, mounted.container);
});

test("localizes the discovery context without changing canonical IDs", async () => {
	await act(async () => {
		await i18n.changeLanguage("zh-CN");
	});
	const mounted = mount(makeAdapter());

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
		);
	});
	await settle();

	expect(mounted.container.textContent).toContain("已交接的 Factor 上下文");
	expect(mounted.container.textContent).toContain("feature-dataset-1");
	expect(mounted.container.textContent).toContain("b".repeat(64));

	await unmount(mounted.root, mounted.container);
	await act(async () => {
		await i18n.changeLanguage("en-US");
	});
});

test("blocks Candidate publication until accepted context is handed off", async () => {
	const publishCandidate = jest.fn();
	const mounted = mount(makeAdapter({ publishCandidate }), "user-1", null);

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
		);
	});
	await settle();

	expect(mounted.container.textContent).toContain(
		i18n.t("factors.candidates.contextRequired"),
	);
	const publishButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) =>
		button.textContent?.includes(i18n.t("factors.candidates.publish")),
	);
	expect(publishButton).toHaveProperty("disabled", true);
	expect(publishCandidate).not.toHaveBeenCalled();

	await unmount(mounted.root, mounted.container);
});

test("publishes against the handed-off Feature Dataset identity", async () => {
	const publishCandidate = jest.fn().mockResolvedValue(candidate("published"));
	const mounted = mount(makeAdapter({ publishCandidate }));

	await act(async () => {
		mounted.root.render(
			<CandidatesWorkspace
				userId={mounted.userId}
				adapter={mounted.adapter}
				context={mounted.context}
			/>,
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

	expect(publishCandidate).toHaveBeenCalledWith(
		"user-1",
		expect.objectContaining({
			featureSlots: [{ name: "close" }],
			source: {
				kind: "declarative",
				definition: expect.objectContaining({
					featurePlanHash: "b".repeat(64),
					operatorCatalogVersion: "adaq-feature-operator-catalog@1.0.0",
				}),
			},
		}),
		expect.objectContaining({ name: "Demo" }),
	);

	await unmount(mounted.root, mounted.container);
});
