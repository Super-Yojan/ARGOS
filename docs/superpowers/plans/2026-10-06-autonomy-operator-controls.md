# ARGOS Autonomy Operator Controls Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans when implementation is requested. Track the checkboxes below. This document proposes implementation; no product code is changed by preparing it.

**Goal:** Deliver ARGOS #4 on native macOS and iOS with per-rover autonomy controls, fleet takeover/stop, keyboard teleop, supervised approval, and exportable operator session logs.

**Architecture:** Extend the existing Rust contract/state, Zenoh client, and UniFFI API. Shared SwiftUI renders authoritative Terra state and supplies operator intent; Terra owns planning, arbitration, and motor safety. A Rust session recorder writes operator events off the UI thread.

**Tech stack:** Rust, Zenoh, UniFFI, SwiftUI/MapKit; no Dioxus or Python.

**Spec:** https://github.com/Super-Yojan/ARGOS/issues/4. Depends on https://github.com/Super-Yojan/Terra/issues/17 and the Terra plan at `/Users/yojan/git/Terra/docs/superpowers/plans/2026-10-06-level-of-autonomy.md`. Build on native dashboard PR https://github.com/Super-Yojan/ARGOS/pull/5.

## Global constraints

- Initial platforms remain macOS 14+ and iOS 17+, using the current Apple project and shared SwiftUI sources.
- Expose Teleop, Assisted teleop, Waypoint, and Supervised only according to Terra's supported-level status. Never represent a submitted change as confirmed.
- Connected mode changes should appear within approximately 1 s. Missing status/acknowledgement produces pending/unknown feedback, not fabricated success.
- Takeover and emergency stop are required both per rover and fleet-wide. Keyboard teleop is required; gamepad support is optional follow-up.
- Keep requested/effective level, assigned experiment condition, command source, and safety reason distinct. Show unexpected external changes and their reasons.

## Review focus

- Lost key-up or app focus must stop transmission and send zero; Terra's lease provides the disconnected fallback.
- A fleet action may partially succeed; show per-rover results and never claim an unreachable rover stopped.
- Old acknowledgements/proposals after reconnect must not enable motion or approve superseded goals.
- Operator and rover monotonic clocks are unrelated; exported timestamps must retain clock provenance.
- Log write/export failure must be visible without blocking input or preventing emergency stop.

## Mission context

Design the operator view around completing a mission. For the initial disaster search-and-rescue scenario, display the objective, search boundaries, remaining budget, reported survivor locations, and objective progress using observable mission telemetry. Keep this visible while changing autonomy. Do not expose hidden survivor locations or simulator ground truth. Teleop supports precise movement, assisted mode supports obstacle maneuvering, waypoint supports transit, and supervised exploration supports search; frontier approval is not itself a completed search objective.

Record mission ID/scenario seed, phase and observable conditions with switches and operator actions. An optional reason can explain a switch after the action; it never delays takeover or emergency stop. Present safety holds separately from deliberate mode changes. Support both fixed assigned-level and adaptive trials; show deviations from the assigned condition without restricting emergency intervention. Mission completion and safety outcomes, alongside operator effort, determine which autonomy is sufficient under each condition.

## Behavior and shared contract

Use Terra's `/autonomy`, `/autonomy/status`, `/teleop`, `/goal/proposal`, `/goal/decision`, `/safety`, and `/experiment/status` contract. Freeze codecs and fixtures in Terra first; reuse identical fixtures in ARGOS tests. Existing goal/cancel payloads retain their encoding.

Takeover submits a tokenized teleop-level request that clears previous motion even if already in teleop. Clear local held keys immediately; require confirmed authority plus a fresh key press before sending nonzero twists. Do not automatically restore a held key after a mode transition/reconnect. Emergency stop is latched by Terra; reset is a separate guarded action and never resumes old intent.

Fleet controls snapshot the last-known roster at activation and fan out unique per-rover tokens under one parent action ID. Include stale members for stop/takeover attempts, show their freshness, and retain each result independently. A successful network publication is not a rover acknowledgement. Retry only idempotent requests with the original token; do not replay motion. There is no atomic fleet guarantee and no reset-all control in this scope.

Keyboard controls apply only to the selected rover with confirmed Teleop/Assisted authority and healthy connection/status. Hold W/S or up/down for forward/reverse and A/D or left/right for yaw, using configured speed caps. Opposite keys cancel that axis. Emit at 20 Hz while active; key release, focus loss, view disappearance, rover selection change, level change, e-stop, or disconnection clears keys and sends zero where connected. Do not capture movement keys while editing text. macOS receives native key events; iOS supports external keyboards and a hold-to-drive touch pad using the same intent path.

Supervised mode shows the current proposal's location, reason, expiry, and pending decision. Approve/reject use proposal ID and token; disable stale/superseded decisions. Map goal entry redirects through the existing goal topic. Expose pause/cancel and explicit resume according to Terra's contract; a proposal alone never authorizes motion.

Start an operator session when connecting; end/flush on explicit end or disconnect. Record local UTC and session-relative monotonic time, session UUID, rover IDs, sent requests, acknowledged/rejected results, observed external mode/safety changes, teleop start/end plus transmitted packets, goal/proposal actions, connection changes, and logging errors. Preserve command tokens; canonical teleop includes optional session UUID and packet sequence. Record `/experiment/status` run IDs/time samples and retain uncertainty when aligning clocks. Join requests to rover receipts by tokens/sequence; do not count both as separate interventions. Export a versioned JSONL file with a session manifest through native save/share UI. Mark incomplete logs explicitly; emergency controls remain usable after recording failure.

## Task 1: Contract and authoritative state

**Files:** Modify `crates/argos-core/src/{contract,state,lib}.rs`; create `crates/argos-core/src/autonomy.rs` and `crates/argos-core/tests/autonomy.rs`; extend existing contract tests.

**Interfaces:** `AutonomyStatus`, `GoalProposal`, `ExperimentStatus`, `OperatorAction`, and `ActionReceipt` own wire values. `FleetState::apply_autonomy(rover_id, status, received_at)` accepts authoritative state; `FleetState::apply_proposal(...)` replaces proposals by ID/revision. Snapshots include capability, freshness, pending actions, and confirmed state.

- [ ] Add failing fixtures/tests for four levels, null effective level during hold, accepted/rejected tokens, unsupported capability, out-of-order revisions, proposal expiry/replacement, and run identity changes.
- [ ] Implement bounded validation and state updates with Terra's finalized schema; retain requested and confirmed state independently.
- [ ] Run `cargo test -p argos-core`; require all codec/state fixtures to pass and commit the contract change.

## Task 2: Transport and per-rover/fleet actions

**Files:** Modify `crates/argos-zenoh/src/lib.rs`; create `crates/argos-zenoh/src/operator.rs`; extend `crates/argos-zenoh/tests/loopback.rs`.

**Interfaces:** `submit_action(rover_id, OperatorAction) -> Result<ActionReceipt, Error>` returns local submission state; authoritative completion comes from subscribed status. `submit_fleet_action(OperatorAction) -> Vec<ActionReceipt>` snapshots targets. `send_teleop(rover_id, twist, session_id, sequence)` sends leased intent.

- [ ] Write failing loopback tests for immediate acknowledgements, publication without acknowledgement, rejected mode, stale member fan-out, partial fleet success, duplicate-token retry, reconnect, and per-rover isolation.
- [ ] Subscribe to autonomy/proposal/experiment state; publish operator intents. Expire pending feedback after a configurable 2 s acknowledgement timeout while accepting valid later receipts.
- [ ] Run `cargo test -p argos-zenoh`; verify fleet failures remain separate and motion is never replayed; commit.

## Task 3: Held-input lifecycle and UniFFI

**Files:** Create `crates/argos-core/src/teleop.rs` and `crates/argos-core/tests/teleop.rs`; modify `crates/argos-ffi/src/lib.rs` and `crates/argos-ffi/tests/api.rs`.

**Interfaces:** `TeleopIntent::press/release`, `TeleopIntent::clear`, and `TeleopIntent::sample` produce bounded twists. Export `set_level`, `take_over`, `take_over_fleet`, `emergency_stop`, `emergency_stop_fleet`, `reset_stop`, `decide_proposal`, `resume_supervision`, and held-input methods through the existing client.

- [ ] Test opposing keys, speed bounds, stale authority, selection change, release/focus loss, reconnect, and transition to teleop requiring a new press.
- [ ] Implement shared input state and a 20 Hz sender lifecycle; stopping clears intent and requests zero without waiting for the next ordinary tick.
- [ ] Run `cargo test -p argos-core -p argos-ffi`, regenerate Apple bindings/XCFramework using existing scripts, and commit.

## Task 4: Operator session recorder and export

**Files:** Create `crates/argos-core/src/operator_events.rs` and tests; create `crates/argos-zenoh/src/session_log.rs` and recorder tests; extend FFI session/export methods.

**Interfaces:** `OperatorSessionManifest`, `OperatorEvent`, `SessionRecorder`, `begin_session(path)`, `end_session()`, and `export_session(destination)` use owned versioned records. Native UI supplies an app-sandbox path; Rust owns bounded off-thread writing.

- [ ] Test command-token and teleop-sequence joins against Terra fixtures, differing clock origins, missing run IDs, external changes, duplicate acknowledgements, and incomplete logs.
- [ ] Implement bounded recording, explicit flush/close, and consistent export snapshots; record backpressure/I/O failure as incomplete without blocking emergency actions.
- [ ] Test unwritable destination, full queue, partial final line, export during recording, and final flush; run Rust recorder/API tests and commit.

## Task 5: Native dashboard controls

**Files:** Modify `apps/apple/Shared/{FleetModel,FleetView,RoverDetailView}.swift`; create `AutonomyControls.swift`, `TeleopControls.swift`, and `SessionLogView.swift`, and `MissionView.swift`. Extend native unit/UI tests in the existing Apple project.

- [ ] Add model tests for pending versus confirmed mode, external change reason, partial fleet results, stale proposal, and unavailable log export.
- [ ] Add mission objective/progress/constraints from observable telemetry, plus mode picker, authority/safety display, per-rover and fleet takeover/stop, guarded reset, proposal approve/reject/resume, and session export.
- [ ] Connect native keyboard/touch lifecycle to Task 3; text editing never drives a rover and leaving the view clears intent.
- [ ] Run macOS/iOS native tests and build both targets. Exercise keyboard/focus handling on macOS and external-keyboard/touch behavior on iOS; record any host/device limitations and commit.

## Task 6: Linked acceptance evidence

- [ ] Run Terra's simulator with at least two rovers; select each operational level and measure request-to-displayed-status latency, targeting approximately 1 s on a healthy local connection.
- [ ] Verify individual/fleet takeover and emergency stop, including one unreachable target; inspect rover logs for cleared intent and latched stops.
- [ ] Demonstrate keyboard motion, release-to-stop, focus loss, and disconnect lease expiry; verify no old key resumes after takeover or reconnect.
- [ ] Demonstrate frontier proposal -> approval -> motion and reject/redirect; confirm obsolete approvals cannot launch motion.
- [ ] Complete a seeded search-and-rescue mission in fixed-level and adaptive trial flows; verify mission context persists while switching and no hidden targets appear. Record switch phase/conditions and optional reason without delaying control.
- [ ] Export an operator session and join its events to Terra run logs by tokens/session/sequence. Verify aligned timestamps retain clock uncertainty and incomplete sessions are identified.
- [ ] Save screenshots, logs, and a requirements-to-evidence table under `docs/evidence`; update README and link both issues in the ARGOS PR. Do not infer physical-device validation from simulator builds.

## Delivery order

Finalize Terra contract first. Develop ARGOS contract/transport/logging against loopback fixtures while Terra implements the shared runtime. Integrate native controls once authoritative status is available, then collect linked evidence. Keep Terra implementation and ARGOS controls in separate PRs; the existing dashboard PR #5 remains the foundation. Gamepad support and continuous autonomy blending are follow-ups.
