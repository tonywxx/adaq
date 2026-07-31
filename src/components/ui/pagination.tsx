import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react";
import * as React from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

function Pagination({ className, ...props }: React.ComponentProps<"nav">) {
	return (
		<nav
			role="navigation"
			aria-label="pagination"
			data-slot="pagination"
			className={cn("mx-auto flex w-full justify-center", className)}
			{...props}
		/>
	);
}

function PaginationContent({
	className,
	...props
}: React.ComponentProps<"ul">) {
	return (
		<ul
			data-slot="pagination-content"
			className={cn("flex items-center gap-0.5", className)}
			{...props}
		/>
	);
}

function PaginationItem(props: React.ComponentProps<"li">) {
	return <li data-slot="pagination-item" {...props} />;
}

function PaginationPrevious(props: React.ComponentProps<typeof Button>) {
	return (
		<Button aria-label="Go to previous page" variant="ghost" {...props}>
			<ChevronLeftIcon data-icon="inline-start" />
			Previous
		</Button>
	);
}

function PaginationNext(props: React.ComponentProps<typeof Button>) {
	return (
		<Button aria-label="Go to next page" variant="ghost" {...props}>
			Next
			<ChevronRightIcon data-icon="inline-end" />
		</Button>
	);
}

export {
	Pagination,
	PaginationContent,
	PaginationItem,
	PaginationNext,
	PaginationPrevious,
};
