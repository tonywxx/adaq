/** @jest-environment jsdom */
import { renderToStaticMarkup } from "react-dom/server";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { changeInterfaceLocale, i18n } from "@/lib/i18n";
import { identifierLabel, truncateIdentifier } from "@/lib/identifier-display";
import { IdentifierDisplay } from "./identifier-display";

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

test("names and localized fallbacks retain complete, distinct raw identities", async () => {
	const ids = ["ab".repeat(32), `${"ab".repeat(31)}cd`];
	try {
		for (const locale of ["en-US", "zh-CN"] as const) {
			await changeInterfaceLocale(locale);
			const label = i18n.t("identifiers.featureDataset");
			const container = document.createElement("div");
			container.innerHTML = renderToStaticMarkup(
				<>
					<IdentifierDisplay id={ids[0]} label={label} name="Momentum signals" />
					<IdentifierDisplay id={ids[1]} label={label} name=" " />
					<select defaultValue={ids[1]}>
						{ids.map((id) => (
							<option key={id} value={id}>
								{identifierLabel(id, label, id)}
							</option>
						))}
					</select>
				</>,
			);
			const codes = container.querySelectorAll("code");
			expect(container.textContent).toContain("Momentum signals");
			expect(codes[1].previousElementSibling?.textContent).toBe(label);
			ids.forEach((id, index) => {
				expect(codes[index].textContent).toContain(id);
			});
			expect(container.querySelector("select")?.value).toBe(ids[1]);
			const options = container.querySelectorAll("option");
			ids.forEach((id, index) => {
				expect(options[index].value).toBe(id);
				expect(options[index].textContent).toBe(`${label} · ${id}`);
			});
		}
	} finally {
		await changeInterfaceLocale("en-US");
	}
});

test("truncated identifiers only gain an ellipsis when something was cut", () => {
	expect(truncateIdentifier("ab".repeat(32), 16)).toBe(`${"ab".repeat(8)}…`);
	expect(truncateIdentifier("short-id", 16)).toBe("short-id");
});

test("compact identifiers abbreviate the value and copy the complete one", async () => {
	const writeText = jest.fn().mockResolvedValue(undefined);
	Object.defineProperty(navigator, "clipboard", {
		value: { writeText },
		configurable: true,
	});
	const id = "ab".repeat(32);
	const container = document.createElement("div");
	document.body.appendChild(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(
			<IdentifierDisplay
				id={id}
				label={i18n.t("identifiers.featureDataset")}
				variant="compact"
			/>,
		);
	});
	expect(container.textContent).toContain("aba…bab");
	expect(container.textContent).not.toContain(id);
	const trigger = container.querySelector("button");
	expect(trigger).not.toBeNull();
	await act(async () => {
		trigger?.click();
	});
	expect(writeText).toHaveBeenCalledWith(id);
	await act(async () => root.unmount());
	container.remove();
});
