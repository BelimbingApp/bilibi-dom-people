# Progression

Progression records company policy versions and explains whether the signed-in employee meets their criteria.

This crate ports `Bilimbi.People.Progression`. Pages and navigation follow in PG2. It installs no policy rows and makes no promotion, employment, position or pay decision.

## Record and publish a policy

Use the `Progression` service (`people/progression.progression`) with the caller's sealed scope and an explicit company. `draft` takes `DraftInput`: a code, positive version, name, ISO effective date, optional `ProfilePin`, and optional `PerformanceRule`. At least one criterion is required.

For example, pin a published Skills profile's ID and version. The service copies all its skill IDs and required levels into immutable `rules.competence`; callers cannot weaken a copied threshold. Each criterion keeps the skill’s company-defined code. Performance criteria declare one or more periods, accepted outcome values, and whether missing evidence means `unknown` or `not_met`. These values are company data, with no installed defaults.

`publish` revalidates the exact published profile and copied requirements. For the same policy code, its version must exceed all published versions and its effective date must be on or after every published effective date. Equal dates permit a same-day correction. Future versions leave today's policy in force until their start date.

Drafting and publishing require `people.progression.policy.manage`. Both refuse impersonation, hold the writable company lock, recheck authorization, and record every changed row through Base Audit's mutation boundary. PostgreSQL refuses content edits or deletion of any version and every further update to a published version. Correct a draft by recording another version.

## Read history and standing

`policies` requires `people.progression.policy.view` and returns `PolicyPage`. `PageOptions` accepts positive pages and sizes 10, 25, 50 or 100; oversized pages clamp to the last page. Rows sort by effective date, then ID, descending.

`explain` requires `people.progression.self.view` and resolves the account's current working employment on every call. It accepts no employee selector. Each policy code independently uses its latest effective publication on or before the current UTC date, with the highest version winning a same-day tie. No effective policy returns `NoPublishedPolicy`.

Competence uses the Skills standing API and requires `people.skills.self.view`. A score counts only when current and matched to the pinned profile ID, profile version and scale. Missing, overdue, expired or differently profiled evidence is `unknown`. Retiring the pinned profile does not rewrite the policy or its assessment provenance.

Performance uses every page of the employee's released records through `Performance::my_records` and requires `people.performance.self.view`. A review must fit entirely within a declared period. The latest period end wins, followed by version and ID. Results retain review ID, version and supersession provenance; draft reviews never count. Missing reviews follow the policy's explicit missing-evidence rule.

Each explanation returns its policy, individual criteria, and combined `Status`: any `not_met` wins, otherwise any `unknown` wins, otherwise `met`. Evidence owners retain their own permission checks. An unavailable employee, stale workforce or denied evidence read returns a typed refusal.

## Storage and verification

Migration `people-progression-001-progression` owns `people_progression_policy_versions`, its identity/scope indexes, workflow checks and immutability guard. Dates bind and read as text; publication times are UTC. SQL and validation remain private to this crate.

Run the disposable proof with `PGPASSFILE` and `DATABASE_URL` set:

```sh
BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin progression
```

See [Skills](../skills/README.md) and [Performance](../performance/README.md) for the evidence contracts, and the Platform's `scripts/domain-ci.sh` for mounted validation.
