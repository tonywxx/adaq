import * as React from "react";

function composeRefs<T>(
	...refs: Array<React.Ref<T> | undefined>
): React.RefCallback<T> {
	return (node) => {
		for (const ref of refs) {
			if (!ref) continue;
			if (typeof ref === "function") {
				ref(node);
			} else {
				ref.current = node;
			}
		}
	};
}

function mergeProps(
	slotProps: React.HTMLAttributes<HTMLElement>,
	childProps: React.HTMLAttributes<HTMLElement>,
) {
	const props = { ...slotProps, ...childProps };

	if (slotProps.className || childProps.className) {
		props.className = [slotProps.className, childProps.className]
			.filter(Boolean)
			.join(" ");
	}

	if (slotProps.style || childProps.style) {
		props.style = { ...slotProps.style, ...childProps.style };
	}

	for (const key of Object.keys(slotProps) as Array<
		keyof React.HTMLAttributes<HTMLElement>
	>) {
		const slotValue = slotProps[key];
		const childValue = childProps[key];

		if (
			/^on[A-Z]/.test(key) &&
			typeof slotValue === "function" &&
			typeof childValue === "function"
		) {
			props[key] = ((event: React.SyntheticEvent<HTMLElement>) => {
				childValue(event);
				if (!event.defaultPrevented) {
					slotValue(event);
				}
			}) as never;
		}
	}

	return props;
}

const Root = React.forwardRef<HTMLElement, React.HTMLAttributes<HTMLElement>>(
	({ children, ...props }, ref) => {
		if (!React.isValidElement(children)) {
			return null;
		}

		const child = children as React.ReactElement<
			React.HTMLAttributes<HTMLElement> & { ref?: React.Ref<HTMLElement> }
		>;

		return React.cloneElement(child, {
			...mergeProps(props, child.props),
			ref: composeRefs(ref, child.props.ref),
		} as React.HTMLAttributes<HTMLElement>);
	},
);
Root.displayName = "Slot";

function Slottable({ children }: { children: React.ReactNode }) {
	return <>{children}</>;
}

export { Root, Root as Slot, Slottable };
