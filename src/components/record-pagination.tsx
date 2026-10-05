import { Button } from "@/components/ui/button";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

export const RECORD_PAGE_SIZE = 10;

export type RecordPage<T> = {
	items: T[];
	page: number;
	pageSize: number;
	total: number;
};

export function RecordPagination({
	page,
	total,
	onPage,
	label,
	busy = false,
	pageSize = RECORD_PAGE_SIZE,
}: {
	page: number;
	total: number;
	onPage: (page: number) => void;
	label: string;
	busy?: boolean;
	pageSize?: number;
}) {
	const { t } = useTranslation();
	const pages = Math.max(1, Math.ceil(total / pageSize));
	return (
		<nav
			aria-label={t("paperTrading.paginationLabel", {
				section: t(label, { defaultValue: label }),
			})}
			className="flex items-center justify-center gap-2 py-2"
		>
			<Button
				type="button"
				size="sm"
				variant="ghost"
				disabled={busy || page <= 1}
				onClick={() => onPage(page - 1)}
			>
				{t("paperTrading.previousPage")}
			</Button>
			<span className="text-xs" aria-live="polite">
				{t("paperTrading.pageOf", { current: page, total: pages })}
			</span>
			<Button
				type="button"
				size="sm"
				variant="ghost"
				disabled={busy || page >= pages}
				onClick={() => onPage(page + 1)}
			>
				{t("paperTrading.nextPage")}
			</Button>
		</nav>
	);
}

export function PaginatedList<T>({
	items,
	children,
	label,
	list = false,
	tableColumns,
}: {
	items: readonly T[];
	children: (item: T, index: number) => ReactNode;
	label: string;
	list?: boolean;
	tableColumns?: number;
}) {
	const [requestedPage, setPage] = useState(1);
	const page = Math.min(
		requestedPage,
		Math.max(1, Math.ceil(items.length / RECORD_PAGE_SIZE)),
	);
	const offset = (page - 1) * RECORD_PAGE_SIZE;
	const controls = (
		<RecordPagination
			page={page}
			total={items.length}
			onPage={setPage}
			label={label}
		/>
	);
	return (
		<>
			{items
				.slice(offset, offset + RECORD_PAGE_SIZE)
				.map((item, index) => children(item, offset + index))}
			{items.length > RECORD_PAGE_SIZE &&
				(tableColumns ? (
					<tr>
						<td colSpan={tableColumns}>{controls}</td>
					</tr>
				) : list ? (
					<li className="list-none">{controls}</li>
				) : (
					controls
				))}
		</>
	);
}
