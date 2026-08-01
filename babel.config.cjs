module.exports = {
	sourceType: "unambiguous",
	presets: [
		["@babel/preset-env", { targets: { node: "current" } }],
		["@babel/preset-typescript", { ignoreExtensions: true }],
		["@babel/preset-react", { runtime: "automatic" }],
	],
	plugins: [["babel-plugin-transform-import-meta", { module: "CommonJS" }]],
};
