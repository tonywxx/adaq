# Prioritize Help-first startup and defer market session loading

Status: accepted

The authenticated Desktop Client Surface keeps the Help home (Workflow Guide) as the smallest useful startup path: only authentication and the minimal auth i18n load up front. The authenticated workspace runtime, full i18n resources, non-Help routes, market/chart dependencies, and native Research/Watchlist state initialize after authentication and on navigation. The market-session Provider and store load at the first route that consumes market-session state, preserving the existing realtime subscription guard and using the existing page loading skeleton. Startup prefetch and telemetry remain outside this slice.

## Consequences

- Help no longer pays the market-session hydration cost during startup.
- Help no longer pays the native SQLite/Research state initialization cost during startup.
- Full business translations and workspace runtime load after auth instead of blocking the auth surface.
- The first market-dependent route pays the Provider load and hydration cost.
- The first process-to-Help and WebView-to-Help timings are measured locally without persistence or network telemetry.
