# GUI Localization Guide

[简体中文](./gui-localization.zh-CN.md)

Status: V1 user, authoring, and acceptance contract.

## V1 language contract

V1 bundles exactly two translation resources:

| Resource locale | User-facing name | Purpose |
| --- | --- | --- |
| `en-US` | English (US) | Default and fallback GUI copy. |
| `zh-CN` | 简体中文 | Complete Simplified Chinese GUI copy. |

Settings > General exposes three selections, but System is a resolver rather than a third language:

| Selection | Resolution |
| --- | --- |
| System | A Chinese system language resolves to `zh-CN`; every other system language resolves to `en-US`. |
| English (US) | Always uses `en-US`. |
| 简体中文 | Always uses `zh-CN`. |

Unsupported system languages never trigger runtime translation or a downloaded language pack. They use the complete `en-US` resources.

## Setting behavior

- System is the default on a new device.
- The selection is stored as a device-local Interface Locale preference rather than User Profile data.
- Changing it applies immediately without restarting or signing in again.
- The application resolves and initializes the locale before its first visible paint so it does not flash copy from another language.
- The choice survives sign-out and user-scoped research-data resets.
- The active resource locale updates the document `lang` attribute for assistive technology.

## What is localized

Every user-facing GUI surface must use translation resources, including:

- Navigation, page headings, tabs, cards, tables, empty states, and Dashboard labels.
- Buttons, menus, forms, placeholders, validation messages, and confirmation dialogs.
- Loading, progress, success, warning, error, connection, reconciliation, and Bot-state summaries.
- Tooltips, accessible names, image alternatives, keyboard instructions, and screen-reader-only text.
- User-facing explanations of Factors, Models, Strategies, metrics, Risk Decisions, and execution evidence.

Interpolation, plurals, dates, quantities, and values belong in complete translated sentences. Code must not assemble a sentence by concatenating translated fragments whose grammar depends on English word order.

## What localization must not change

The Interface Locale is a presentation setting. It never changes the identity or stored meaning of:

- Instrument IDs, Venue codes, tickers, Component IDs, versions, hashes, enum wire values, or schema fields.
- Exact Decimal prices, quantities, balances, rates, metrics, timestamps, Trading Dates, or Venue Time Zones.
- Market Data Snapshots, research protocols, model artifacts, evidence payloads, provider responses, logs, or exports.
- User-authored names, notes, source code, model labels, or imported Component metadata.

A translated label may appear beside a canonical value. Provider errors and technical diagnostics retain their original detail; ADAQ may add a translated category and recovery explanation without rewriting the raw evidence.

## Formatting rules

Dates, numbers, percentages, and currencies use the platform `Intl` APIs with the active resolved resource locale. Formatting never changes the underlying exact value:

- `en-US` and `zh-CN` may display the same instant or Decimal differently.
- Venue-local market time remains governed by its Trading Calendar and IANA Venue Time Zone, not by the Interface Locale.
- Currency formatting always retains the actual currency code or an unambiguous symbol.
- A formatted display string is never reused as a canonical identifier, hash input, serialized value, or calculation input.

## Translation-resource rules

- `i18next` owns locale resolution, fallback, interpolation, and resource lookup; React surfaces use `react-i18next`.
- Resources are bundled with the desktop application and are available offline.
- Translation keys are semantic and stable, such as `settings.general.language.title`, rather than English sentences used as keys.
- `en-US` is the fallback locale. A missing key must show valid English copy, never an empty label.
- The English and Chinese resources must contain the same keys and interpolation variables.
- Domain enums remain canonical in storage and are mapped to translated display labels at the GUI boundary.
- Raw technical evidence remains available even when a translated summary is shown first.

## Acceptance checks

Before V1 is accepted:

1. Automated checks compare `en-US` and `zh-CN` keys and interpolation variables.
2. The application is launched directly into each explicit language and System mode without first-paint language flashing.
3. Every shipped route is exercised in both locales, including loading, empty, degraded, failure, and confirmation states.
4. Language selection is changed at runtime and verified after restart and sign-out.
5. Tables, charts, dialogs, title bars, narrow layouts, and long Chinese or English strings are checked for clipping and inaccessible overflow.
6. Keyboard and screen-reader labels are verified in both locales.
7. Canonical exports produced under both locales are equivalent except for explicitly localized presentation documents.

## Adding another language later

An additional language is post-V1 work. It requires a complete resource set, key and interpolation parity, locale-aware formatting review, route-level layout and accessibility acceptance, translated user documentation, and an explicit System-resolution rule. It does not require changing ADAQ's domain records or evidence formats.
