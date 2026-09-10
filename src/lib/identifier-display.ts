export function identifierName(
	id: string,
	label: string,
	name?: string | null,
) {
	const readableName = name?.trim();
	return readableName && readableName !== id.trim() ? readableName : label;
}

// Native options need text content; keep their complete identity alongside the label.
export function identifierLabel(
	id: string,
	label: string,
	name?: string | null,
) {
	return `${identifierName(id, label, name)} · ${id}`;
}
