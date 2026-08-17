import { useId } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function Field({
	label,
	value,
	onChange,
	type = "text",
	placeholder,
	mono = false,
}: {
	label: string;
	value: string;
	onChange: (value: string) => void;
	type?: string;
	placeholder?: string;
	mono?: boolean;
}) {
	const id = useId();
	return (
		<div className="grid gap-1.5">
			<Label htmlFor={id}>{label}</Label>
			<Input
				id={id}
				type={type}
				value={value}
				placeholder={placeholder}
				className={mono ? "font-mono text-xs" : undefined}
				onChange={(event) => onChange(event.target.value)}
			/>
		</div>
	);
}

export function TextField({
	label,
	value,
	onChange,
	hint,
}: {
	label: string;
	value: string;
	onChange: (value: string) => void;
	hint?: string;
}) {
	const id = useId();
	const hintId = hint ? `${id}-hint` : undefined;
	return (
		<div className="grid gap-1.5">
			<Label htmlFor={id}>{label}</Label>
			<textarea
				id={id}
				className="min-h-24 w-full rounded-md border bg-background px-3 py-2 font-mono text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
				value={value}
				onChange={(event) => onChange(event.target.value)}
				aria-label={label}
				aria-describedby={hintId}
			/>
			{hint ? (
				<p id={hintId} className="text-xs text-muted-foreground">
					{hint}
				</p>
			) : null}
		</div>
	);
}

export function JsonEditor({
	label,
	value,
	onChange,
	hint,
}: {
	label: string;
	value: string;
	onChange: (value: string) => void;
	hint?: string;
}) {
	return (
		<TextField label={label} value={value} onChange={onChange} hint={hint} />
	);
}

export function Detail({
	label,
	value,
	mono = false,
}: {
	label: string;
	value: string;
	mono?: boolean;
}) {
	const className = mono
		? "mt-1 break-all font-mono text-xs"
		: "mt-1 break-all font-medium";
	return (
		<div className="min-w-0">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className={className}>{value}</dd>
		</div>
	);
}
