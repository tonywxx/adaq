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

### Issue tracker

Issues and PRDs are tracked in GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the five canonical triage labels without aliases. See `docs/agents/triage-labels.md`.

### Domain docs

Use the single-context layout with root `CONTEXT.md` and `docs/adr/`. See `docs/agents/domain.md`.
