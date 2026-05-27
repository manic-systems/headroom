# Headroom Refactor Plan

This plan synthesizes the thermo-nuclear code quality review and the
architecture review. It is intentionally structural: the goal is to reduce
sprawling control flow, deepen shallow modules, and make the codebase easier
to test and navigate without changing behavior.

## Guiding Principles

- Preserve behavior first; refactors should land behind existing tests or with
  new tests that cover observable behavior.
- Prefer deep modules: small interfaces with substantial implementation hidden
  behind them.
- Move policy into the module that owns the concept. Avoid more feature checks
  scattered through shared flows.
- Delete coordination complexity where possible. Do not just move large maps,
  conditionals, and flags into new files.
- Keep comments terse and lowercase, with minimal punctuation. Omit comments
  unless they add value for a competent reader. Do not use em dashes or
  distinctive generated-text tells. Comments must only reference context that
  is immediately apparent to an outside reader familiar with the field; never
  reference prompts, review artifacts, or discussion history.
- Keep topic commits in the repo's house style, folding refactors into the
  relevant feature commits while this branch remains pre-merge.

## Execution Workflow

Use this workflow for each refactor phase:

1. Convert the phase into a concrete implementation outlay before editing code.
2. Split independent work into temporary worktrees, usually tests, extraction,
   and guardrails or audit checks.
3. Reconcile the worktrees into `main` only after the local package tests pass.
4. Run focused tests, the file-size guardrail, and a thermo-nuclear audit over
   the reconciled diff.
5. Fix any structural audit issues before committing.
6. Commit as fixups and autosquash into the relevant feature-shaped commits.

## Phase 0: Safety Net Before Surgery

### 0.1. Capture Current Behavior With Focused Tests

Add tests around the behaviors most likely to regress during decomposition:

- `setting.set` for keys that affect DSP, routing, default route, global bypass,
  and per-app level control.
- Full bypass behavior: default sink policy, processed graph inactive, routed
  streams retargeted to the real sink.
- PipeWire graph intent behavior using a fake object/link adapter where possible.
- Profile overlay materialization, including invalid setting fallback and
  active-profile-missing fallback.
- TUI/GUI event reduction behavior once shared monitor state exists.

Expected outcome: future refactors can prove behavior through stable seams
rather than through incidental internal state.

### 0.1 Implementation Outlay

Land this as small test-first commits. The intent is not to refactor yet; it
is to pin the behavior that the later refactors must preserve.

#### A. Profile Mutation Effect Tests

Target files:

- `crates/headroom-core/src/ipc/ops.rs`
- `crates/headroom-core/src/profile_store.rs`
- `crates/headroom-core/src/state.rs`

Add IPC-level tests using `ops::dispatch` and a test `SharedState` with:

- a `FilterControl::for_testing` receiver to observe DSP commands;
- a `pw_command_tx` test channel to observe `PwCommand` posts;
- a `Broadcaster` subscriber if event publication needs direct assertion.

Cases to pin:

- `setting.set agc.enabled=false` pushes AGC enable state to the filter.
- `setting.set compressor.enabled=false` pushes compressor config.
- `setting.set limiter.ceiling_dbtp=-1.5` pushes limiter config.
- `setting.set default_route.route=bypass` posts `ReevaluateAll`.
- `setting.set per_app.enabled=false` posts `ReevaluateLayerA`.
- `setting.clear` and `setting.reset` post all required reapply work.
- `profile.use` and `profile.reload` push DSP and post route/per-app
  re-evaluation.
- `bypass.set` posts route re-evaluation.
- `route.set` / `route.unset` post route re-evaluation.
- `per-app.set` / `per-app.master` post Layer A re-evaluation.

Expected immediate finding: at least one `setting.set` case will expose the
stale-effect bug from the review. Capture the failing test first, then decide
whether to fix it inside Phase 0 or defer the fix to Phase 1.

#### B. Bypass And Default Policy Tests

Target files:

- `crates/headroom-core/src/state.rs`
- `crates/headroom-core/src/routing.rs`
- `crates/headroom-core/src/pw/registry.rs`

Start with pure tests before touching PipeWire-heavy code:

- `routing::evaluate` sends every routable stream to `Bypass` under global
  bypass.
- `routing::evaluate` sends >2-channel streams to `Bypass`.
- `DaemonState::apply_real_sink_change` retargets only bypass streams.

Then add registry-policy tests only where existing seams allow it:

- default route `Bypass` and global bypass should prefer the real sink as the
  default sink policy.
- bus graph should be inactive when global bypass is true.
- bus graph should be active when at least one effective stream is processed.

If registry tests require too much private-state access, record the gap and
defer detailed coverage until the graph reconciler seam exists.

#### C. Profile Store Materialization Tests

Target files:

- `crates/headroom-core/src/profile_store.rs`

Expand existing tests to pin:

- invalid active profile falls back to built-in default and keeps the missing
  profile warning;
- invalid setting override is preserved on disk but skipped in the active
  materialized profile;
- valid route overrides are prepended and shadow profile rules;
- per-app overrides update existing matching rules and synthesize fallback
  rules when no match exists;
- `setting_overrides()` reports user-visible values in JSON shape.

These tests are the safety net for Phase 2. They should assert observable store
behavior, not JSON/TOML implementation details that Phase 2 intends to delete.

#### D. Monitor State Regression Tests

Target files:

- `crates/headroom-cli/src/tui.rs`
- `crates/headroom-gui/src/app.rs`

Before extracting shared monitor state, make the duplicated behavior explicit:

- snapshot merge prefers `route.list.current` and fills missing streams from
  `status.streams`;
- `stream_routed` inserts/updates stream route;
- `stream_removed` clears stream, Layer A level, and Layer A snapshot;
- `layer_a_attached` creates an unknown-reduction entry;
- `layer_a_level` updates reduction;
- daemon overflow accumulates monotonically;
- profile `used` / `changed` behavior is consistent between TUI and GUI.

If the TUI and GUI currently disagree, mark the expected canonical behavior in
the test name and fix before Phase 6.

#### E. Verification Commands

Run after each test batch:

```sh
cargo test -p headroom-core
cargo test -p headroom-cli
cargo test -p headroom-gui
cargo test
```

Use the smaller package test while iterating; run the full workspace before
committing Phase 0.

### 0.2. Add Architectural Guardrails

- Track first-party Rust file sizes in CI or a local check.
- Flag files over 1,000 lines for deliberate decomposition:
  - `crates/headroom-core/src/pw/registry.rs`
  - `crates/headroom-gui/src/app.rs`
  - `crates/headroom-core/src/profile_store.rs`
  - `crates/headroom-cli/src/tui.rs`
  - `crates/headroom-core/src/ipc/ops.rs`
  - `crates/headroom-dsp/src/limiter.rs`
  - `crates/headroom-core/src/app_level.rs`

Expected outcome: the current large-file problem stops getting worse while the
deeper refactors proceed.

### 0.2 Implementation Outlay

Add a small first-party file-size check. Keep it boring and local.

Suggested shape:

- script path: `scripts/check-rust-file-sizes.sh`;
- default threshold: `1000` lines;
- scan only `crates/**/*.rs`;
- ignore generated/build output;
- print all files over threshold with line counts;
- fail only when a file exceeds a hard threshold or when a new file crosses the
  threshold, depending on how strict we want CI to be.

Because the repo already has known large files, start with a tracked allowlist
inside the script:

- `crates/headroom-core/src/pw/registry.rs`
- `crates/headroom-gui/src/app.rs`
- `crates/headroom-core/src/profile_store.rs`
- `crates/headroom-cli/src/tui.rs`
- `crates/headroom-core/src/ipc/ops.rs`
- `crates/headroom-dsp/src/limiter.rs`
- `crates/headroom-core/src/app_level.rs`

Behavior:

- allowed files over threshold print a warning;
- non-allowed files over threshold fail;
- allowed files growing materially should be reviewed manually during refactor
  commits.

Optional follow-up:

- wire the script into CI once CI shape is clear;
- add `cargo clippy --all-targets --all-features` as advisory until the
  existing missing-doc warnings are either fixed or consciously allowed.

## Phase 1: Profile Mutation Effects

### Problem

IPC mutation paths manually decide which side effects to run. `setting.set`
pushes DSP changes but does not consistently trigger routing or per-app
re-evaluation, while `setting.clear` and `setting.reset` already assume any
setting may affect any subsystem.

Relevant files:

- `crates/headroom-core/src/ipc/ops.rs`
- `crates/headroom-core/src/profile_store.rs`
- `crates/headroom-core/src/state.rs`
- `crates/headroom-core/src/pw/command.rs`

### Refactor Item

Introduce one profile mutation effects module that owns:

- store mutation result classification;
- profile/routing event publication;
- DSP snapshot construction and push;
- PipeWire route re-evaluation posts;
- Layer A re-evaluation posts;
- bypass/default policy consequences.

The interface should be one mutation application path, not a family of
per-operation helpers that recreate the same orchestration.

### Work Plan

1. Add tests documenting current side effects for each profile-affecting IPC op.
2. Extract common post-mutation orchestration from `ipc/ops.rs`.
3. Have `ProfileStore` or the new module return a typed effect summary for
   changes to routing, DSP, per-app, default route, and bypass policy.
4. Route `profile.use`, `profile.reload`, `setting.set`, `setting.clear`,
   `setting.reset`, `route.set`, `route.unset`, `bypass.set`,
   `per-app.set`, and `per-app.master` through the same application path.
5. Remove duplicated publication/posting/push logic from `ipc/ops.rs`.

### Done When

- `setting.set` cannot leave stale routing or per-app state for settings that
  change those concepts.
- The side-effect order is documented and tested at one seam.
- `ipc/ops.rs` becomes request dispatch plus thin response conversion, not
  profile-effect orchestration.

### 1.0 Implementation Outlay

Parallelize Phase 1 into three temporary branches:

#### A. Effect-Orchestration Extraction

Target files:

- `crates/headroom-core/src/ipc/ops.rs`
- new `crates/headroom-core/src/ipc/profile_effects.rs` or
  `crates/headroom-core/src/ipc/ops/profile_effects.rs`

Build one small mutation effect interface that can:

- capture the post-mutation DSP snapshot while the lock is held;
- clone any command/control handles needed after the lock drops;
- publish the appropriate profile/routing events;
- post route re-evaluation exactly once when required;
- post Layer A re-evaluation exactly once when required;
- push DSP updates after the lock drops.

Start with broad effect categories rather than trying to classify every
setting key perfectly. Phase 2 will provide typed setting categories. Phase 1
should remove duplicated orchestration and make the broad behavior explicit.

#### B. Exact Side-Effect Tests

Target files:

- `crates/headroom-core/src/ipc/ops/effect_tests.rs`

Strengthen the Phase 0 tests:

- assert exact `PwCommand` counts for route and Layer A re-evaluation;
- assert `profile.use` and `profile.reload` do not post duplicate route
  re-evaluation;
- assert event publication for profile used, profile reloaded, and route rule
  changed where the broadcaster seam allows it;
- keep tests at the IPC seam so the extraction can move internals freely.

#### C. Guardrail And Audit Pass

Target files:

- `scripts/check-rust-file-sizes.sh`
- touched Phase 1 modules

Run after reconciliation:

```sh
nix develop -c cargo test -p headroom-core
scripts/check-rust-file-sizes.sh
```

Audit questions:

- did the new module delete orchestration complexity rather than hiding it?
- does `ipc/ops.rs` read more like request dispatch and response conversion?
- are effect flags few and explicit, or did they become another scattered
  policy language?
- did any already-oversized file grow when a module extraction was available?

## Phase 2: Typed Settings Instead Of Magical Dotted Patching

### Problem

`ProfileStore` materializes profiles by serializing the typed profile to JSON,
patching dotted keys, then deserializing back. This hides setting invariants,
makes effect classification hard, and permits fallback behavior that is too
easy to misunderstand.

Relevant files:

- `crates/headroom-core/src/profile_store.rs`
- `crates/headroom-core/src/profile.rs`
- `crates/headroom-core/src/ipc/ops.rs`
- `crates/headroom-ipc/src/proto.rs`

### Refactor Item

Create a typed settings module that owns:

- the list of mutable setting keys;
- setting value validation and conversion;
- applying a setting to a `Profile`;
- listing/getting settings for IPC;
- setting effect classification.

This should reduce reliance on `serde_json::Value` inside core logic. IPC can
still accept JSON at the wire seam, but core should convert into typed setting
operations immediately.

### Work Plan

1. Enumerate every setting key currently intended to be user-mutable.
2. Implement typed lookup/apply/list operations for those keys.
3. Keep compatibility with existing dotted key strings at the IPC seam.
4. Move TOML/JSON conversion to persistence and wire adapters only.
5. Delete `set_dotted`, broad JSON round-tripping, and skip-on-failure
   materialization once typed settings cover current behavior.

### Done When

- Unknown setting keys fail at the setting seam.
- Wrong-type setting values fail before profile rematerialization.
- Each setting reports its effect category.
- Profile materialization no longer requires full JSON round-trip patching.

## Phase 3: PipeWire Graph Reconciler

### Problem

PipeWire link intent, existing links, bus graph activation, retry state, and
cleanup are coordinated by several maps and flags in `RoutingState`:
`pending_routes`, `managed_route_links`, `links_by_id`,
`outbound_links_by_node`, and `bus_graph_active`. This makes graph correctness
depend on many scattered special cases.

Relevant files:

- `crates/headroom-core/src/pw/registry.rs`
- `crates/headroom-core/src/pw/metadata.rs`
- `crates/headroom-core/src/pw/command.rs`

### Refactor Item

Introduce a graph reconciler module. It should receive desired graph state and
an indexed view of current PipeWire objects, then create/destroy links through
an adapter.

The important shift: callers describe desired links; the reconciler owns
deduplication, conflicting-link cleanup, retry, and teardown.

### Work Plan

1. Extract a PipeWire object index for nodes, ports, sinks, streams, and links.
2. Model desired graph links for:
   - app stream -> processed sink;
   - app stream -> real sink;
   - processed monitor -> filter;
   - filter -> real sink;
   - Layer A tap passive links.
3. Add a production adapter around `link-factory` create/destroy operations.
4. Add an in-memory adapter for tests.
5. Move `apply_pending_routes`, `drop_bus_graph_links`, link cleanup, and
   conflicting-link destruction into the reconciler.
6. Delete graph-specific maps from `RoutingState` once the reconciler owns
   them.

### Done When

- Bus graph activation is a desired-graph input, not a flag with scattered
  cleanup.
- Link creation and destruction are tested without live PipeWire.
- Registry callbacks no longer manipulate raw route/link maps directly.

## Phase 4: Split `RoutingState` Into Owned Domain Modules

### Problem

`crates/headroom-core/src/pw/registry.rs` is over 2,000 lines and mixes:

- registry object discovery;
- default sink policy;
- stream route policy application;
- bus graph policy;
- graph link reconciliation;
- Layer A lifecycle;
- real sink format tracking;
- object removal cleanup;
- event publication.

### Refactor Item

After the graph reconciler exists, split `RoutingState` into domain modules:

- `PwObjectIndex`: registry globals, ports, links, sinks, streams.
- `DefaultSinkPolicy`: real sink adoption and default sink reassertion.
- `StreamRouter`: route evaluation and desired stream routes.
- `BusGraph`: whether the processed graph should exist.
- `LayerAManager`: tap/controller lifecycle, deference, volume writes.
- `RegistryDriver`: PipeWire callback wiring and command draining.

### Work Plan

1. Extract pure or near-pure modules first: object index and default sink policy.
2. Move graph operations only after Phase 3 removes raw link-map coupling.
3. Move Layer A lifecycle into a manager with a small interface.
4. Keep `RegistryDriver` as orchestration over the other modules.
5. Add tests at each new seam and remove tests that assert incidental
   `RoutingState` internals.

### Done When

- `registry.rs` is a driver, not the implementation of every PipeWire concept.
- Each domain can be understood without reading the whole registry file.
- Removal cleanup is localized to the module that owns each resource.

## Phase 5: Layer A Decomposition

### Problem

`app_level.rs` sits at exactly 1,000 lines and contains matching, controller
state, deference, silence catchup, echo suppression, and tests. It is more
cohesive than `registry.rs`, but it is now at the size limit.

Relevant files:

- `crates/headroom-core/src/app_level.rs`
- `crates/headroom-core/src/pw/registry.rs`
- `crates/headroom-core/src/pw/tap.rs`

### Refactor Item

Deepen Layer A around a controller interface while separating:

- rule matching/evaluation;
- gain controller;
- external volume deference;
- echo suppression;
- tap lifecycle integration.

### Work Plan

1. Keep `AppLevelController` as the main behavior seam.
2. Extract rule evaluation and matching if it can move without increasing
   caller knowledge.
3. Extract echo/deference state if it reduces controller complexity.
4. Move tests into focused modules or integration-style tests around the
   controller seam.
5. Coordinate with Phase 4 so tap lifecycle belongs in `LayerAManager`, not
   `registry.rs`.

### Done When

- Layer A behavior remains testable through the controller seam.
- `app_level.rs` drops below the size threshold without creating pass-through
  modules.
- Registry code no longer owns Layer A lifecycle details.

## Phase 6: Shared Monitor State For CLI And GUI

### Problem

The TUI and GUI both interpret daemon snapshots, wire events, profile changes,
routing changes, Layer A updates, overflow events, and override formatting.
This duplicates protocol semantics and invites frontend drift.

Relevant files:

- `crates/headroom-cli/src/tui.rs`
- `crates/headroom-gui/src/app.rs`
- `crates/headroom-gui/src/io.rs`
- `crates/headroom-ipc/src/proto.rs`

### Refactor Item

Create a shared monitor state reducer module. It should accept:

- initial `Status`;
- `RouteList`;
- profile list snapshots;
- typed daemon events;
- control-result snapshots.

It should expose reduced monitor state for frontends to render. The TUI and GUI
should keep their rendering and input code, but stop owning protocol event
semantics.

### Work Plan

1. Extract duplicate snapshot merge and event application tests from both
   frontends.
2. Implement shared state reduction in a reusable crate or shared module.
3. Move override formatting or expose a display-ready projection.
4. Port the TUI to render the shared state.
5. Port the GUI to render the shared state.
6. Delete duplicate event injection/reduction logic from both frontends after
   Phase 7.

### Done When

- TUI and GUI event behavior is tested once.
- Frontend files shrink substantially.
- Adding a protocol event does not require duplicating reducer logic.

## Phase 7: Align Event Typing With Wire Shape

### Problem

The wire event frame carries `event` outside `data`, but typed event enums are
tagged inside `data`. Both TUI and GUI manually re-inject the event name before
deserializing.

Relevant files:

- `crates/headroom-ipc/src/proto.rs`
- `crates/headroom-cli/src/tui.rs`
- `crates/headroom-gui/src/app.rs`
- `crates/headroom-client/src/client.rs`

### Refactor Item

Move event decoding into `headroom-ipc` or `headroom-client` so callers receive
typed events without knowing about the wire reshaping detail.

### Work Plan

1. Add typed decode helpers for routing, profile, daemon, and meter events.
2. Test decode helpers against representative wire frames from `IPC.md`.
3. Replace TUI/GUI manual injection with typed decode calls.
4. Feed typed events into the shared monitor reducer from Phase 6.

### Done When

- No frontend manually constructs an injected event payload.
- Protocol shape knowledge is localized to the IPC/client module.
- Event tests live at the protocol seam.

## Phase 8: Limiter Internal Decomposition

### Problem

`limiter.rs` is over 1,000 lines and mixes config sanitation, hard tier, soft
tier, oversampled path state, silence idle state, telemetry, and tests.

Relevant files:

- `crates/headroom-dsp/src/limiter.rs`
- `crates/headroom-dsp/src/oversample.rs`
- `crates/headroom-dsp/src/sliding_max.rs`
- `crates/headroom-dsp/src/delay.rs`

### Refactor Item

Keep the public limiter interface stable while moving internals into private
modules:

- hard tier state;
- soft tier state;
- oversampled path state;
- silence/idle drain state;
- tests around public limiter behavior.

### Work Plan

1. Move tests out of `limiter.rs` into a focused test module first.
2. Extract private tier structs only where they hide complexity without adding
   caller knowledge.
3. Keep `Limiter::process_frame` as the primary behavior seam.
4. Preserve the silence fast path benchmark coverage.

### Done When

- `limiter.rs` drops below the size threshold.
- Public DSP behavior and benchmarks remain stable.
- Tests assert limiter outputs and telemetry, not private tier internals.

## Phase 9: Documentation And Context

### Problem

The repo has good product architecture in `PLAN.md`, but no concise context map
for future architecture reviews or agents. Important domain terms are scattered
across `README.md`, `PLAN.md`, and code comments.

### Refactor Item

Add a short architecture context document that names core concepts and points
to owning modules.

Suggested concepts:

- processed route;
- bypass route;
- per-app level control;
- bus graph;
- preferred real sink;
- profile overlay;
- setting effect;
- graph reconciler;
- monitor state.

### Done When

- Future reviews can find domain language without reading the entire canonical
  plan.
- New modules from this refactor queue have clear ownership statements.

## Suggested Order

1. Phase 0: safety net and guardrails.
2. Phase 1: profile mutation effects.
3. Phase 2: typed settings.
4. Phase 3: graph reconciler.
5. Phase 4: split `RoutingState`.
6. Phase 7: typed event decode.
7. Phase 6: shared monitor state.
8. Phase 8: limiter decomposition.
9. Phase 5: Layer A decomposition.
10. Phase 9: context documentation.

The first four phases are the highest leverage. They address real behavioral
risk and remove the worst coordination knots before cosmetic decomposition.

## Open Questions

- Should typed settings remain string-key compatible only at IPC, or should CLI
  commands move to first-class typed setting names?
- Should the graph reconciler own Layer A passive links, or should Layer A own a
  separate desired-link set that the graph reconciler consumes?
- Should shared monitor state live in `headroom-ipc`, a new crate, or
  `headroom-client`?
- Should the architecture context document be `CONTEXT.md` at repo root or a
  section in `PLAN.md`?
