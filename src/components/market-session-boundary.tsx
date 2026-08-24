import {
	MarketRealtimeConnection,
	MarketSessionProvider,
} from "@/lib/market-session";
import type { ReactNode } from "react";

export function MarketSessionBoundary({
	userId,
	realtime,
	children,
}: {
	userId: string;
	realtime: boolean;
	children: ReactNode;
}) {
	return (
		<MarketSessionProvider userId={userId}>
			{realtime ? <MarketRealtimeConnection enabled /> : null}
			{children}
		</MarketSessionProvider>
	);
}
