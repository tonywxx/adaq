import { create } from "zustand";
import type { InstrumentSearchResult } from "@/lib/eastmoney";

interface ActiveInstState {
	activeInst: InstrumentSearchResult | null;
	setActiveInst: (instrument: InstrumentSearchResult) => void;
}

export const useActiveInstStore = create<ActiveInstState>((set) => ({
	activeInst: null,
	setActiveInst: (instrument) => set({ activeInst: instrument }),
}));
