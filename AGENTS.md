# Repository Guidelines

## Project Structure & Module Organization

ADAQ is a Tauri 2 desktop app with a React 19 + Vite frontend. Frontend source lives in `src/`: entry points are `src/main.tsx` and `src/App.tsx`, reusable UI is in `src/components/`, shadcn primitives are in `src/components/ui/`, hooks are in `src/hooks/`, layout code is in `src/layout/`, shared helpers are in `src/lib/`, global CSS is in `src/styles/`, and static assets belong in `src/assets/`. Native code and desktop configuration live in `src-tauri/`: Rust sources in `src-tauri/src/`, capabilities in `src-tauri/capabilities/`, icons in `src-tauri/icons/`, app config in `src-tauri/tauri.conf.json`, and external component contracts in `src-tauri/wit/<project-name>.wit`. Never reference machine-local WIT paths. Manual release automation lives in `.github/workflows/release.yml`.

## Build, Test, and Development Commands

- `pnpm install --frozen-lockfile`: install dependencies from `pnpm-lock.yaml`.
- `pnpm dev`: start the Vite frontend dev server.
- `pnpm tauri dev`: run the desktop app with the Tauri shell.
- `pnpm run build`: run strict TypeScript checking, then build the frontend.
- `pnpm exec vite build`: build only the frontend, matching the release workflow.
- `cd src-tauri && cargo check`: verify Rust/Tauri code.
- GitHub Actions `Release`: manually publish signed macOS ARM64, Windows x86_64, and Linux x86_64 updater assets from `main` after synchronizing the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`.

## Coding Style & Naming Conventions

Use TypeScript strict mode and the `@/*` alias for imports from `src/`. Follow existing React patterns: PascalCase component exports, kebab-case component files such as `app-titlebar.tsx`, and `use-*` hook names. Keep shadcn/Radix primitives in `src/components/ui/` and composed app components one level higher. Match existing formatting, organize imports per `biome.json`, and use standard `rustfmt`.

## Testing Guidelines

No dedicated frontend or Rust test suite is currently present. For behavior changes, run the narrowest meaningful checks: `pnpm run build` for frontend/type changes and `cargo check` from `src-tauri/` for native changes. If adding tests, colocate frontend tests as `*.test.ts` or `*.test.tsx`; keep Rust unit tests inside the relevant module.

## Commit & Pull Request Guidelines

Git history uses Conventional Commits (`feat:`, `fix:`, `chore:`) plus release commits such as `chore(release): 0.8.0`. Pull requests should describe the change, list verification commands, link issues, and include screenshots or recordings for UI/titlebar/sidebar changes. Note skipped checks with the reason.

## Security & Configuration Tips

Do not commit signing keys, release tokens, or local environment files. Tauri updater releases require valid GitHub release assets plus `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the release environment. Keep updater capability changes scoped in `src-tauri/capabilities/default.json`.

## Agent skills

### Development workflow

ADAQ exclusively uses the Matt skills workflow documented by [ADR 0091](docs/adr/0091-use-one-matt-skills-workflow-and-user-gated-delivery-order.md): `/grill-with-docs` → `/to-spec` → `/to-tickets` → one fresh `/implement <issue>` session per ticket. Do not use `planning-with-files`, create repository-root `task_plan.md`, `findings.md`, or `progress.md`, or substitute another agent planning workflow.

All agents must preserve the strict product order: Market Data Acquisition → Data Validation/Canonicalization/Quality/Persistence → Feature Engineering → Factor Steps 1–3 → Model Steps 4–6 → Strategy Steps 7–8 → Paper Operations Steps 9–10. Complete current-head verification for one module, stop for User verification, and wait for explicit User approval before starting the next module. Issue closure, reusable cores, fixtures, green CI, and issue frontiers do not waive this gate.

### Issue tracker

Issues and PRDs are tracked in GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the five canonical triage labels without aliases. See `docs/agents/triage-labels.md`.

### Domain docs

Use the single-context layout with root `CONTEXT.md` and `docs/adr/`. See `docs/agents/domain.md`.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
