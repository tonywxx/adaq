/** @type {import('jest').Config} */
export default {
	testEnvironment: "node",
	roots: ["<rootDir>/src"],
	transform: {
		"^.+\\.[tj]sx?$": "babel-jest",
	},
	moduleNameMapper: {
		"^@/(.*)$": "<rootDir>/src/$1",
		"^(\\.{1,2}/.*)\\.js$": "$1",
	},
};
