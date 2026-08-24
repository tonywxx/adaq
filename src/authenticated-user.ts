import { createContext, useContext } from "react";

export const AuthenticatedUserContext = createContext<string | null>(null);

export function useAuthenticatedUserId() {
	const userId = useContext(AuthenticatedUserContext);
	if (!userId) {
		throw new Error("AuthenticatedUserContext is required for the workspace");
	}
	return userId;
}
