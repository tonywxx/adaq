# Bind Desktop authentication to a Host-verified User Context

Status: accepted

For V1, the embedded Desktop Client Surface keeps Supabase sign-in, refresh, and sign-out ownership, while the ADAQ Host becomes the authority that accepts or rejects the resulting session. The Client binds the current access token through one narrow Host authentication seam on initial session, sign-in, and refresh. The Host verifies the token and derives the ADAQ User only from the verified `sub`; a request payload `userId` is never an authority input.

The Host keeps a bounded `Authenticated User Context` in process memory, bound to the invoking Client Surface/window. It clears that Context on sign-out, User change, verification failure, expiry, or window teardown. User-scoped Host Capability Contracts obtain identity from this Context; V1 makes no compatibility promise for the current Client-supplied `userId` request fields and may remove them directly.

With an asymmetric Supabase signing key, ordinary local work may use an unexpired locally verified token and a known cached project JWKS key. The Host must perform fresh online/session validation for strict operations: User changes, credentials and Secret References, destructive resets, Bot or order operations, reconciliation, and any operation whose authority depends on immediate revocation. Cold launch, unknown or rotated keys, expired tokens, missing cache, and failed online validation fail closed. If the hosted project remains HS256, Host verification is online-only through the Supabase Auth user endpoint; V1 must not claim offline authenticated work until asymmetric signing is enabled.

The Supabase Client remains responsible for session persistence. The Host does not persist raw access or refresh tokens in SQLite, Parquet, logs, Operational Events, or research evidence; it retains only the process-memory verified Context and redacted diagnostics. A restart therefore restores the Client session first and re-binds it before User-scoped Host work resumes.

## Required evidence

- Forged, mismatched, expired, malformed, unknown-key, rotated-key, and wrong-audience tokens never establish a Context.
- A valid token derives the User from `sub`; a conflicting request `userId` cannot cross User data, secrets, Components, Attempts, or reset boundaries.
- Sign-in, refresh, sign-out, User change, process restart, window teardown, offline launch, and verification failure clear or re-establish Context deterministically.
- Ordinary offline reads obey the cached-key and expiry policy; strict operations require fresh validation and fail closed offline.
- Raw tokens, authorization headers, private paths, and secrets remain absent from logs, Operational Events, SQLite, Parquet, exports, and diagnostics.
- macOS ARM64, Windows x86_64, and Linux x86_64 prove User isolation and the same Host boundary.

## Consequences

- The existing React Supabase auth experience can remain the first repair surface without making the Client authoritative.
- The current pattern of passing `userId` through Tauri commands must be replaced by Host-derived identity; this is an intentional V1-breaking change.
- Immediate server-side revocation is not promised for ordinary offline local work; strict operations pay the online-validation cost.
- No service-role secret, generic token vault, or second authentication authority is introduced.
