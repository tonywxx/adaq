// Babel 8's TypeScript transform leaves call-site type arguments behind.
function stripTypeArguments() {
	return {
		name: "strip-type-arguments",
		visitor: {
			CallExpression(path) {
				path.node.typeParameters = null;
				path.node.typeArguments = null;
			},
			NewExpression(path) {
				path.node.typeParameters = null;
				path.node.typeArguments = null;
			},
			OptionalCallExpression(path) {
				path.node.typeParameters = null;
				path.node.typeArguments = null;
			},
		},
	};
}

module.exports = {
	sourceType: "unambiguous",
	presets: [
		["@babel/preset-env", { targets: { node: "current" } }],
		["@babel/preset-typescript", { ignoreExtensions: true }],
		["@babel/preset-react", { runtime: "automatic" }],
	],
	plugins: [
		["babel-plugin-transform-import-meta", { module: "CommonJS" }],
		stripTypeArguments,
	],
};
