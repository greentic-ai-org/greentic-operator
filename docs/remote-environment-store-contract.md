# Remote `EnvironmentStore` HTTP contract (A8)

Status: **contract only** (Phase A gate A8 of `next-gen-deployment.md`). The
`LocalFsStore` in `greentic-deployer` is the only implementation that ships in
Phase A. This document is the HTTP surface every **non-local production**
`EnvironmentStore` must implement before AWS/K8s deploys can be called
production-ready (plan §388 optimistic concurrency, §389 audit + authorization,
§391 corruption detection). Non-local mutations **fail closed** until the RBAC
engine behind this contract exists.

The wire shapes are owned by the `greentic-deploy-spec` crate
(`greentic-deployer/crates/greentic-deploy-spec`). This doc references those
types by name; the crate is the normative source for field-level detail and the
HTTP status mapping (`RemoteStoreError::http_status`).

## Why a contract before an implementation

Production `Environment` state is **not** local files. `LocalFsStore` is the
developer/default implementation. Any non-local store (operator DB, Kubernetes
CRD controller, or object storage plus a real lock service) must provide
compare-and-swap writes, idempotency replay, RBAC decisions, an append-only
audit record, backup/restore, and at-rest corruption detection. Plain
`fs::write` or best-effort S3 writes are **not** production state. Defining the
contract now lets the local and remote paths converge on identical semantics
(the local path already does flock + generation checks; the remote path does the
HTTP equivalent below).

## Resources

Each environment is addressed by its `EnvId`. The mutating operations are the
same nouns/verbs the `gtc op` CLI exposes:

| Noun | Verbs (mutating) |
|------|------------------|
| `env` | `create`, `update`, `destroy` |
| `env-packs` | `add`, `update`, `remove`, `rollback` |
| `bundles` | `add`, `update`, `remove` |
| `revisions` | `stage`, `warm`, `drain`, `archive` |
| `traffic` | `set`, `rollback` |
| `config` | `set` |
| `credentials` | `bootstrap`, `rotate` |
| `secrets` | `put`, `rotate` |

Read verbs (`list`, `show`, `doctor`, `get`) are safe and unconditional.

## Common headers

Every **mutating** request:

| Header | Required | Meaning |
|--------|----------|---------|
| `Idempotency-Key` | yes | Non-empty, ULID recommended. See [Idempotency](#idempotency-2). Maps to `IdempotencyKey`. |
| `If-Match` | conditional | Strong ETag of the resource the caller last observed. Required for `update`/`destroy`/`rollback`-class verbs (any guarded write); omitted only for create-if-absent. See [Concurrency](#concurrency-1). |
| `Expected-Generation` | optional | Integer generation the caller expects. Belt-and-suspenders alongside `If-Match`. Maps to `Precondition.expected_generation`. |

A **guarded** write (anything that is not create-if-absent) must pin at least
one of `If-Match` / `Expected-Generation`. A request that pins neither is **not**
treated as an unconditional overwrite — it is rejected with `428 Precondition
Required` (`Precondition::check` → `PreconditionError::Required`). This prevents a
stale or malformed client from blindly clobbering a newer generation.

Every response that reflects committed state carries:

| Header | Meaning |
|--------|---------|
| `ETag` | Strong validator of the new state — the resource's content hash (`StateEtag`). |

## Concurrency (#1)

Optimistic concurrency. `StateEtag` is a **strong** validator: the SHA-256 of
the resource's canonical JSON (identical to the `StateIntegrity` digest). Each
resource also carries a monotonic `generation: u64`.

A guarded mutating request pins the prior state via `If-Match` (ETag),
`Expected-Generation`, or both — the `Precondition` type. The server evaluates
`Precondition::check` against current state:

- **match** → the mutation proceeds; the response carries the new `ETag` and `generation`.
- **mismatch** → `412 Precondition Failed` with a `ConcurrencyConflict` body
  (`expected_etag`, `actual_etag`, `expected_generation`, `actual_generation`).
- **empty** (pins nothing) → `428 Precondition Required`. `check` never silently
  passes a blind write; this is `PreconditionError::Required`.

Create-if-absent is the **only** mode that legitimately carries no precondition,
and it does **not** go through `Precondition::check`: the create handler is gated
by an existence check and must fail (`409`/`412`) if the resource already exists,
never silently overwrite.

## Idempotency (#2)

Every mutating request carries an `Idempotency-Key`. The server stores an
`IdempotencyRecord { key, request_fingerprint, response, stored_at }`, where
`request_fingerprint` is the SHA-256 of the canonical request body and
`response` is the **full original `MutationResponse`** — ETag, generation, the
`IdempotencyOutcome`, and the original `AuditEvent`. Persisting the whole
response (not just etag + generation) is what lets a retry whose original HTTP
response was lost be answered **verbatim** — including the original audit event
— without re-applying state or fabricating a fresh audit record.

On a key reuse the server calls `IdempotencyRecord::match_request(fingerprint)`:

| `IdempotencyReplay` | Condition | HTTP |
|---------------------|-----------|------|
| `Replay(&MutationResponse)` | same key + same fingerprint | `200` with the **stored** `MutationResponse`, returned verbatim, no re-apply |
| `Conflict { reason }` | same key + different fingerprint | `409 Conflict` (`RemoteStoreError::IdempotencyConflict`) |

A new key is `Applied`. The success `IdempotencyOutcome` recorded on a
`MutationResponse` is therefore `Applied` (first apply) or `Replayed`; conflicts
are **not** a success outcome — they surface as `RemoteStoreError::IdempotencyConflict`.

This mirrors the local `traffic::set` replay semantics already in tree: a retry
of the exact same request is safe and returns the original body; reusing a key
for a different body is an error.

## Authorization / RBAC (#3)

Before applying any mutation the server evaluates an `RbacRequest { actor, env_id, noun, verb, target }`
and produces an `AuditDecision`:

- `Allow { policy, reason }` → proceed.
- `Deny { policy, reason }` → `403` (`RemoteStoreError::Unauthorized`); the
  rejected attempt is **still audited**.

Phase A ships only the `local-only` policy (`POLICY_LOCAL_ONLY`): `env_id == "local"`
allows, anything else denies with a reason pointing at this contract. The
production RBAC engine replaces the policy while keeping the `AuditDecision`
shape.

## Audit response (#4)

Every mutating call — allowed **or** denied — writes one append-only
`AuditEvent` and returns it. A successful mutation returns:

```
MutationResponse {
  etag: StateEtag,            // new strong validator for the next CAS
  generation: u64,            // new generation
  idempotency: IdempotencyOutcome,
  audit: AuditEvent,          // the record the server wrote
}
```

`AuditEvent` carries every field plan §389 requires: `actor { kind, user, uid }`,
`env_id`, `noun`/`verb`, `target`, `previous_generation`, `new_generation`,
`idempotency_key`, `authorization` (the `AuditDecision`), and `result`
(`Ok` / `Error { kind, message }` / `NotYetImplemented { detail }`).

## Backup / restore (#5)

| Operation | Request | Response |
|-----------|---------|----------|
| Create backup | — | `BackupManifest { schema, backup_id, env_id, created_at, generation, integrity, size_bytes }` |
| List backups | — | `[BackupManifest]` |
| Restore | `RestoreRequest { backup_id, precondition }` | `RestoreOutcome { restored_generation, etag, integrity }` |

`RestoreRequest.precondition` is **mandatory and must pin prior state** — a
restore is never a create, so a blind restore could clobber a newer generation.
The field has no serde default (a request omitting it fails to deserialize), and
`RestoreRequest::validate` additionally rejects a present-but-empty precondition
(`RemoteContractError::UnconditionalRestore`). It then carries the same
`412`/`428` semantics as a normal guarded mutation. The restore response's
`integrity` lets the caller confirm the restored state hashes to the backup's
recorded digest.

## Corruption detection (#6)

`StateIntegrity { algorithm: "sha-256", digest }` is the at-rest content hash —
SHA-256 over the resource's **canonical JSON** (object keys sorted
lexicographically, no insignificant whitespace, arrays in order). The store
recomputes and compares on load; a mismatch is `422`
(`RemoteStoreError::IntegrityMismatch { expected, actual }`). `StateIntegrity::verify`
errors rather than silently passing if the stored algorithm is one this build
cannot recompute.

Canonicalization sorts keys explicitly so the digest is reproducible across
implementations and independent of any JSON library's map-ordering behavior.

## Error → HTTP status mapping

`RemoteStoreError::http_status` is the normative mapping:

| `RemoteStoreError` | HTTP | When |
|--------------------|------|------|
| `PreconditionFailed(ConcurrencyConflict)` | `412` | stale `If-Match`/generation |
| `PreconditionRequired { detail }` | `428` | guarded write pinned no prior state (blind write rejected) |
| `IdempotencyConflict { reason }` | `409` | key reused with a different body |
| `Unauthorized { policy, reason }` | `403` | RBAC deny |
| `NotFound` | `404` | unknown environment/resource |
| `IntegrityMismatch { expected, actual }` | `422` | stored state failed its hash |
| `NotYetImplemented { detail }` | `501` | recognized but unimplemented |
| `Internal { message }` | `500` | store-internal failure |

## Phase A posture

- Only `LocalFsStore` is implemented; it satisfies #1 (flock + generation), #2
  (on `traffic set`), #3 (`local-only`), #4 (append-only `events.jsonl`), and #6
  (atomic writes).
- Non-local environments **fail closed** (`403`) until the RBAC engine ships.
- Backup/restore (#5) and the full remote transport are deferred; the types are
  defined so the local and remote paths converge on one contract.
