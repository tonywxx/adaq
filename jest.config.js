/** @type {import('jest').Config} */
export default {
	testEnvironment: "node",
	setupFiles: ["<rootDir>/src/test-setup.js"],
	roots: ["<rootDir>/src"],
	transform: {
		"^.+\\.[tj]sx?$": "babel-jest",
	},
	moduleNameMapper: {
		"^@/(.*)$": "<rootDir>/src/$1",
		"^(\\.{1,2}/.*)\\.js$": "$1",
	},
};
