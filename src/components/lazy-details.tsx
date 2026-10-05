import { useState, type ComponentProps, type ReactNode } from "react";

export function LazyDetails({
	summary,
	children,
	onToggle,
	...props
}: Omit<ComponentProps<"details">, "children"> & {
	summary: ReactNode;
	children: () => ReactNode;
}) {
	const [expanded, setExpanded] = useState(Boolean(props.open));
	return (
		<details
			{...props}
			onToggle={(event) => {
				setExpanded(event.currentTarget.open);
				onToggle?.(event);
			}}
		>
			{summary}
			{expanded && children()}
		</details>
	);
}
