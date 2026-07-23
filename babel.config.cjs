module.exports = {
	sourceType: "unambiguous",
	presets: [
		["@babel/preset-env", { targets: { node: "current" } }],
		"@babel/preset-typescript",
	],
	plugins: [
		"@babel/plugin-syntax-import-meta",
		["babel-plugin-transform-import-meta", { module: "CommonJS" }],
	],
};
