# Repository Guidelines

## Project Structure & Module Organization

ADAQ is a Tauri 2 desktop app with a React 19 + Vite frontend. Frontend source lives in `src/`: entry points are `src/main.tsx` and `src/App.tsx`, reusable UI is in `src/components/`, shadcn primitives are in `src/components/ui/`, hooks are in `src/hooks/`, layout code is in `src/layout/`, shared helpers are in `src/lib/`, global CSS is in `src/styles/`, and static assets belong in `src/assets/`. Native code and desktop configuration live in `src-tauri/`: Rust sources in `src-tauri/src/`, capabilities in `src-tauri/capabilities/`, icons in `src-tauri/icons/`, and app config in `src-tauri/tauri.conf.json`. Release automation uses `.releaserc.json`, `.github/workflows/release.yml`, and `scripts/semantic-release-prepare.mjs`.

## Build, Test, and Development Commands

- `pnpm install --frozen-lockfile`: install dependencies from `pnpm-lock.yaml`.
- `pnpm dev`: start the Vite frontend dev server.
- `pnpm tauri dev`: run the desktop app with the Tauri shell.
- `pnpm run build`: run strict TypeScript checking, then build the frontend.
- `pnpm exec vite build`: build only the frontend, matching the release workflow.
- `cd src-tauri && cargo check`: verify Rust/Tauri code.
- `pnpm release`: run semantic-release on `main`.
- `pnpm run release:github -- <version>`: publish signed updater assets with the local helper.

## Coding Style & Naming Conventions

Use TypeScript strict mode and the `@/*` alias for imports from `src/`. Follow existing React patterns: PascalCase component exports, kebab-case component files such as `app-titlebar.tsx`, and `use-*` hook names. Keep shadcn/Radix primitives in `src/components/ui/` and composed app components one level higher. Match existing formatting, organize imports per `biome.json`, and use standard `rustfmt`.

## Testing Guidelines

No dedicated frontend or Rust test suite is currently present. For behavior changes, run the narrowest meaningful checks: `pnpm run build` for frontend/type changes and `cargo check` from `src-tauri/` for native changes. If adding tests, colocate frontend tests as `*.test.ts` or `*.test.tsx`; keep Rust unit tests inside the relevant module.

## Commit & Pull Request Guidelines

Git history uses Conventional Commits (`feat:`, `fix:`, `chore:`) plus release commits such as `chore(release): 0.3.0 [skip ci]`. Prefer Conventional Commits because semantic-release analyzes commits on `main`. Pull requests should describe the change, list verification commands, link issues, and include screenshots or recordings for UI/titlebar/sidebar changes. Note skipped checks with the reason.

## Security & Configuration Tips

Do not commit signing keys, release tokens, or local environment files. Tauri updater releases require valid GitHub release assets plus `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the release environment. Keep updater capability changes scoped in `src-tauri/capabilities/default.json`.
