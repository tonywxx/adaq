export type PasswordCheck = {
	ok: boolean;
	items: Array<{
		key: "length" | "lowercase" | "uppercase" | "digit" | "symbol";
		met: boolean;
	}>;
};

export function checkStrongPassword(password: string): PasswordCheck {
	const items: PasswordCheck["items"] = [
		{ key: "length", met: password.length >= 8 },
		{ key: "lowercase", met: /[a-z]/.test(password) },
		{ key: "uppercase", met: /[A-Z]/.test(password) },
		{ key: "digit", met: /\d/.test(password) },
		{ key: "symbol", met: /[^A-Za-z0-9]/.test(password) },
	];

	return {
		ok: items.every((item) => item.met),
		items,
	};
}
