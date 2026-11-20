# Test rules for the enzimOS workspace

Applies to authored code in this workspace and its owned repositories under
`repos/`. Framework and vendored upstream code remain under their owners' control.
Owned shell/compositor integration and enzwall follow these rules too. The aim is a small, useful suite that survives implementation changes.
Test count and coverage percentage are not quality targets.

## Default: extend coverage without growing the suite

- A fix, refactor, new helper or changed file does **not** automatically need a
  new test. Documentation, formatting and reversible cosmetic changes normally
  need none. Compiler and type checks already protect many refactors.
- First find the existing test that owns the behavior. Update that test when the
  contract changes; extend it with a relevant case when the invariant is the same.
- A new independent test is justified only for a new, durable failure boundary
  or an important user workflow that existing coverage cannot exercise.
- Aim for one representative workflow and one focused boundary test per behavior,
  rather than a test for each layer, message, field or failure variant. Extra cases
  need a distinct failure that those tests would miss; this is a review threshold,
  not a reason to delete a unique safety check to meet a quota.
- In the change description, state the failure being protected and why an existing
  test cannot cover it. For suite growth, identify what was reused or retired, or
  give the concrete reason a net increase is necessary. No separate evidence file
  or test-registration ledger is required.

## Admission rule: every retained test must earn its place

A test must satisfy all four conditions:

1. **Consequence:** failure would cause a meaningful product problem: data loss,
   unauthorized execution or access, secret exposure, wrong ownership/scope,
   unbounded work, broken persistence, or failure of a core user workflow.
2. **Ownership:** the assertion checks our behavior at a boundary we control,
   rather than rechecking a dependency, the test harness, or a language feature.
3. **Distinct value:** no retained test already exercises the same invariant
   through an equal or stronger path. Different names or layers are insufficient.
4. **Credible observation:** breaking the relevant production behavior would make
   the test fail for the right reason. Assertions are tied to an observable effect
   or an explicit external contract, rather than setup assumptions or mock scripts.

A focused unit test can qualify for an important algorithm, parser, resource bound
or safety primitive when a broader workflow cannot isolate that behavior reliably.
Pure validation alone is not automatically sufficient: internal record/reply codecs
and phase variants still need a distinct consequential failure and an owner.

## What to keep

- Public API/protocol admission and authorization, including argument and path
  injection, exact account/session/resource scope, and hostile input bounds.
- Real file integrity and persistence boundaries: atomic publication, interrupted
  writes, private secrets, replacement refusal, and preservation of unrelated data.
- Process lifetime, cancellation, stale results, bounded queues and exact ownership
  where failure could execute work twice, affect another owner, or leak resources.
- Durable install/update/backup/recovery workflows and representative ordinary
  user workflows that demonstrate useful results through production entry points.
- Security-sensitive shipped configuration checks when the actual configuration
  is the boundary and the check protects a concrete permission or isolation rule.
  Searching implementation source for strings does not establish that rule.
- Valid-input controls alongside refusals. A boundary test must demonstrate that
  legitimate work succeeds as well as that hostile or stale work is refused.

## What not to add or keep

- A permanent test for every bug, issue number, branch, helper or edited function.
  A regression case must pass the same admission rule as any other test.
- Source scans, banned symbol names, source-file counts, checks of code spelling,
  or assertions that an old implementation disappeared.
- Enum inequality, trivial getters/setters, constants compared with their own
  formulas, framework defaults, serialization smoke tests or fixture self-checks.
- Exhaustive internal JSON-field/phase matrices, per-role command-factory copies,
  or near-identical stale/replay/replacement cases repeated at every wrapper.
  Keep distinct public boundaries and shared primitive coverage instead.
- Assertions limited to mocked call counts or scripted outcomes when the meaningful
  effect—written bytes, spawned argv, authorization or persistence—is unobserved.
- Exact prose, colours, pixels, row counts, coordinates or responsive layout variants
  for cosmetic details. Exceptions need a concrete accessibility, security or core
  interaction failure that the representative workflow does not detect.
- Timing/RSS microbenchmarks in normal unit suites, arbitrary sleeps, or repeated
  performance cases for implementation caches. Measure performance separately;
  retain deterministic liveness/resource limits when they protect product behavior.
- Another language's copy of the same test unless it proves an independent external
  interoperability contract. Do not copy framework/dependency tests into app repos.

## Keep tests and their fixtures small

- Group related input variants under one invariant with clear case labels; do not
  create a standalone test for each equivalent spelling, error or input field.
- Count the actual scenarios and maintenance burden too. Combining hundreds of
  unrelated assertions into one huge test is not suite reduction.
- Reuse small fixtures. Do not build a second implementation in the expected-value
  code, hide unrelated contracts in a shared scenario, or create one-off frameworks.
- Remove orphaned fixtures, counters, source walkers, snapshots and test-only hooks
  when their owning test goes. Preserve helpers still needed by retained checks.
- Keep boundary positive controls and useful failure diagnostics. Do not weaken
  assertions, production checks or authorization merely to make a suite pass.

## Pruning and verification

During relevant maintenance, review the tests being touched and their nearest
neighbors. Retire obsolete contracts, duplicate invariants and checks that fail the
admission rule. Remove the test code; do not merely rename, ignore or relocate it.
Record the reason briefly in the change description.

A failing test is not evidence that it is unnecessary. If it exposes a real defect,
keep it and report the defect. Clearly distinguish an existing reproduced failure
from a new regression; never advertise a green suite when retained checks fail.

Batch edits, then run the changed repository's applicable unit/integration checks
using existing caches and its resource limits. Prefer the cheapest check that
answers the question; broaden after failures, shared API changes or unresolved
concerns. A test-only edit does not justify a full OS build or ISO/VM qualification.
Run required repository checks before pushing. Do not rebuild unchanged dependencies
or repeat successful checks without a new reason.

Use private temporary files, private buses, owned processes and local fixture
servers. Tests must not modify host accounts, services, policy, disks or credentials,
make paid calls, or depend on external accounts. Privileged installed-system checks
belong in explicitly scoped disposable VMs and are separate from the unit suite.

## Examples

| Proposed check | Decision |
| --- | --- |
| A hostile app ID never reaches an install executor, while a valid reviewed ID does | Keep: observable admission boundary with a positive control |
| A cancelled operation kills its own child and leaves a peer alive | Keep: process ownership and cancellation |
| A second test checks a wrapper's serialized fields already rejected by the public request test | Remove: duplicate internal-layer coverage |
| A getter returns the fixture value just assigned to it | Remove: no consequential independent behavior |
| A new UI spacing fix gets three screenshots at nearby widths | Do not add: cosmetic variants |
| A retained archive test detects hidden duplicate package metadata | Keep even while failing: it identifies a real admission defect |
