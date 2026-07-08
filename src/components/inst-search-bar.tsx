import { useQuery } from "@tanstack/react-query";
import { LoaderCircleIcon, SearchIcon } from "lucide-react";
import * as React from "react";
import { Input } from "@/components/ui/input";
import { useActiveInstStore } from "@/lib/active-inst-store";
import {
	type InstrumentSearchResult,
	searchEastmoneyInstruments,
} from "@/lib/eastmoney";

const SEARCH_DEBOUNCE_MS = 250;

export function InstSearchBar() {
	const [query, setQuery] = React.useState("");
	const [isOpen, setIsOpen] = React.useState(false);
	const inputRef = React.useRef<HTMLInputElement>(null);
	const setActiveInst = useActiveInstStore((state) => state.setActiveInst);
	const debouncedQuery = useDebouncedValue(query.trim(), SEARCH_DEBOUNCE_MS);
	const hasQuery = query.trim().length > 0;
	const shouldSearch = debouncedQuery.length > 0;
	const {
		data = [],
		isError,
		isFetching,
	} = useQuery({
		queryKey: ["eastmoney-suggest", debouncedQuery],
		queryFn: ({ signal }) => searchEastmoneyInstruments(debouncedQuery, signal),
		enabled: shouldSearch,
		staleTime: 30_000,
	});

	React.useEffect(() => {
		const handleKeyDown = (event: KeyboardEvent) => {
			if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
				event.preventDefault();
				setIsOpen(true);
				inputRef.current?.focus();
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, []);

	return (
		<div
			className="relative w-[280px] max-w-[32vw] shrink-0"
			onPointerDown={(event) => event.stopPropagation()}
		>
			<SearchIcon className="-translate-y-1/2 pointer-events-none absolute top-1/2 left-2.5 size-4 text-muted-foreground" />
			<Input
				ref={inputRef}
				type="search"
				aria-label="Search instruments"
				aria-expanded={isOpen && hasQuery}
				className="h-8 rounded-md bg-background/70 pr-15 pl-8 text-sm shadow-none"
				placeholder="Search"
				value={query}
				onBlur={() => window.setTimeout(() => setIsOpen(false), 120)}
				onChange={(event) => {
					setQuery(event.target.value);
					setIsOpen(true);
				}}
				onFocus={() => setIsOpen(true)}
				onKeyDown={(event) => {
					if (event.key === "Escape") {
						setIsOpen(false);
						inputRef.current?.blur();
					}
				}}
			/>
			<span className="-translate-y-1/2 pointer-events-none absolute top-1/2 right-2 rounded border border-border px-1.5 py-0.5 text-[10px] leading-none text-muted-foreground">
				Cmd K
			</span>
			{isOpen && hasQuery ? (
				<SearchPanel
					results={data}
					isError={isError}
					isFetching={isFetching}
					hasSearch={shouldSearch}
					onSelect={(instrument) => {
						setActiveInst(instrument);
						setQuery(`${instrument.symbol} ${instrument.name}`);
						setIsOpen(false);
						inputRef.current?.blur();
					}}
				/>
			) : null}
		</div>
	);
}

function SearchPanel({
	results,
	isError,
	isFetching,
	hasSearch,
	onSelect,
}: {
	results: InstrumentSearchResult[];
	isError: boolean;
	isFetching: boolean;
	hasSearch: boolean;
	onSelect: (instrument: InstrumentSearchResult) => void;
}) {
	return (
		<div className="absolute top-[calc(100%+0.5rem)] right-0 z-60 w-[min(92vw,440px)] overflow-hidden rounded-md border border-border bg-popover text-popover-foreground shadow-md ring-1 ring-foreground/10">
			<div className="flex items-center justify-between border-b border-border px-3 py-2 text-xs text-muted-foreground">
				<span>Related instruments</span>
				{isFetching ? (
					<LoaderCircleIcon className="size-3.5 animate-spin" />
				) : null}
			</div>
			<div className="max-h-80 overflow-y-auto py-1">
				{isError ? (
					<div className="px-3 py-3 text-sm text-destructive">
						Search failed.
					</div>
				) : results.length > 0 ? (
					results.map((result) => (
						<button
							type="button"
							className="grid w-full grid-cols-[4rem_5.5rem_1fr_5rem] items-center gap-3 px-3 py-2 text-left text-sm hover:bg-muted/60 focus-visible:bg-muted/60 focus-visible:outline-none"
							key={result.id}
							onClick={() => onSelect(result)}
						>
							<span className="text-muted-foreground">{result.market}</span>
							<span className="font-medium tabular-nums">{result.symbol}</span>
							<span className="truncate font-medium">{result.name}</span>
							<span className="truncate text-right font-semibold text-primary">
								{result.pinyin}
							</span>
						</button>
					))
				) : hasSearch && !isFetching ? (
					<div className="px-3 py-3 text-sm text-muted-foreground">
						No matches.
					</div>
				) : null}
			</div>
		</div>
	);
}

function useDebouncedValue(value: string, delay: number) {
	const [debouncedValue, setDebouncedValue] = React.useState(value);

	React.useEffect(() => {
		const timer = window.setTimeout(() => setDebouncedValue(value), delay);
		return () => window.clearTimeout(timer);
	}, [value, delay]);

	return debouncedValue;
}
