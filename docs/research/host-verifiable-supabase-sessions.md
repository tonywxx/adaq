# Host-verifiable Supabase sessions for the embedded Desktop client

**Captured:** 2026-08-20

**Scope:** decision research only; no product code or Supabase configuration was changed.

**Evidence convention:** **Verified** means directly supported by current ADAQ code or a first-party source. **Inference** is a security or architecture conclusion drawn from those facts. **Recommendation** is the proposed V1 choice.

## Decision

**Recommendation:** use a Host-bound hybrid session:

1. Keep sign-in and refresh ownership in the React Supabase client for V1.
2. On initial session, sign-in, refresh, and sign-out, send only the current access token (or clear signal) to one narrow Tauri auth command.
3. The Rust Host accepts an identity only after verifying the token itself. With an asymmetric Supabase signing key, verify locally against the configured project's JWKS and derive the User ID exclusively from the verified `sub` claim. Never accept a request `userId` as authority.
4. Store the resulting `AuthenticatedUserContext` in process-managed Host state, bound to the invoking Webview/window label. Every User-scoped command obtains its User from that state before reaching a domain service.
5. Use fresh online validation for strict operations. If immediate session revocation must be guaranteed, also check that the JWT's `session_id` still exists through a narrowly authorized server-side capability; neither local JWT verification nor `getUser` alone provides immediate sign-out revocation.
6. Permit offline use only for explicitly low-risk local research reads/computation, with an unexpired token and a previously fetched key for the pinned issuer. Cold-cache, unknown-key, expired-token, user-changing, credential, destructive, bot/order, and other strict operations fail closed offline.

This is the smallest V1 design that removes client-selected identity without making Supabase Auth or the network the hot path for every local operation. It is conditional on the hosted project using an asymmetric signing key. If the project still uses HS256, use the Auth `/user` endpoint as the temporary online-only verifier and schedule migration to ES256 before enabling offline authenticated operation.

## Current ADAQ boundary

- **Verified:** ADAQ creates `@supabase/supabase-js` with a publishable key, falling back to the legacy anon key, and supplies no explicit auth storage or refresh options ([`src/lib/supabase.ts`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src/lib/supabase.ts#L1-L12)). The locked SDK is 2.112.2 ([`pnpm-lock.yaml`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/pnpm-lock.yaml#L32-L34)); 2.112.3 was the npm latest on the capture date ([npm registry](https://registry.npmjs.org/@supabase%2Fsupabase-js/latest)).
- **Verified:** `AuthGate` restores `getSession()`, subscribes to `onAuthStateChange`, and passes `session.user.id` into the frontend `MarketSessionProvider` ([`auth-gate.tsx`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src/components/auth-gate.tsx#L29-L52), [`auth-gate.tsx`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src/components/auth-gate.tsx#L89-L100)). Supabase warns that `getSession()` reads attached storage and its values must not be treated as authentic at an authorization boundary ([Supabase `getSession`](https://supabase.com/docs/reference/javascript/auth-getsession)).
- **Verified:** Rust's shared User check only rejects empty or over-128-byte values; it does not authenticate them ([`user.rs`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/src/user.rs#L1-L12)). User IDs are consequently selectors supplied by the Webview, not Host-derived identity.
- **Verified:** the app currently has one configured window and a process-wide Tauri state setup ([`tauri.conf.json`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/tauri.conf.json#L13-L30), [`lib.rs`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/src/lib.rs#L2468-L2502)). Tauri commands can receive managed `State`, the invoking `WebviewWindow`, or the raw IPC request and headers ([Tauri commands](https://v2.tauri.app/develop/calling-rust/#accessing-managed-state), [window injection](https://v2.tauri.app/develop/calling-rust/#accessing-the-webviewwindow-in-commands), [raw request](https://v2.tauri.app/develop/calling-rust/#accessing-raw-request)).
- **Verified:** ADAQ already has an OS secret-store seam backed by keyring 3.6.3, mapping to macOS Keychain, Windows Credential Manager, and Linux Secret Service ([`secret_store.rs`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/src/connections/secret_store.rs#L1-L10), [`Cargo.lock`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/Cargo.lock#L3193-L3206)). Released V1 targets are macOS ARM64 and Windows x86_64; Linux is deferred ([README](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/README.md#L87-L94)).

## Supabase facts that constrain the choice

### Token verification and project signing configuration

- **Verified:** Supabase access tokens carry signed claims including `iss`, `exp`, `sub`, and `role`. Supabase directs non-SDK implementations to use a high-quality JWT library rather than implement cryptography themselves ([Supabase JWTs](https://supabase.com/docs/guides/auth/jwts)).
- **Verified:** an asymmetric project exposes trusted public keys at `https://<project>.supabase.co/auth/v1/.well-known/jwks.json`. The endpoint returns no keys for a project that is not using asymmetric signing keys ([Supabase JWT verification](https://supabase.com/docs/guides/auth/jwts)).
- **Verified:** supported production choices are ES256 and RS256; Supabase recommends ES256. EdDSA is documented as coming soon, while HS256 is not recommended for production ([Supabase signing algorithms](https://supabase.com/docs/guides/auth/signing-keys#public-key-discovery-and-caching)).
- **Verified:** Supabase documents 10-minute Edge caching of JWKS and potentially another 10 minutes in client memory. It says not to cache longer, recommends allowing at least 20 minutes around signing-key changes, and warns that custom caches can continue trusting a revoked key until purged ([Supabase key discovery and caching](https://supabase.com/docs/guides/auth/signing-keys#public-key-discovery-and-caching), [Supabase JWT verification](https://supabase.com/docs/guides/auth/jwts)).
- **Inference:** the Host verifier must pin the configured project issuer and an explicit algorithm allowlist, select by `kid`, verify signature and `exp` (and `nbf` when present), validate `sub` and `session_id` as UUIDs, require the expected authenticated role and validate the expected audience when that claim is present, and reject tokens from another Supabase project even if cryptographically valid. These are standard validations implied by the documented token header/claims, not a Supabase-provided ADAQ recipe.
- **Verified unknown:** this checkout contains no deploy-time Supabase URL or dashboard state, so the hosted project's current signing algorithm, JWT lifetime, session timeout, and single-session settings could not be verified. This is a decision precondition, not a reason to trust HS256 locally.

### Lifetime, refresh, sign-out, deletion, and revocation

- **Verified:** a Supabase session is a short-lived access JWT plus a single-use refresh token. Access tokens are normally 5 minutes to 1 hour; refresh tokens do not expire but rotate. Supabase recommends the default 1-hour access lifetime for most apps and discourages less than 5 minutes because of refresh load, clock skew, proactive refresh timing, and long requests ([Supabase sessions](https://supabase.com/docs/guides/auth/sessions)).
- **Verified:** refresh-token reuse is normally detected, with a default 10-second reuse interval and a parent-token recovery exception for unreliable clients. A reuse outside those exceptions terminates the session ([Supabase sessions](https://supabase.com/docs/guides/auth/sessions#what-is-refresh-token-reuse-detection-and-what-does-it-protect-from)). `refreshSession()` returns a new session and errors for an invalid refresh token ([Supabase `refreshSession`](https://supabase.com/docs/reference/javascript/auth-refreshsession)).
- **Verified:** session time-box, inactivity, and single-session policies are enforced when the session next refreshes, so their observed effect can lag by the access-token lifetime ([Supabase sessions](https://supabase.com/docs/guides/auth/sessions)).
- **Verified:** sign-out removes the selected sessions/refresh tokens, but an already-issued access JWT cannot be revoked and remains usable until `exp`; the default JS sign-out scope is global, while `local` and `others` scopes also exist ([Supabase `signOut`](https://supabase.com/docs/reference/javascript/auth-signout)).
- **Verified:** deleting an Auth User removes its sessions and invalidates refresh tokens, but an already-issued stateless access JWT remains valid until `exp`; Supabase gives the same bounded-expiry or sensitive-operation `session_id`-check choices ([Supabase removing account access](https://supabase.com/docs/guides/auth/managing-user-data#removing-account-access)).
- **Verified:** for a stronger post-sign-out guarantee, Supabase says to verify that the JWT's `session_id` still maps to a row in `auth.sessions`; absence means the session was logged out. Supabase recommends reserving this database check for sensitive actions ([Supabase strict session validation](https://supabase.com/docs/guides/auth/sessions#how-to-ensure-an-access-token-jwt-cannot-be-used-after-a-user-signs-out)).
- **Inference:** `getUser(jwt)` is authoritative for the returned user at request time and detects invalid tokens or a missing user record, but it must not be advertised as immediate sign-out revocation. Supabase separately states that access JWTs survive sign-out until expiry and prescribes a `session_id` database lookup for the stronger guarantee ([Supabase `getUser`](https://supabase.com/docs/reference/javascript/auth-getuser), [Supabase strict session validation](https://supabase.com/docs/guides/auth/sessions#how-to-ensure-an-access-token-jwt-cannot-be-used-after-a-user-signs-out)).

### Keys and local persistence

- **Verified:** publishable keys are intended to be embedded in public desktop apps and identify the application, not the User. Secret and `service_role` keys are elevated, bypass RLS, and must not be packaged in the app ([Supabase API keys](https://supabase.com/docs/guides/getting-started/api-keys)).
- **Verified:** Supabase JS defaults `persistSession` to true and attempts local-storage persistence; it supports a custom sync or async storage object with `getItem`, `setItem`, and `removeItem` ([Supabase Auth overview](https://supabase.com/docs/reference/javascript/auth), [Supabase sessions storage](https://supabase.com/docs/guides/auth/sessions#using-http-only-cookies-to-store-access-and-refresh-tokens)).
- **Inference:** because ADAQ supplies no custom options, the current Webview uses the SDK browser defaults. A compromised Webview or local-storage reader can steal the refresh token; Host-side access-token verification prevents changing `userId`, but cannot make a stolen same-User session harmless.
- **Recommendation:** do not move refresh ownership during the first identity-boundary repair. If persistence is hardened later, give ownership to exactly one layer and adapt the SDK storage to a narrow Host command backed by ADAQ's existing OS secret store. Never let both React and Rust rotate the same refresh token.

## Option comparison

| Option | What it proves | Availability and latency | Revocation behavior | V1 fit |
| --- | --- | --- | --- | --- |
| Local JWKS verification | Signature, pinned issuer/project, accepted algorithm, token time bounds, and claims | No per-command network call after key acquisition; can work offline with an unexpired token and cached matching key | Does not see sign-out, deletion, or session-policy changes until token expiry; key revocation can lag JWKS caches | Best normal path if the project is asymmetric |
| `GET /auth/v1/user` / `getUser(jwt)` | Auth-server-validated token and current returned User | Requires network for every check; Auth is in the hot path and region latency/outages affect local work | Does not override the documented access-token-until-expiry sign-out/deletion limit | Safe temporary path for HS256; too fragile for every command |
| Hybrid | Local proof for ordinary operations plus fresh online/session proof where policy demands it | Fast normal path; strict operations explicitly unavailable offline | Bounded ordinary-operation window; immediate strict revocation only when `session_id` is freshly checked | Recommended |

For HS256, Supabase explicitly recommends `GET /auth/v1/user` with the publishable `apikey` and bearer JWT rather than shipping the shared signing secret ([Supabase shared-secret verification](https://supabase.com/docs/guides/auth/jwts#verifying-with-a-shared-secret-signing-key)). For asymmetric projects, Supabase's JS `getClaims()` performs JWKS verification and falls back to the Auth server for symmetric keys, but invoking it in React would still leave the Rust Host trusting the Webview's answer ([Supabase `getClaims`](https://supabase.com/docs/reference/javascript/auth-getclaims)).

## Proposed V1 Host contract and enforcement seam

### One session owner, one Host binding

1. `AuthGate` remains responsible for Supabase sign-in and refresh.
2. A single `auth_session_bind(accessToken)` command sends the token into Rust after `INITIAL_SESSION`/sign-in and every `TOKEN_REFRESHED`; `SIGNED_OUT` calls `auth_session_clear`.
3. Rust verifies the token and stores only a bounded context: verified User UUID (`sub`), `session_id`, issuer, `exp`, accepted algorithm/`kid`, last online validation time, and invoking window label. It must not log or persist the access token.
4. V1 permits one bound User for the process/main window. Binding a different User requires explicit clear/sign-out first; a second window must either share the same process User intentionally or get a separate label-bound context. Never silently switch existing Attempts, stores, or paths to another User.
5. User-scoped command request DTOs stop carrying authoritative `userId`. During incremental migration, any retained `userId` is ignored or compared against Host context and rejected on mismatch; domain services receive only the Host-derived User ID.

**Verified:** Tauri managed State is process-managed and injectable into commands, and the invoking WebviewWindow label is also injectable ([Tauri state](https://v2.tauri.app/develop/state-management/), [Tauri commands](https://v2.tauri.app/develop/calling-rust/)). **Inference:** a small `AuthenticatedUserContext` state plus one shared guard is therefore the narrowest enforcement seam; passing bearer headers on every ordinary IPC call would unnecessarily spread token handling.

### Strict-operation policy

The exact capability list belongs to the follow-up decision ticket, but the safe default is:

| Operation class | Offline with unexpired locally verified token | Fresh online check |
| --- | --- | --- |
| Read existing local evidence; inspect immutable reports | Allow | Not required |
| Start deterministic research computation that cannot access secrets or place orders | Allow only if policy explicitly classifies it low risk | Not required by auth alone |
| Publish/promote authoritative evidence; import/export sensitive material | Deny by default | Require |
| Credential create/rotate/delete; account/security changes; local data reset/delete | Deny | Require and re-confirm User intent |
| Start/reconfigure Paper Bot, submit/cancel order intent, Risk/OMS/reconciliation action | Deny | Require; future real-money execution needs a stronger dedicated policy |

For a fresh online check, call `/auth/v1/user` with the packaged publishable key and bearer access token. If immediate revocation is required, additionally use a narrowly authorized server/RPC check of the current token's `session_id`; do not package a secret key merely to query `auth.sessions`.

### JWKS cache and failure rules

- Fetch only from the JWKS URL derived from the pinned Supabase project URL; never follow an issuer or JWKS URL supplied by the token.
- Keep a bounded key set keyed by `kid`, with fetch time and issuer. Refresh at or before the documented 10-minute edge window, on an unknown `kid`, and after online recovery. Provide an explicit purge for signing-key incident response.
- A fresh unknown `kid` causes one bounded JWKS refresh, then rejection. Do not try arbitrary algorithms or other issuers.
- A stale persisted JWKS may be used offline only for an already known key and a still-unexpired token, and only for low-risk operations. This deliberately trades immediate key revocation for bounded offline availability; strict operations fail closed.
- Expired token, invalid signature/claims, excessive clock skew, missing User/session binding, mismatched window, network timeout during a required check, key parse error, or (if protected persistence is later enabled) unavailable OS secret store all produce stable non-secret error codes and no domain call.
- Access tokens, refresh tokens, authorization headers, JWT payloads, and raw Auth errors must be redacted from logs. Correlate with a generated request ID, verified User ID only where policy allows, and a coarse failure code.

## Security limits and automation resistance

- **Verified:** Tauri capabilities constrain which APIs windows/webviews can call, but Tauri says they do not protect against incorrect command scope checks or malicious Rust code. Registered app commands are allowed to all app windows/webviews by default unless restricted ([Tauri capabilities](https://v2.tauri.app/security/capabilities/#security-boundaries)).
- **Verified:** ADAQ's current capability applies to `main`, includes broad filesystem permissions, and the CSP is `null` ([`default.json`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/capabilities/default.json#L1-L27), [`tauri.conf.json`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/tauri.conf.json#L36-L38)). Those are adjacent hardening gaps, not solved by session verification.
- **Inference:** Host verification prevents a compromised/automated Webview from selecting another User, replaying an expired token, or binding a token from another project. It cannot stop the Webview from exercising whatever capabilities the currently authenticated User legitimately has. Destructive confirmation, Surface Capability Profile, Risk/OMS gates, per-command authorization, rate limits, and auditable intent remain necessary.
- **Recommendation:** expose bind/clear/current-auth-status only to the main bundled window, keep auth tokens out of events/channels and frontend logs, and make all automation surfaces establish their own independently verified Host context rather than inherit the Desktop Webview's context.

## Version and platform snapshot

Captured from the checkout and official registries on 2026-08-20:

- ADAQ: `@supabase/supabase-js` 2.112.2; npm latest 2.112.3 ([lockfile](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/pnpm-lock.yaml#L32-L34), [npm](https://registry.npmjs.org/@supabase%2Fsupabase-js/latest)).
- ADAQ: Tauri Rust 2.11.5, reqwest 0.13.4, keyring 3.6.3 ([Tauri lock](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/Cargo.lock#L5734-L5738), [`Cargo.toml`](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/Cargo.toml#L62-L83), [keyring lock](https://github.com/tonywxx/adaq/blob/ce43b1c20bd1b9603707c1d4893adb1a67c3c343/src-tauri/Cargo.lock#L3193-L3206)). Existing reqwest and keyring seams cover online validation and future protected persistence.
- A quality Rust JWT verifier is still required for the asymmetric local path. `jsonwebtoken` 11.0.0 was current, declares Rust 1.88, has JWK support, and offers RustCrypto or AWS-LC crypto features ([crates.io](https://crates.io/api/v1/crates/jsonwebtoken), [docs.rs](https://docs.rs/jsonwebtoken/latest/jsonwebtoken/)). Dependency selection should be validated in the implementation ticket on both release targets; this report does not add it.
- The design is viable on current released macOS ARM64 and Windows x86_64. Linux code paths exist in keyring but are not a V1 release commitment.

## Acceptance and security tests for implementation

The follow-up implementation is not complete without the following runnable checks.

### Token and key verification

1. Accept a valid token from the pinned issuer for the configured User and derive exactly its `sub`; no caller-provided User ID reaches the domain call.
2. Reject modified payload/signature, expired token, future `nbf`, wrong issuer/project, wrong role/audience, missing or malformed `sub`, malformed `session_id`, disallowed `alg`, `alg=none`, and algorithm/key-type confusion.
3. Reject an otherwise valid token from a second Supabase project.
4. Select the correct key by `kid`; on unknown `kid`, refresh once and accept only if the refreshed pinned JWKS supplies an allowed matching key.
5. Exercise rotation with old and new keys, then revocation/cache purge; prove no cache lives beyond policy and strict operations do not use stale keys.
6. Simulate clock skew at the documented tolerance boundary without accepting an expired token.

### Host/IPC identity enforcement

7. Without a Host binding, every representative User-scoped command fails before touching SQLite, Parquet, filesystem, Provider, Credential, Python, or WASM state.
8. Bind User A, submit a request containing User B's legacy `userId`, and prove it is rejected (or ignored while the domain receives A). Cover at least one read, write, delete, Attempt start/cancel, Provider/Credential, and stream command.
9. Bind from an unapproved window/label and reject it. Bind User B while A is active and reject until explicit clear; clearing A must not relabel A's persisted Attempts or evidence.
10. Sign out/clear while commands are in flight: new commands fail immediately; already-created Attempts retain their original User ownership; no completion publishes under another User.
11. Prove access/refresh tokens and raw Auth errors do not appear in application logs, panic messages, SQLite, Parquet metadata, events, or serialized error responses.

### Online, revocation, and offline behavior

12. `/user` success binds only the returned verified User. 401/403, timeout, TLS/DNS failure, malformed response, missing User, and rate limit fail closed with stable redacted codes.
13. Sign out locally and globally, revoke the session, delete the User, and expire the token. Verify the documented distinction: ordinary local verification may remain valid only until `exp`, while every strict operation fails once the fresh session check fails.
14. Cold-start offline with no JWKS: reject. Offline with unknown `kid`: reject. Offline with cached key plus unexpired token: permit only the explicit low-risk matrix. Offline with expired token or stale-key strict operation: reject.
15. Restore connectivity after offline use: refresh JWKS and session authority before enabling strict operations; a revoked key/session must not regain authority from stale cache.
16. Refresh races: process duplicate refresh notifications and the Supabase 10-second reuse scenario without changing User or accepting an older token over a newer one. Reject a refreshed token whose `sub` changes until explicit sign-out/rebind.

### Persistence and platforms

17. If the later OS-store adapter is adopted, round-trip, overwrite, sign-out deletion, unavailable/locked keychain, and crash recovery on macOS and Windows; never fall back to plaintext or an in-memory store while claiming persistence.
18. Build and run focused auth tests on macOS ARM64 and Windows x86_64. Verify the selected JWT crypto backend on both; Linux is informational until release support returns.

## Preconditions for the follow-up decision

Before implementation, record these live project facts:

1. Current signing key type and algorithm (ES256, RS256, or HS256), current/standby/previous key state, and whether the JWKS endpoint returns the expected public keys.
2. Access-token expiry, time-box, inactivity, and single-session settings.
3. Whether V1 requires immediate post-sign-out/admin-deletion revocation for ordinary local research, or accepts a window bounded by JWT expiry.
4. Which V1 commands are strict operations, and whether a narrow `session_id` validation capability will be provisioned.
5. Whether the first repair intentionally leaves refresh persistence in Webview local storage or includes the separate OS-secret-store migration.

The recommended default is: migrate/confirm ES256, retain the default one-hour JWT until measured need says otherwise, leave refresh ownership in React for the first repair, use local Host verification for ordinary work, and fail closed online for the strict-operation set. Add a `session_id` server/RPC check only if immediate revocation is an explicit requirement.
