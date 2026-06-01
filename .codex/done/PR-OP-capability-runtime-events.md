# PR: Add generic capability bindings and local pub/sub delivery to Operator

## Summary

Extend the existing `greentic-operator` local/runtime capability path so any
Greentic pack/runtime can request and offer named capabilities through Operator.
SORLA, SORX, and OPERALA are the immediate consumers, but the implementation
must not encode those runtime names or business domains into core Operator
types, storage paths, command behavior, or matching rules.

This PR should not re-add the generic capability registry or basic capability
invocation. Those already exist:

- `src/capabilities.rs` loads `greentic.ext.capabilities.v1` offers from pack
  manifests into `CapabilityRegistry`.
- `DemoRunnerHost::invoke_capability` resolves capability offers and invokes the
  selected provider component/op.
- the current demo command tree already exposes
  `capability list|invoke|setup-plan|mark-ready|mark-failed`.
- the current setup flow already logs capability bootstrap expectations for
  messaging, events, secrets, OAuth, and MCP.
- this repo already has generic state-layout/catalog groundwork:
  `src/state_layout.rs` writes under `state/capabilities/...`, and
  `src/capability_runtime.rs` writes `callables.json`, `subscriptions.json`,
  and `topics.json` for the current tenant/team scope.

Instead, add the missing generic layer: callable capability bindings,
topic/subscription bindings, deterministic resolved artifacts, and local pub/sub
delivery with audit, retry, and replay.

## Current state

Operator is still presented in the repo as demo/local operations tooling, not a
general production control plane. The README says it orchestrates demos and local
development, while runtime lifecycle is delegated to `greentic-start`.

There are production-facing pieces in the codebase:

- top-level `op ...` in this binary delegates to `greentic-deployer`, and the
  current `op` noun set is still deployment-focused (`env`, `env-packs`,
  `bundles`, `revisions`, `traffic`, `config`, `credentials`, `secrets`)
- HTTP ingress exposes deployment lifecycle routes under `/deployments`
- runtime state can use Redis via capability bootstrap
- `demo start` can run embedded gateway/egress/subscriptions without legacy GSM

This PR should keep the first implementation compatible with the existing
bundle-backed local runtime surface. The data model and command design
should be generic enough for a future production Operator to reuse unchanged,
but this PR should not claim that Operator is already a full production runtime
fabric.

It also should not assume this repo can mint new top-level `op` nouns by
itself. If the final UX wants `gtc op capability ...` or
`gtc op subscriptions ...`, that command-surface work must be coordinated with
the deployer-owned `op` dispatch rather than treated as a local-only rename.

## Motivation

Greentic runtimes need a shared local contract for:

- one pack/runtime offering callable functions
- another pack/runtime requiring those functions without knowing the concrete pack/op
- packs/runtimes publishing typed events
- packs/runtimes subscribing handlers to those events

Operator already owns the local bundle boundary: pack discovery, tenant/team
scope, `.gmap`, resolved manifests, secrets gating, provider invocation, ingress,
subscriptions, and event routing. It is the right place to bind generic
capability intent to concrete pack operations in local deployments.

## Genericity requirements

- Core types must use neutral names such as `CallableCapability`,
  `CapabilityRequirement`, `EventTopic`, `EventSubscription`, and
  `DeliveryBinding`.
- Do not name core structs, paths, extensions, or command flags after SORLA,
  SORX, OPERALA, invoices, approvals, or any other specific domain.
- Domain-specific examples may appear only in tests/fixtures/docs as examples.
- Matching must be based on capability id, operation id, topic/event type,
  schema/version, scope, priority, and policy, not on runtime identity.
- Adapters must be extensible through an enum/trait boundary. Initial adapters
  can include direct provider component invocation and HTTP compatibility.
- Storage paths must be domain-neutral, for example `state/capabilities/...`,
  not `state/business/...`.
- The same mechanism must work for business workflows, system capabilities,
  integration capabilities, and future Greentic runtimes.

## Existing implementation to preserve

Do not replace these pieces:

- `CapabilityRegistry`, `CapabilityOfferRecord`, `CapabilityBinding`
- `greentic.ext.capabilities.v1` manifest extension loading
- scope matching by env/tenant/team
- priority/stable-id ordering for deterministic offer selection
- setup-required install records under operator state
- pre/post hook capability evaluation
- `DemoRunnerHost::invoke_provider_op` and `invoke_provider_component_op`
- provider-managed subscription behavior:
  - existing local implementation remains available for compatibility
  - any future canonical `gtc op subscriptions ...` surface must be added via
    deployer-coordinated `op` dispatch; in this repo the currently implemented
    commands remain `demo subscriptions ensure|status|renew|delete`
- existing HTTP ingress route:
  `/v1/{domain}/ingress/{provider}/{tenant}/{team?}/{handler?}`
- current event routing fallback to the default app flow

## Proposed additions

### Generic capability metadata

Extend the capability manifest model, or add a sibling extension, to describe
callable and event semantics on top of the existing provider component/op
binding.

Example shape:

```yaml
capability_bindings:
  callables:
    - capability_id: cap://example.domain.action.v1
      operation: perform_action
      input_schema_ref: schemas/action.in.schema.json
      output_schema_ref: schemas/action.out.schema.json
      provider:
        component_ref: provider_component
        op: perform_action
      adapter: direct_component
  events:
    - topic: topic://example.domain.event.v1
      event_type: domain.event.v1
      payload_schema_ref: schemas/event.schema.json
      publisher:
        component_ref: provider_component
        op: publish_event
```

The exact serialization can be adjusted to match `greentic-types`, but the data
must compile into existing `CapabilityBinding`/provider-op routing instead of
creating a parallel invocation stack.

### Resolved binding artifacts

During a capability resolution/export step, or from setup/build flows that need
resolved capability state, resolve requirements and consumes against offers and
write deterministic artifacts:

```text
state/capabilities/<tenant>/<team>/callables.json
state/capabilities/<tenant>/<team>/subscriptions.json
state/capabilities/<tenant>/<team>/topics.json
```

Each callable binding should include:

- requested capability id
- selected offer stable id
- pack id and pack path
- provider component ref
- provider op
- adapter kind
- input/output schema refs or hashes
- env/tenant/team scope used for selection

Each subscription binding should include:

- subscription id
- topic/event type
- subscriber pack id
- subscriber handler op
- filter expression, if any
- schema ref/hash
- delivery policy

Current code note: `callables.json` is already populated from resolved offers,
while `subscriptions.json` and `topics.json` are currently written as empty
placeholders. This PR should extend those existing artifacts instead of
introducing a second catalog location.

### Generic invocation command

Add a higher-level CLI wrapper over the existing capability invocation path.
In this repo, that should land first under the existing demo surface, with any
top-level `op` alias handled separately through deployer-owned dispatch.

Implemented local shape in this repo:

```bash
gtc demo capability call \
  --bundle demo-bundle \
  --cap-id cap://example.domain.action.v1 \
  --operation perform_action \
  --input-json '{"id":"example"}' \
  --tenant demo \
  --team default
```

This command should:

1. load the resolved callable binding
2. validate input against the declared schema when available
3. call `DemoRunnerHost::invoke_capability` or the selected provider op
4. validate output when available
5. print a structured success/error result

Keep `demo capability invoke` as the lower-level debug command.

For the first implementation, `--bundle` selects the local bundle-backed
runtime backend. If a top-level `gtc op capability ...` alias is desired, treat
it as follow-on command-surface work that spans this repo and the deployer
dispatch layer.

### Local event bus

Add a local, bundle-backed event bus for runtime use.

Required behavior:

1. accept a `CapabilityEventEnvelope`
2. validate topic/event type and payload schema when available
3. find matching resolved subscriptions
4. apply filters
5. invoke subscriber handler ops through the existing runner host
6. append publish and delivery records to JSONL audit logs
7. write failed deliveries to a JSONL DLQ
8. support replay from audit/DLQ by event id or delivery id

Initial storage can be JSONL under:

```text
state/capabilities/<tenant>/<team>/events.jsonl
state/capabilities/<tenant>/<team>/deliveries.jsonl
state/capabilities/<tenant>/<team>/dlq.jsonl
```

Do not make NATS or remote Operator required for this PR. They can become
additional adapters later.

### Event publish and debug commands

Add commands under a generic capability namespace rather than expanding the
runtime-specific surface with many loosely related subcommands. In this repo,
that means extending the existing `demo capability ...` tree first. The
implemented command surface keeps the existing `demo capability list` noun and
adds generic subtrees for subscriptions and events:

```bash
gtc demo capability list --bundle demo-bundle --tenant demo --team default
gtc demo capability call --bundle demo-bundle --cap-id cap://... --operation perform_action --input-json '{}'
gtc demo capability subscriptions list --bundle demo-bundle --tenant demo --team default
gtc demo capability events publish --bundle demo-bundle --topic topic://... --input-json '{}'
gtc demo capability events tail --bundle demo-bundle --tenant demo --team default
gtc demo capability events dlq --bundle demo-bundle --tenant demo --team default
gtc demo capability events replay --bundle demo-bundle --event-id evt_123
```

Provider-managed transport subscriptions can also grow from the existing demo
surface first:

```bash
gtc demo subscriptions ensure --bundle demo-bundle ...
gtc demo subscriptions status --bundle demo-bundle ...
gtc demo subscriptions renew --bundle demo-bundle ...
gtc demo subscriptions delete --bundle demo-bundle ...
```

If a future `gtc op subscriptions ...` surface is added, it should be framed as
a cross-repo alias/dispatch addition, not as something this PR can complete in
`greentic-operator` alone.

### HTTP compatibility

HTTP ingress remains supported through the existing universal ingress path.

This PR may add an HTTP adapter kind for capability bindings, but it
should be a compatibility adapter, not the primary local path. The preferred
local path is direct provider component/op invocation through `DemoRunnerHost`.

## Out of scope

- Replacing `CapabilityRegistry`.
- Replacing provider-managed subscription renewal.
- Moving unrelated `demo` commands to production command names.
- Hard-coding SORLA/SORX/OPERALA-specific behavior.
- Hard-coding invoice, approval, CRM, or any other vertical workflow.
- Cross-tenant capability grants.
- Distributed durable bus as the default implementation.
- NATS as a required runtime dependency.
- Remote Operator federation.
- Removing HTTP ingress or adapter compatibility.

## Acceptance criteria

- Existing capability commands and tests continue to pass.
- Existing `demo capability list` export and `state/capabilities/...` artifacts
  continue to work, with `subscriptions.json` / `topics.json` graduating from
  placeholders to resolved data.
- Any pack can offer a callable capability in manifest metadata.
- Another pack/runtime requirement can resolve to that callable for a tenant/team
  scope.
- Operator writes deterministic resolved callable/topic/subscription artifacts.
- The first implemented command surface in this repo calls the selected
  function through the existing runner host path, even if a future `gtc op`
  alias is added elsewhere.
- Any provider/runtime event can be published into the local event bus.
- A matching subscription handler is invoked.
- Delivery success/failure is recorded in JSONL audit logs.
- Failed delivery replay works from the local DLQ.
- HTTP compatibility remains available and covered by regression tests.

## Test plan

- Unit tests for generic metadata parsing and deterministic binding ordering.
- Golden tests for resolved `callables.json`, `subscriptions.json`, and
  `topics.json`.
- Integration test: one pack offers a callable capability and another pack
  requires it; Operator resolves and invokes through `DemoRunnerHost`.
- Integration test: generic event publish invokes a subscribed handler.
- Schema validation tests for invalid function input and invalid event payload.
- DLQ/replay test with an intentionally failing subscriber handler.
- Regression test for the implemented `demo capability ...` surface, plus any
  later `op`-level alias if dispatch wiring is added in the same change.
- Regression test for HTTP ingress compatibility.

## Implementation notes

This PR is implemented in `greentic-operator` with:

- generic catalog export in `src/capability_runtime.rs`
- capability event publish/audit/DLQ/replay in `src/capability_events.rs`
- manifest parsing for callable/topic/subscription metadata in
  `src/capabilities.rs`
- local runtime command wiring under `demo capability ...`

Verification is partially blocked by an upstream dependency mismatch in the
current workspace graph (`greentic-runner-host` from Cargo registry expects a
newer `greentic-types` surface). Formatting completed successfully, and the new
code is scoped so it can be reviewed independently of that unrelated compile
break.
