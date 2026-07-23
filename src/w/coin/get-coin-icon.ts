// - get crypto coin icon url by coin name
export const getCoinIcon = (cryptoCoin: string) => {
	cryptoCoin = cryptoCoin.toLowerCase();

	// OKX 图标 URL 映射（不同币种的时间戳可能不同）
	const iconMap: Record<string, string> = {
		btc: "btc20230419112752",
		eth: "eth20230419112854",
		sol: "sol20230419112951",
		doge: "doge20230419112833",
		sui: "sui20230414104031",
		hype: "hype20241203112708",
		aster: "aster20250922123241",
		bnb: "bnb20221218121954",
		xrp: "xrp20230419113140",
		zec: "zec20251111171831",
		trx: "trx20230419113015",
		xau: "xau20260108170832",
		avax: "avax20230419112726",
		pepe: "pepe20230814141256",
	};

	if (iconMap[cryptoCoin]) {
		return `https://www.okx.com/cdn/oksupport/asset/currency/icon/${iconMap[cryptoCoin]}.png`;
	} else {
		if (cryptoCoin === "shib") {
			return "https://www.okx.com/cdn/oksupport/asset/currency/icon/shib.png";
		}
		// https://assets.coincap.io/assets/icons/hype@2x.png
		// https://assets.kraken.com/marketing/web/icons-uni-webp/s_btc.webp
		return `https://raw.githubusercontent.com/spothq/cryptocurrency-icons/master/128/color/${cryptoCoin}.png`;
	}
};
