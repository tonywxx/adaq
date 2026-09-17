import { botDisplayName } from "./bots-page";

test("Bot cards identify the scheduled market before their opaque ID", () => {
	expect(
		botDisplayName({ type: "ema-double-cross", instrumentId: "okx:ETH-USDT" }),
	).toBe("EMA Double-Cross · ETH-USDT");
	expect(
		botDisplayName({
			type: "scheduled-cross-section",
			universeId: "universe",
			instruments: ["okx:BTC-USDT", "okx:ETH-USDT"],
		}),
	).toBe("Cross-Section · BTC-USDT, ETH-USDT");
});
