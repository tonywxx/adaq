export function identifierName(
	id: string,
	label: string,
	name?: string | null,
) {
	const readableName = name?.trim();
	return readableName && readableName !== id.trim() ? readableName : label;
}

// Only shorten what needs shortening, so short identities never carry a dangling ellipsis.
export function truncateIdentifier(id: string, length: number) {
	return id.length > length ? `${id.slice(0, length)}…` : id;
}

// Native options need text content; keep their complete identity alongside the label.
export function identifierLabel(
	id: string,
	label: string,
	name?: string | null,
) {
	return `${identifierName(id, label, name)} · ${id}`;
}
