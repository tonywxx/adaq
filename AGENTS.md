# Repository Guidelines

## Computer Use

Prefer ChatGPT's built-in Computer Use (`mcp__cua_repl`) for desktop and browser UI interactions. If it is unavailable, use Orca Computer Use as the user-authorized fallback. If both are unavailable, report the limitation.

## Project Structure & Module Organization

ADAQ is a Tauri 2 desktop app with a React 19 + Vite frontend. Frontend source lives in `src/`: entry points are `src/main.tsx` and `src/App.tsx`, reusable UI is in `src/components/`, shadcn primitives are in `src/components/ui/`, hooks are in `src/hooks/`, layout code is in `src/layout/`, shared helpers are in `src/lib/`, global CSS is in `src/styles/`, and static assets belong in `src/assets/`. Native code and desktop configuration live in `src-tauri/`: Rust sources in `src-tauri/src/`, capabilities in `src-tauri/capabilities/`, icons in `src-tauri/icons/`, app config in `src-tauri/tauri.conf.json`, and external component contracts in `src-tauri/wit/<project-name>.wit`. Never reference machine-local WIT paths. Release automation lives in `.github/workflows/release.yml`.

## Build, Test, and Development Commands

- `pnpm install --frozen-lockfile`: install dependencies from `pnpm-lock.yaml`.
- `pnpm dev`: start the Vite frontend dev server.
- `pnpm tauri dev`: run the desktop app with the Tauri shell.
- `pnpm run build`: run strict TypeScript checking, then build the frontend.
- `pnpm exec vite build`: build only the frontend without TypeScript checking.
- `pnpm exec jest --watchman=false --runInBand`: run frontend tests; append relevant test paths for a focused check.
- `pnpm run lint`: run the configured Biome rules.
- `cd src-tauri && cargo check`: verify Rust/Tauri code.
- `cd src-tauri && cargo test -p <crate>`: run the relevant Rust crate's tests; CI runs the workspace suite.
- Release: pushing a `vX.Y.Z` tag triggers `.github/workflows/release.yml`, which publishes macOS ARM64 and Windows x86_64 updater assets after validation and build checks. Synchronize the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` before tagging.

## Coding Style & Naming Conventions

Use TypeScript strict mode and the `@/*` alias for imports from `src/`. Follow existing React patterns: PascalCase component exports, kebab-case component files such as `app-titlebar.tsx`, and `use-*` hook names. Keep shadcn/Radix primitives in `src/components/ui/` and composed app components one level higher. Match existing formatting, organize imports per `biome.json`, and use standard `rustfmt`.

## Testing Guidelines

Frontend tests use Jest (`jest.config.js`); Rust tests live in crate modules and integration suites. For behavior changes, run the relevant existing tests and update coverage as needed. Use the commands above for frontend build/lint and native compilation checks. Colocate frontend tests as `*.test.ts` or `*.test.tsx`; keep Rust unit tests inside the relevant module. The CI check sequence and trigger paths are defined in `.github/workflows/v1-acceptance.yml`.

## Commit & Pull Request Guidelines

Git history uses Conventional Commits (`feat:`, `fix:`, `chore:`) plus release commits such as `chore(release): 0.8.0`. Pull requests should describe the change, list verification commands, link issues, and include screenshots or recordings for UI/titlebar/sidebar changes. Note skipped checks with the reason.

## Security & Configuration Tips

Do not commit signing keys, release tokens, or local environment files. Tauri updater releases require valid GitHub release assets plus `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the release environment. Keep updater capability changes scoped in `src-tauri/capabilities/default.json`.

## Agent skills

### Issue tracker

Issues and PRDs are tracked in GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the five canonical triage labels without aliases. See `docs/agents/triage-labels.md`.

### Domain docs

Use the single-context layout with root `CONTEXT.md` and `docs/adr/`. See `docs/agents/domain.md`.

## Code Navigation and Tool Discovery

Choose the entry point that matches the question; switch tools when the first result lacks the needed evidence.

- Symbols, signatures, callers/callees, impact, and execution traces: use CodeGraph.
- Broad architecture, domain concepts, and semantic relationships across files: use graphify when `graphify-out/graph.json` exists. Use `graphify query`, `graphify path`, or `graphify explain` for scoped results; use `graphify-out/wiki/index.md` for broad navigation when available. Read `GRAPH_REPORT.md` when these results are insufficient.
- Literal text, known files, and configuration: use `rg` and direct reads.
- For `/graphify` commands or selected graphify queries, read the installed graphify skill. Dirty graph outputs are expected after incremental updates and remain usable unless the task concerns incorrect graph output.
- Discover tools by exact name or one narrow capability. Return at most 10 matching names with short descriptions; read full tool metadata only for the selected tool.
