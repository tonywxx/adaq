/// <reference types="vite/client" />

import "react";

interface ImportMetaEnv {
	readonly VITE_SUPABASE_URL?: string;
	readonly VITE_SUPABASE_PUBLISHABLE_KEY?: string;
	readonly VITE_SUPABASE_ANON_KEY?: string;
}

// biome-ignore lint/correctness/noUnusedVariables: Ambient Vite ImportMeta augmentation is consumed by TypeScript.
interface ImportMeta {
	readonly env: ImportMetaEnv;
}

declare module "react" {
	interface CSSProperties {
		WebkitAppRegion?: "drag" | "no-drag";
	}
}
