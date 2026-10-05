/** @jest-environment jsdom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { useFactorPage } from "./factor-workspace-data";
import type { FactorPage } from "./factor-types";

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

test("complete catalogs cap concurrent reads and stop queuing after navigation", async () => {
	const pending = new Map<number, (page: FactorPage<number>) => void>();
	let active = 0;
	let maximum = 0;
	const loadPage = jest.fn(
		(_userId: string, page: number) =>
			new Promise<FactorPage<number>>((resolve) => {
				active += 1;
				maximum = Math.max(maximum, active);
				pending.set(page, (result) => {
					active -= 1;
					resolve(result);
				});
			}),
	);
	const container = document.createElement("div");
	const root = createRoot(container);
	function Catalog() {
		const result = useFactorPage("catalog-test", "bounded", loadPage, {
			allPages: true,
		});
		return <p>{result.loading ? "Loading" : "Ready"}</p>;
	}
	const settle = () =>
		act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 120));
		});
	await act(async () => root.render(<Catalog />));
	await settle();
	await act(async () =>
		pending.get(1)!({ items: [1], page: 1, pageSize: 10, total: 130 }),
	);
	await settle();
	expect([...pending.keys()]).toEqual([1, 2, 3, 4, 5]);
	expect(maximum).toBe(4);
	await act(async () => root.unmount());
	await act(async () => {
		for (const page of [2, 3, 4, 5])
			pending.get(page)!({ items: [page], page, pageSize: 10, total: 130 });
	});
	await settle();
	expect(loadPage).toHaveBeenCalledTimes(5);
});
