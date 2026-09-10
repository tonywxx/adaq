export function identifierName(
	id: string,
	label: string,
	name?: string | null,
) {
	const readableName = name?.trim();
	return readableName && readableName !== id.trim() ? readableName : label;
}

// Undefined when nothing human-readable is known: the caller then shows the value alone.
export function identifierOptionalName(name?: string | null, id = "") {
	const readableName = name?.trim();
	return readableName && readableName !== id.trim() ? readableName : undefined;
}

// Long identities read as "abc…xyz"; the complete value belongs in the tooltip.
export function abbreviateIdentifier(id: string, edge = 3) {
	return id.length > edge * 2 ? `${id.slice(0, edge)}…${id.slice(-edge)}` : id;
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
