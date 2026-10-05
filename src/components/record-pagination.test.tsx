/** @jest-environment jsdom */
import "@/lib/i18n";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { PaginatedList } from "./record-pagination";
import { LazyDetails } from "./lazy-details";

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

test("large lists render ten records, retain original indices and clamp after shrink", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	const renderItem = jest.fn((item: number, index: number) => (
		<p key={item} data-index={index}>
			{item}
		</p>
	));
	const render = (items: number[]) =>
		root.render(
			<PaginatedList items={items} label="Records">
				{renderItem}
			</PaginatedList>,
		);
	await act(async () =>
		render(Array.from({ length: 20_000 }, (_, index) => index)),
	);
	expect(renderItem).toHaveBeenCalledTimes(10);
	expect(container.querySelectorAll("p")).toHaveLength(10);
	await act(async () =>
		(container.querySelectorAll("button")[1] as HTMLButtonElement).click(),
	);
	expect(container.querySelector("p")?.textContent).toBe("10");
	expect(container.querySelector("p")?.getAttribute("data-index")).toBe("10");
	await act(async () => render([1, 2]));
	expect(container.querySelectorAll("p")).toHaveLength(2);
	expect(container.querySelector("nav")).toBeNull();
	await act(async () => root.unmount());
});

test("collapsed evidence never formats its contents and remains inspectable", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	const format = jest.fn(() => <pre>retained evidence</pre>);
	await act(async () =>
		root.render(
			<LazyDetails summary={<summary>Evidence</summary>}>{format}</LazyDetails>,
		),
	);
	expect(format).not.toHaveBeenCalled();
	const details = container.querySelector("details")!;
	await act(async () => {
		details.open = true;
		details.dispatchEvent(new Event("toggle"));
	});
	expect(container.textContent).toContain("retained evidence");
	await act(async () => root.unmount());
});
