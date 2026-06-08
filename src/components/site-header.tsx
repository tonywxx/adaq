export function SiteHeader() {
	return (
		<header
			className="flex h-(--header-height) shrink-0 items-center gap-2 border-b transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-(--header-height)"
			style={{ WebkitAppRegion: "drag" }}
		>
			<div className="flex w-full items-center px-4 lg:px-6">
				<h1 className="text-base font-medium">Header</h1>
			</div>
		</header>
	);
}
