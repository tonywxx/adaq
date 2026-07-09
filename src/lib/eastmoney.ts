import em from "@/eastmoney/eastmoney-url.json";
import { createHttpClient, isTauriRuntime } from "@/lib/http";

const EASTMONEY_BASE = em.searchInstrument.base;
const EASTMONEY_WEB_PROXY = "/em.searchInstrument";
const eastmoneyClient = createHttpClient(
	{
		baseURL: isTauriRuntime() ? EASTMONEY_BASE : EASTMONEY_WEB_PROXY,
	},
	undefined,
	{
		transport: "auto",
	},
);

interface EastmoneyCodetableItem {
	code?: string;
	innerCode?: string;
	shortName?: string;
	securityTypeName?: string;
	pinyin?: string;
}

interface EastmoneyCodetableResponse {
	msg?: string;
	pageIndex?: number;
	pageSize?: number;
	result?: EastmoneyCodetableItem[];
	searchId?: string;
}

export interface InstrumentSearchResult {
	market: string;
	symbol: string;
	code: string;
	name: string;
	pinyin: string;
	id: string;
}

export async function searchEastmoneyInstruments(
	input: string,
	signal?: AbortSignal,
) {
	const query = input.trim();

	if (!query) {
		return [];
	}

	const response = await eastmoneyClient.get<EastmoneyCodetableResponse>(
		em.searchInstrument.uri,
		{
			allowConcurrent: true,
			signal,
			params: {
				client: "web",
				clientType: "webSuggest",
				clientVersion: "lastest",
				pageIndex: 1,
				pageSize: 5,
				_: Date.now(),
				type: "1,2,25,27",
				keyword: query,
			},
		},
	);

	return (response.result ?? []).flatMap((item) => {
		const result = normalizeCodetableItem(item);
		return result ? [result] : [];
	});
}

function normalizeCodetableItem(
	item: EastmoneyCodetableItem,
): InstrumentSearchResult | null {
	const code = item.code?.trim();
	const name = item.shortName?.trim();

	if (!code || !name) {
		return null;
	}

	return {
		market: item.securityTypeName?.trim() || "证券",
		symbol: code,
		code,
		name,
		pinyin: (item.pinyin ?? "").toUpperCase(),
		id: item.innerCode || code,
	};
}
