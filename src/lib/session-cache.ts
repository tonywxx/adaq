// User-scoped current-session list caches. Reads render re-entries instantly;
// callers replace values only after a successful background refresh.

const sessionCaches: Map<string, unknown> = new Map();

function sessionCacheKey(userId: string, resource: string): string {
	return `${userId}:${resource}`;
}

export function readSessionCache(
	userId: string | null,
	resource: string,
): unknown {
	if (!userId) return undefined;
	return sessionCaches.get(sessionCacheKey(userId, resource));
}

export function writeSessionCache(
	userId: string | null,
	resource: string,
	value: unknown,
): void {
	if (!userId) return;
	sessionCaches.set(sessionCacheKey(userId, resource), value);
}

export function clearSessionCache(): void {
	sessionCaches.clear();
}
