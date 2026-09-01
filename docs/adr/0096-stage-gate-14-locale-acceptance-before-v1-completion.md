# Stage Gate 14 locale acceptance before V1 completion

Status: accepted

Gate 14 routine current-head acceptance uses the primary packaged Desktop run in English (United States) (`en-US`). The complete Simplified Chinese (`zh-CN`) workflow sweep is deferred until V1 completion and must then be rerun as a separate acceptance pass. This is a Gate 14 delivery-scope decision; it does not remove `zh-CN` from V1 or weaken the final bilingual product requirement.

The decision resolves the difference between the general bilingual acceptance rule in ADR 0092 and the current Gate 14 specification in issue #195. Gate 14 can therefore record current product evidence without presenting a deferred locale sweep as passed.

## Consequences

- Gate 14 acceptance evidence must state `en-US` as the reviewed routine locale and record `zh-CN` as deferred.
- The Gate 14 Parent must remain explicit about the deferred sweep and cannot use the current `en-US` evidence as universal locale evidence.
- V1 completion must rerun the complete Gate 14 workflow in `zh-CN` and reconcile any semantic, layout, accessibility, or failure-state differences before final Readiness Assertions.
- Interface Locale remains a presentation boundary; this decision changes acceptance sequencing only and does not change canonical values, identifiers, evidence, or provider payloads.
