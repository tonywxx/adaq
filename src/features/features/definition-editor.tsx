import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	FEATURE_SCOPES,
	MARKET_FIELDS,
	OPERATOR_KINDS,
	addMarketInput,
	createEmptyNode,
	defaultNodeId,
	moveNode,
	parseParameterValue,
	removeInput,
	removeNode,
	updateNode,
} from "./features-data";
import type {
	DefinitionDraft,
	FeatureInput,
	FeatureScope,
	MarketField,
} from "./features-types";
import { useState } from "react";
import { useTranslation } from "react-i18next";

// Accessible ordered node editor: reordering happens through labeled
// keyboard-operable buttons, never a drag canvas.
export function DefinitionEditor({
	draft,
	onChange,
}: {
	draft: DefinitionDraft;
	onChange: (draft: DefinitionDraft) => void;
}) {
	const { t } = useTranslation();

	const setScope = (scope: FeatureScope) => {
		onChange({
			...draft,
			scope,
			nodes: draft.nodes.map((node) => ({ ...node, scope })),
		});
	};

	return (
		<div className="space-y-6">
			<fieldset className="space-y-2">
				<legend className="mb-2 text-sm font-medium">
					{t("features.definitions.editor.scope")}
				</legend>
				<Label htmlFor="definition-scope" className="sr-only">
					{t("features.definitions.editor.scope")}
				</Label>
				<select
					id="definition-scope"
					className="w-full max-w-xs rounded-md border bg-transparent px-3 py-2 text-sm"
					value={draft.scope}
					onChange={(event) => setScope(event.target.value as FeatureScope)}
				>
					{FEATURE_SCOPES.map((scope) => (
						<option key={scope} value={scope}>
							{scope}
						</option>
					))}
				</select>
			</fieldset>

			<section aria-label={t("features.definitions.editor.nodes")}>
				<ol className="space-y-4">
					{draft.nodes.map((node, index) => (
						<li key={node.id} className="space-y-3 rounded-md border p-4">
							<div className="flex flex-wrap items-center gap-2">
								<span className="font-mono text-sm font-semibold">
									{index + 1}. {node.id}
								</span>
								<div className="ml-auto flex gap-1">
									<Button
										type="button"
										variant="outline"
										size="sm"
										disabled={index === 0}
										aria-label={`${t("features.definitions.editor.moveUp")}: ${node.id}`}
										onClick={() => onChange(moveNode(draft, index, -1))}
									>
										↑ {t("features.definitions.editor.moveUp")}
									</Button>
									<Button
										type="button"
										variant="outline"
										size="sm"
										disabled={index === draft.nodes.length - 1}
										aria-label={`${t("features.definitions.editor.moveDown")}: ${node.id}`}
										onClick={() => onChange(moveNode(draft, index, 1))}
									>
										↓ {t("features.definitions.editor.moveDown")}
									</Button>
									<Button
										type="button"
										variant="ghost"
										size="sm"
										aria-label={`${t("features.definitions.editor.removeNode")}: ${node.id}`}
										onClick={() => onChange(removeNode(draft, index))}
									>
										{t("features.definitions.editor.removeNode")}
									</Button>
								</div>
							</div>

							<div className="grid gap-3 sm:grid-cols-3">
								<div>
									<Label htmlFor={`operator-${node.id}`}>
										{t("features.definitions.editor.operator")}
									</Label>
									<select
										id={`operator-${node.id}`}
										className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
										value={node.operator.kind}
										onChange={(event) =>
											onChange(
												updateNode(draft, index, {
													operator:
														event.target.value === "indicator"
															? { kind: "indicator", id: "" }
															: { kind: event.target.value },
												}),
											)
										}
									>
										{OPERATOR_KINDS.map((kind) => (
											<option key={kind} value={kind}>
												{kind}
											</option>
										))}
									</select>
								</div>
								{node.operator.kind === "indicator" && (
									<div>
										<Label htmlFor={`indicator-${node.id}`}>
											{t("features.definitions.editor.indicatorId")}
										</Label>
										<Input
											id={`indicator-${node.id}`}
											className="mt-1"
											value={node.operator.id ?? ""}
											onChange={(event) =>
												onChange(
													updateNode(draft, index, {
														operator: {
															kind: "indicator",
															id: event.target.value,
														},
													}),
												)
											}
										/>
									</div>
								)}
								<div>
									<Label htmlFor={`warmup-${node.id}`}>
										{t("features.definitions.editor.warmup")}
									</Label>
									<Input
										id={`warmup-${node.id}`}
										className="mt-1"
										type="number"
										min={0}
										value={node.warmupBars}
										onChange={(event) =>
											onChange(
												updateNode(draft, index, {
													warmupBars: Math.max(
														0,
														Number.parseInt(event.target.value, 10) || 0,
													),
												}),
											)
										}
									/>
								</div>
							</div>

							<div className="space-y-2">
								<p className="text-sm font-medium">
									{t("features.definitions.editor.inputs")}
								</p>
								<ul className="space-y-2">
									{node.inputs.map((input, inputIndex) => (
										<li
											key={`${node.id}-input-${inputIndex}`}
											className="flex flex-wrap items-end gap-2"
										>
											<InputReference
												input={input}
												labelId={`${node.id}-input-${inputIndex}`}
												onChange={(next) =>
													onChange(
														updateNode(draft, index, {
															inputs: node.inputs.map((item, position) =>
																position === inputIndex ? next : item,
															),
														}),
													)
												}
											/>
											<Button
												type="button"
												variant="ghost"
												size="sm"
												aria-label={`${t("features.definitions.editor.removeInput")}: ${node.id} ${inputIndex + 1}`}
												onClick={() => onChange(removeInput(draft, index, inputIndex))}
											>
												{t("features.definitions.editor.removeInput")}
											</Button>
										</li>
									))}
								</ul>
								<Button
									type="button"
									variant="outline"
									size="sm"
									onClick={() => onChange(addMarketInput(draft, index, "close"))}
								>
									{t("features.definitions.editor.addInput")}
								</Button>
							</div>

							<div className="space-y-2">
								<p className="text-sm font-medium">
									{t("features.definitions.editor.parameters")}
								</p>
								<ul className="space-y-2">
									{Object.entries(node.parameters).map(
										([name, value], parameterIndex) => (
											<li
												key={`${node.id}-param-${name}`}
												className="flex flex-wrap items-end gap-2"
											>
												<div className="w-40">
													<Label htmlFor={`${node.id}-param-key-${parameterIndex}`}>
														{t("features.definitions.editor.parameterName")}
													</Label>
													<Input
														id={`${node.id}-param-key-${parameterIndex}`}
														className="mt-1"
														value={name}
														readOnly
													/>
												</div>
												<div className="w-56">
													<Label htmlFor={`${node.id}-param-value-${parameterIndex}`}>
														{t("features.definitions.editor.parameterValue")}
													</Label>
													<Input
														id={`${node.id}-param-value-${parameterIndex}`}
														className="mt-1 font-mono"
														defaultValue={JSON.stringify(value)}
														key={`${name}-${JSON.stringify(value)}`}
														onBlur={(event) => {
															const parameters = {
																...node.parameters,
																[name]: parseParameterValue(event.target.value),
															};
															onChange(updateNode(draft, index, { parameters }));
														}}
													/>
												</div>
												<Button
													type="button"
													variant="ghost"
													size="sm"
													aria-label={`${t("features.definitions.editor.removeParameter")}: ${name}`}
													onClick={() => {
														const parameters = { ...node.parameters };
														delete parameters[name];
														onChange(updateNode(draft, index, { parameters }));
													}}
												>
													✕
												</Button>
											</li>
										),
									)}
								</ul>
								<AddParameterRow
									nodeId={node.id}
									onAdd={(name, value) =>
										onChange(
											updateNode(draft, index, {
												parameters: {
													...node.parameters,
													[name]: parseParameterValue(value),
												},
											}),
										)
									}
								/>
							</div>
						</li>
					))}
				</ol>
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="mt-3"
					onClick={() => {
						const id = defaultNodeId(draft);
						onChange({
							...draft,
							nodes: [...draft.nodes, createEmptyNode(id, draft.scope)],
						});
					}}
				>
					{t("features.definitions.editor.addNode")}
				</Button>
			</section>

			<section aria-label={t("features.definitions.editor.outputs")}>
				<p className="mb-2 text-sm font-medium">
					{t("features.definitions.editor.outputs")}
				</p>
				<ul className="space-y-2">
					{draft.outputs.map((output, outputIndex) => (
						<li
							key={`${output.name}-${outputIndex}`}
							className="flex flex-wrap items-end gap-2"
						>
							<div className="w-48">
								<Label htmlFor={`output-name-${outputIndex}`}>
									{t("features.definitions.editor.outputName")}
								</Label>
								<Input
									id={`output-name-${outputIndex}`}
									className="mt-1"
									value={output.name}
									onChange={(event) =>
										onChange({
											...draft,
											outputs: draft.outputs.map((item, position) =>
												position === outputIndex
													? { ...item, name: event.target.value }
													: item,
											),
										})
									}
								/>
							</div>
							<div className="w-40">
								<Label htmlFor={`output-node-${outputIndex}`}>
									{t("features.definitions.editor.outputNode")}
								</Label>
								<select
									id={`output-node-${outputIndex}`}
									className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
									value={output.nodeId}
									onChange={(event) =>
										onChange({
											...draft,
											outputs: draft.outputs.map((item, position) =>
												position === outputIndex
													? { ...item, nodeId: event.target.value }
													: item,
											),
										})
									}
								>
									{draft.nodes.map((node) => (
										<option key={node.id} value={node.id}>
											{node.id}
										</option>
									))}
								</select>
							</div>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								aria-label={`${t("features.definitions.editor.removeOutput")}: ${output.name}`}
								onClick={() =>
									onChange({
										...draft,
										outputs: draft.outputs.filter(
											(_, position) => position !== outputIndex,
										),
									})
								}
							>
								{t("features.definitions.editor.removeOutput")}
							</Button>
						</li>
					))}
				</ul>
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="mt-2"
					onClick={() =>
						onChange({
							...draft,
							outputs: [
								...draft.outputs,
								{
									name: `output-${draft.outputs.length + 1}`,
									nodeId: draft.nodes[0]?.id ?? "",
								},
							],
						})
					}
				>
					{t("features.definitions.editor.addOutput")}
				</Button>
			</section>
		</div>
	);
}

function InputReference({
	input,
	labelId,
	onChange,
}: {
	input: FeatureInput;
	labelId: string;
	onChange: (input: FeatureInput) => void;
}) {
	const { t } = useTranslation();
	return (
		<>
			<div className="w-36">
				<Label htmlFor={`kind-${labelId}`}>
					{t("features.definitions.editor.inputKind")}
				</Label>
				<select
					id={`kind-${labelId}`}
					className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
					value={input.kind}
					onChange={(event) => {
						const kind = event.target.value;
						if (kind === "market") onChange({ kind: "market", field: "close" });
						else if (kind === "node") onChange({ kind: "node", nodeId: "" });
						else onChange({ kind: "artifact", artifactId: "" });
					}}
				>
					<option value="market">
						{t("features.definitions.editor.kindMarket")}
					</option>
					<option value="node">{t("features.definitions.editor.kindNode")}</option>
					<option value="artifact">
						{t("features.definitions.editor.kindArtifact")}
					</option>
				</select>
			</div>
			{input.kind === "market" && (
				<div className="w-40">
					<Label htmlFor={`field-${labelId}`}>
						{t("features.definitions.editor.marketField")}
					</Label>
					<select
						id={`field-${labelId}`}
						className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
						value={input.field}
						onChange={(event) =>
							onChange({
								kind: "market",
								field: event.target.value as MarketField,
							})
						}
					>
						{MARKET_FIELDS.map((field) => (
							<option key={field} value={field}>
								{field}
							</option>
						))}
					</select>
				</div>
			)}
			{input.kind === "node" && (
				<div className="w-40">
					<Label htmlFor={`node-ref-${labelId}`}>
						{t("features.definitions.editor.nodeRef")}
					</Label>
					<Input
						id={`node-ref-${labelId}`}
						className="mt-1"
						value={input.nodeId}
						onChange={(event) =>
							onChange({
								kind: "node",
								nodeId: event.target.value,
								definitionHash: input.definitionHash,
							})
						}
					/>
				</div>
			)}
			{input.kind === "artifact" && (
				<div className="w-56">
					<Label htmlFor={`artifact-ref-${labelId}`}>
						{t("features.definitions.editor.artifactRef")}
					</Label>
					<Input
						id={`artifact-ref-${labelId}`}
						className="mt-1 font-mono"
						value={input.artifactId}
						onChange={(event) =>
							onChange({ kind: "artifact", artifactId: event.target.value })
						}
					/>
				</div>
			)}
		</>
	);
}

function AddParameterRow({
	nodeId,
	onAdd,
}: {
	nodeId: string;
	onAdd: (name: string, value: string) => void;
}) {
	const { t } = useTranslation();
	const [name, setName] = useState("");
	const [value, setValue] = useState("");
	const keyId = `new-param-key-${nodeId}`;
	const valueId = `new-param-value-${nodeId}`;
	return (
		<div className="flex flex-wrap items-end gap-2">
			<div className="w-40">
				<Label htmlFor={keyId}>
					{t("features.definitions.editor.parameterName")}
				</Label>
				<Input
					id={keyId}
					className="mt-1"
					value={name}
					onChange={(event) => setName(event.target.value)}
				/>
			</div>
			<div className="w-56">
				<Label htmlFor={valueId}>
					{t("features.definitions.editor.parameterValue")}
				</Label>
				<Input
					id={valueId}
					className="mt-1 font-mono"
					value={value}
					onChange={(event) => setValue(event.target.value)}
				/>
			</div>
			<Button
				type="button"
				variant="outline"
				size="sm"
				disabled={!name.trim()}
				onClick={() => {
					onAdd(name.trim(), value);
					setName("");
					setValue("");
				}}
			>
				{t("features.definitions.editor.addParameter")}
			</Button>
		</div>
	);
}
