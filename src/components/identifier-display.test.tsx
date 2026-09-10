/** @jest-environment jsdom */
import { renderToStaticMarkup } from "react-dom/server";
import { changeInterfaceLocale, i18n } from "@/lib/i18n";
import { identifierLabel } from "@/lib/identifier-display";
import { IdentifierDisplay } from "./identifier-display";

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
