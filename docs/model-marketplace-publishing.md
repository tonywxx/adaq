# Future Model Marketplace Publishing

Status: Post-V1 design record. Nothing in this document is part of the V1 delivery or acceptance scope.

## Purpose

This document records how a future ADAQ Marketplace can distribute Models created or trained through Microsoft Qlib without turning arbitrary Python into trading code or confusing framework licensing with the right to redistribute a particular Model Artifact.

Qlib is an AI-oriented quantitative research platform rather than one distributable foundation model. Its repository uses the MIT License, but that licence covers Qlib software and does not by itself establish redistribution rights for third-party model code, trained weights, dependencies, or training data. See the [official Qlib repository](https://github.com/microsoft/qlib) and [Qlib licence](https://github.com/microsoft/qlib/blob/main/LICENSE).

This is an engineering and product-control workflow, not legal advice. Each publisher remains responsible for proving the rights recorded by the Marketplace review.

## Scope boundary

ADAQ V1 supports `qlib-local-paper` only as a device-bound Paper Trading runtime. It is not automatically a Marketplace Model Candidate.

The future Marketplace is phased:

1. **Portable Marketplace Models**: qualified WASI and ONNX Model Profiles.
2. **Managed Qlib Marketplace Models**: a later, separately reviewed profile for valuable Qlib Artifacts that cannot be exported faithfully.

Marketplace acceptance never grants Real Trading Qualification. Real-money eligibility remains a separate future risk, reliability, compliance, and operational decision.

## Publishable product

The Marketplace publishes an exact ADAQ Model product version, not “Qlib” itself. A Marketplace Model Product binds:

- Publisher identity and product version.
- Exact Model Artifact and Model Provenance.
- Model Scope, ordered Feature Slots, Forecast Signals, availability, missingness, and Warmup contracts.
- One Model Deployment Profile and its exact runtime identity.
- Signed graph, weight, code, environment, and dependency identities.
- Model Runtime Qualification and Component Equivalence where applicable.
- Supported operating systems, hardware requirements, latency and memory limits.
- Licence, attribution, entitlement, and redistribution-rights evidence.
- An immutable Marketplace Review Decision.

Training data is not bundled by default. Its provider, snapshot identity, permitted use, and relevant rights remain provenance evidence without redistributing provider records.

## Distribution profiles

### WASI Model

The Model Exporter generates a self-contained WASI inference Component. It is eligible only when the candidate reproduces the approved Model Artifact under the declared numeric contract and fits the supported package and runtime limits.

### ONNX Model

The Model Exporter produces an ONNX graph and exact weight assets. A controlled ADAQ runtime executes it through the same Prediction Batch and Forecast Batch contract used by other Models.

Large ONNX models may store weights in external data files instead of one graph file. ONNX Runtime documents this mechanism for models whose weights cannot fit in one Protobuf payload: [Working with Large Models](https://onnxruntime.ai/docs/tutorials/web/large-models.html).

### Managed Qlib Model

This is a future Marketplace profile, distinct from V1 Local Qlib Paper. It contains an exact Model Artifact, frozen Python/Qlib environment, dependency lock, declared inference entry point, and platform-specific runtime evidence.

It must run in an ADAQ-managed child-process sidecar with no broker credentials, order API, package installer, ambient network, or authority to alter ADAQ records. The runner receives only bounded Prediction Batches and returns bounded Forecast Batches. Rust-owned Bot, risk, portfolio, OMS, and fail-safe logic remain authoritative.

Raw Python repositories, arbitrary startup commands, notebooks, and environments that download code or dependencies at execution time are not publishable runtime formats.

## Large Artifact delivery

Large model graphs and weights must not be forced into the current 64 MiB Component archive. The future distribution shape is:

```text
signed model-*.adaq control package
  ├── product and runtime manifest
  ├── Feature and Forecast contracts
  ├── graph and weight SHA-256 identities
  ├── chunk identities, sizes, and platform requirements
  ├── licences, notices, SBOM, and provenance references
  └── Marketplace signature and Review Decision

entitlement-protected Marketplace Artifact Store
  ├── content-addressed model graph
  ├── content-addressed weight chunks or ONNX external data
  └── content-addressed managed-runtime assets when eligible
```

Installation downloads only assets named by the signed manifest, supports resumable transfer, verifies every size and hash before publication to the local Component Library, and performs an atomic registration only after the complete product passes local compatibility checks. Cached bytes may be deduplicated, but Component Entitlements remain User-scoped.

## Rights and provenance gate

Before technical publication, the publisher must provide reviewable evidence for:

- The right to publish the Model architecture and source used by the runtime.
- The right to redistribute the exact trained weights.
- The licences and notices required by Qlib and every shipped dependency.
- The identity and permitted use of each training-data source, including any restrictions on derived Models.
- The absence or declared handling of third-party pretrained weights, tokenizers, normalizers, and feature processors.
- Export, patent, trademark, jurisdiction, and commercial-use constraints that apply to the Product.
- A software bill of materials for all executable and native dependencies.

Unknown or disputed rights reject the Candidate. Strong historical performance never overrides missing redistribution authority.

## Complete publishing workflow

| Stage | Required action | Evidence or gate |
| --- | --- | --- |
| 1. Candidate creation | Select one immutable Model Artifact and proposed Deployment Profile. | Marketplace Model Candidate identity. |
| 2. Provenance freeze | Bind training protocol, inputs, Targets, transformations, code, framework, Adapter, environment, Seed, and all payload hashes. | Complete Model Provenance; unknown fields remain explicit. |
| 3. Rights intake | Review publisher identity, source, weights, data, dependencies, commercial-use rights, notices, and jurisdiction constraints. | Rights dossier; failure rejects the Candidate. |
| 4. Profile selection | Choose WASI, ONNX, or future Managed Qlib from actual exporter and runtime support. | No false portable wrapper and no automatic promotion from Local Qlib Paper. |
| 5. Build or export | Run the versioned Exporter or construct the frozen Managed Qlib bundle. | Reproducible Build Attempt, logs, toolchain identity, and candidate payload hashes. |
| 6. Scientific equivalence | Replay frozen golden inputs and compare row identity, availability, missingness, outputs, and numeric behavior with the approved Artifact. | Component Equivalence for WASI/ONNX; exact or declared tolerance replay for Managed Qlib. |
| 7. Runtime qualification | Test schema, finite values, deadlines, cancellation, crash handling, resource ceilings, supported platforms, and repeated inference. | Model Runtime Qualification Report. |
| 8. Causality review | Verify Feature availability, training and normalization windows, state resets, Bar Gaps, and absence of future-data access. | No-lookahead and Feature replay evidence. |
| 9. Security review | Scan executable payloads and dependencies, verify restricted capabilities, reject secrets and dynamic installation, and exercise sandbox boundaries. | Security and SBOM review tied to exact hashes. |
| 10. Trading evidence | Run ADAQ-native Backtest and Validation on the exact candidate contract and evidence. | Historical evidence only; no profitability guarantee or Real Trading approval. |
| 11. Package and sign | Create the canonical manifest, upload content-addressed assets, and sign the exact Product version. | Package signature, asset inventory, and immutable Product identity. |
| 12. Marketplace decision | Review all cited evidence without modifying it. | Accepted or Rejected Marketplace Review Decision. |
| 13. Publication | Publish listing metadata, licence, price or entitlement policy, supported platforms, resource needs, caveats, and exact version. | Marketplace Model Product becomes discoverable and installable. |
| 14. Installation | Authorize entitlement, download manifest-named assets, verify signatures and hashes, validate compatibility, and register atomically. | User-scoped entitlement and local immutable package identity. |
| 15. Paper execution | Supply frozen Features to the qualified runner and return Forecast Signals to the Rust-owned Bot. | Runtime health, latency, Signals, orders, and pauses enter monitoring evidence. |
| 16. Update or withdrawal | Publish changes as a new immutable version; never mutate installed evidence in place. Suspend or withdraw an exact version through a new Review Decision. | Existing Runs retain their locked identities and historical evidence. |

## Execution fail-safe

The Model runtime never receives exchange credentials or sends orders. If inference is late, malformed, non-finite, crashed, unavailable, or incompatible with the frozen Feature Plan, ADAQ produces no new prediction-driven risk and records a Bot pause. Restart and recovery may resume only from an evidence-safe boundary; no runner may invent or backfill live Signals silently.

## Marketplace states

The minimum future lifecycle is:

```text
Draft
  → Candidate
  → Accepted → Published
  → Rejected
  → Suspended
  → Withdrawn
```

Every transition after Candidate creation is an immutable decision over an exact Product version. A corrected Model is a new version and new Candidate, not an edit to previously published evidence.

## V1 foundations retained for later

The following V1 concepts are deliberately sufficient inputs to the future workflow:

- Model Training Protocol and Attempt.
- Model Artifact and Model Provenance.
- Model Exporter and Deployment Profile.
- Feature Plan, Forecast Signal Dataset, and Evaluation Report.
- Component Build Attempt and Component Equivalence Report.
- Model Runtime Qualification Report.
- ADAQ Backtest, Validation, Paper Bot, and monitoring evidence.

## Deferred Marketplace decisions

These remain future design questions and are not decided by this document:

- Artifact-store quotas, regional replication, pricing, billing, refunds, and revenue share.
- Exact publisher verification and legal-review workflow.
- Weight encryption, offline access, licence enforcement, and intellectual-property protection.
- Exact Managed Qlib sandbox technology and supported dependency policy.
- Supported ONNX operators, execution providers, accelerators, and hardware tiers.
- Security-suspension behavior for already installed Products.
- The independent promotion process from Marketplace or Paper Trading to Real Trading Qualification.
