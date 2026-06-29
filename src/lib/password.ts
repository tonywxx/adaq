export type PasswordCheck = {
	ok: boolean;
	items: Array<{ label: string; met: boolean }>;
};

export function checkStrongPassword(password: string): PasswordCheck {
	const items = [
		{ label: "At least 8 characters", met: password.length >= 8 },
		{ label: "Lowercase letter", met: /[a-z]/.test(password) },
		{ label: "Uppercase letter", met: /[A-Z]/.test(password) },
		{ label: "Digit", met: /\d/.test(password) },
		{ label: "Symbol", met: /[^A-Za-z0-9]/.test(password) },
	];

	return {
		ok: items.every((item) => item.met),
		items,
	};
}
