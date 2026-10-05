import type { RecordPage } from "@/components/record-pagination";
import { afterPaint } from "@/features/factors/factor-workspace-data";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";

export function useRecordPage<T>(
	resource: string,
	userId: string,
	command: string,
	args: Record<string, unknown> = {},
	options: { enabled?: boolean; refetchInterval?: number } = {},
) {
	const [page, setPage] = useState(1);
	const query = useQuery({
		queryKey: [resource, userId, args, page],
		queryFn: async () => {
			await afterPaint();
			return invoke<RecordPage<T>>(command, { ...args, page });
		},
		enabled: Boolean(userId) && options.enabled !== false,
		placeholderData: (previous, previousQuery) =>
			previousQuery?.queryKey[1] === userId &&
			JSON.stringify(previousQuery.queryKey[2]) === JSON.stringify(args)
				? previous
				: undefined,
		retry: false,
		refetchInterval: options.refetchInterval,
	});
	return { ...query, setPage, page: query.data?.page ?? page };
}
