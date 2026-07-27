/// <reference types="node" />

import { readFileSync } from "node:fs";

test("Log out uses the Base UI menu click event", () => {
	const source = readFileSync(
		new URL("./nav-user.tsx", import.meta.url),
		"utf8",
	);

	expect(source).toMatch(
		/<DropdownMenuItem onClick=\{\(\) => supabase\?\.auth\.signOut\(\)\}>\s*<LogOutIcon \/>\s*Log out/,
	);
});
