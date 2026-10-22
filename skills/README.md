# Skills

Module ID: `people/skills`, crate `bilibi-people-skills`. A company's skill
catalog, its versioned proficiency scales, its versioned requirement
profiles and the policy values that steer assessments. This README is the
owner contract; the Bilimbi source it ports is
[`skills/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/skills/docs/README.md).

The catalog and assessment slices (PK1–PK2) include both migrations, the seven policy settings, evidence-backed assessments, independent decisions, current scores and standing reads. Migration 002 also creates the reassessment, action and reminder tables from the same Bilimbi migration. Their writes follow in PK3 and PK7; the pages
(`/people/skills`, `/people/skills/profiles/{id}`, `/assessments`,
`/actions`, `/my`, `/policy`) and the menu leaves land in PK4–PK6.

## Public API

Resolve `Skills` from the registry as `people/skills.skills`
(`bilibi_people_skills::SERVICE`) and call it with the sealed `Scope` you
were handed and an explicit platform company id. Every function first reads
the company through Workforce (`Workforce::company`) and requires the read to
be current: a missing, archived, sibling-tenant or malformed company id is
`NotFound`, a stale or unavailable read `NotCurrent`. Results are typed
records; the schema is private. Reads need no capability beyond seeing the
company, as Bilimbi's do; every write names the capability it authorizes for
the scope's signed-in actor when it runs.

| Function | Capability | Does |
| --- | --- | --- |
| `list_categories`, `list_skills` | none | The company's categories and skills, active first, then by name |
| `create_category`, `update_category`, `set_category_active` | `people.skills.catalog.manage` | A category has a lowercase code unique in the company (stable) and a name; deactivation is refused while it has active skills (`CategoryInUse`) |
| `create_skill`, `update_skill`, `set_skill_active` | `catalog.manage` | A skill has a stable code, a name, a definition, an optional evidence guide, a critical flag and an optional reassessment interval (1–120 months) under an active category of the company (`CategoryUnavailable`); a revision never changes the code |
| `list_scales` | none | Scale versions with their levels, by code, newest first |
| `create_scale`, `new_scale_version`, `rename_scale`, `put_scale_level`, `delete_scale_level`, `discard_scale` | `catalog.manage` | Version 1 of a new code starts as a draft without levels (`CodeExists`); the next version copies a chosen version's levels (`DraftExists` while one is open); only a draft takes levels (`NotDraft`), keyed by number 0–20; a draft no profile uses can be discarded (`ScaleInUse`) |
| `publish_scale`, `retire_scale` | `catalog.manage` | Publishing needs at least two levels numbered from 0 without gaps and with distinct names (`InvalidLevels`) and retires the code's previously published version in the same write; retiring needs a published version (`NotPublished`) |
| `list_profiles`, `get_profile` | none | Profile versions by code, newest first; one version with its ordered items and selectors |
| `create_profile`, `update_profile`, `new_profile_version`, `discard_profile` | `catalog.manage` | Version 1 starts as a draft against a published scale (`ScaleUnavailable`, `CodeExists`); a draft can be renamed or moved to another published scale once every required level exists there (`LevelNotOnScale`); the next version copies items and selectors and adopts the scale code's published version when it holds every required level, else keeps the source scale and reports `missing_levels` |
| `put_item`, `remove_item`, `move_item` | `catalog.manage` | A draft's requirements: an active skill (`SkillUnavailable`), a level on the profile's scale (`LevelNotOnScale`), a criticality, a weight (0–100, two decimals), a mandatory flag; one per skill, in a closed sequence |
| `add_selector`, `remove_selector` | `catalog.manage` | A draft targets the whole company or positions the Workforce position scan exposes (`PositionUnavailable`), never both (`MixedSelectors`), each once (`DuplicateSelector`) |
| `publish_profile` | `people.skills.profiles.publish` | From an effective date: at least one requirement and one target, weights totalling 100, active skills, a published scale with every level, positions still exposed; after the code's latest version (`NotAfterLatest`), which it retires the day before; no other code may target the same people from that date (`OverlappingProfile`) |
| `retire_profile` | `profiles.publish` | Records the last effective day of a published version (`InvalidDate` before its start) |
| `requirements` | none | The single published or retired version in force on a day for the company, optionally for one position; `None` when none, `Ambiguous` when more than one |
| `position_options`, `position_of` | none | The positions a profile may target today, and the position an employee substantively holds on a day, through the Workforce position scan |
| `policy`, `put_policy` | none / `people.skills.policy.manage` | The seven policy values; a write gives at least one whole number within its range (`InvalidPolicy`) and keeps the others |

Refusals are `SkillsError` variants carrying the sentences Bilimbi's pages
show (`profile_live.ex`, `catalog_live.ex`, `web/support.ex`); refused
field values are `Invalid(Vec<FieldError>)` in Ecto's words ("can't be
blank", "should be at most 80 character(s)", "has invalid format", "has
already been taken", "has at most two decimals", ...). A denied or unknown
capability, or a company outside the actor's reach, is `Unauthorized`.
Timestamps are UTC wall clocks (`jiff::civil::DateTime`); dates are
`jiff::civil::Date`; weights are `rust_decimal::Decimal`.

## Authorization and writes

Every write authorizes through the Workforce `Authorization` helpers
(`bilibi_people_workforce::authorization::authorize`) with the scope it was
handed, so a grant revoked while a page stays connected refuses the very
next operation, and a system principal cannot reach the facade at all. It
then begins a `MutationBoundary` transaction and holds the company's writable
lock (`organisation::require_writable(.., lock = true)`) first: an archived
company is `CompanyArchived`, and a company's writes are serialized, which is
what keeps "one draft and one published version per code" and the overlap
rule true under concurrent publishers. Refusals are decided with `tx.sql()`
before the write; every affected row is reported as a `Change`
(`people_skill_category`, `people_skill`, `people_skill_scale`,
`people_skill_scale_level`, `people_skill_profile`,
`people_skill_profile_item`, `people_skill_profile_selector`), an empty
object on the absent side, so audit names it `created` or `deleted`. A
write that would change nothing (deactivating an inactive category, moving
the first requirement up) rolls back and answers the current record.

No capability is granted to a system role: an administrator grants the
seventeen keys, split across roles where duties need separate people. The
Authz domain is `people`; `submit`, `review`, `approve` and `send` are the
verbs this module declares because Base does not.

| Capability | Used by |
| --- | --- |
| `people.skills.catalog.view` | `/people/skills` and each profile page (PK4) |
| `people.skills.catalog.manage` | catalog, scale, profile-draft and (PK3) action-type writes |
| `people.skills.profiles.publish` | `publish_profile`, `retire_profile` |
| `people.skills.assessments.view`, `.submit`, `.review`, `.approve`, `.manage` | PK2 |
| `people.skills.reassessments.submit`, `.execute` | PK3 |
| `people.skills.actions.view`, `.update`, `.manage`, `.approve` | PK3 |
| `people.skills.self.view` | My skills (PK5) |
| `people.skills.reminders.send` | PK7 |
| `people.skills.policy.manage` | `put_policy`, the Skills policy page (PK6), the Settings editor |

## Positions

A profile targets the company or positions. Positions come from Workforce's
position scan (Bilimbi `Workforce.positions/4`, served by Organisation
through the `people/organisation.positions` port when it is mounted):
`position_options` lists what the scan exposes today (pages of 100, at most
20 pages), `add_selector` and `publish_profile` refuse a position the scan
does not expose, and `position_of` finds the position an employee
substantively holds. Without Organisation the scan is unavailable, only
company targets are possible and nobody holds a position.

Both reads use `Workforce::positions` and require every page to be current. They follow the returned cursor for at most 20 pages of 100 positions. Options use today in UTC; `position_of` uses the requested date and ignores acting assignments. Without Organisation, options are `PositionUnavailable` and `position_of` is `None`. Other holder-read failures remain typed refusals.

## Policy

Seven company-scoped Base Settings (`define`d in `src/contributions.rs`,
editor group "People", capability `people.skills.policy.manage`), each a
whole number with Bilimbi's default and range:

| Key | Default | Range | Label |
| --- | --- | --- | --- |
| `people.skills.reassessment_due_days` | 30 | 1–365 | Reassessment request window |
| `people.skills.default_reassessment_months` | 12 | 1–120 | Default reassessment interval |
| `people.skills.reminder_window_days` | 30 | 0–365 | Validity reminder window |
| `people.skills.backup_minimum` | 2 | 1–100 | Critical-skill backup minimum |
| `people.skills.priority_multiplier_critical` | 3 | 0–100 | Critical priority multiplier |
| `people.skills.priority_multiplier_essential` | 2 | 0–100 | Essential priority multiplier |
| `people.skills.priority_multiplier_development` | 1 | 0–100 | Development priority multiplier |

`policy` reads them as one `Policy` (`multiplier(criticality)` picks the
multiplier a requirement's gap is weighted by); `put_policy` writes the given
ones through Base Settings on the write's own transaction. Historical
assessments and actions (PK2, PK3) snapshot the value they used. Until the
policy page lands (PK6), holders of the capability edit them per company in
the Settings editor.

## Database

`migrations/001-catalog.sql`, applied as `people-skills-001-catalog`, is the
end state of Bilimbi's `20260930181101_create_people_skills_catalog`:

| Table | Holds |
| --- | --- |
| `people_skill_categories` | code (80, unique per company), name (160), description (2000), active |
| `people_skills` | category, code (80, unique per company), name, definition (2000), evidence guide, critical, reassessment months (1–120), active |
| `people_skill_scales` | code, name, version (unique per company and code), status (`draft`, `published`, `retired`; one draft and one published per company and code), published/retired at, actor |
| `people_skill_scale_levels` | scale, level (0–20, unique per scale), name (100), anchor, authority (2000) |
| `people_skill_profiles` | code, name, version, status (as scales), scale, effective from/to (to ≥ from; a published version is dated), published/retired at, actor |
| `people_skill_profile_items` | profile, skill (unique per profile), sequence (unique per profile), required level, criticality (`critical`, `essential`, `development`), weight (numeric(5,2), 0–100), mandatory, evidence standard |
| `people_skill_profile_selectors` | profile, selector type (`company` with no position, or `position` with one), unique per profile |

Five guard functions and six triggers refuse what the facade never does: a
changed skill code or owner (`people_skills_guard`); any edit to a published
or retired scale or profile but the one step from `published` to `retired`
with its dates (`people_skill_scales_guard`, `people_skill_profiles_guard`);
and any insert, update or delete of a level, item or selector whose parent
is not a draft (`people_skill_scale_levels_guard`,
`people_skill_profile_children_guard`). Every table carries `tenant_id` and
`company_id`; `inserted_at` and `updated_at` are UTC `timestamp(0)`. Dates
and timestamps are bound and read as text (`$n::text::date`, `to_char`):
the workspace's `tokio-postgres` has no date feature, and a Domain must not
turn one on.

## Walkthrough

The repository's `fixtures/walkthrough` (see the root README) ends with the
Skills block: the admin grants the manager the two catalog keys directly and
keeps publish and policy; the manager builds the category `technical`, the
skills `welding` and `forklift`, the scale `proficiency` (two versions) and
the profile `operator`, meeting every refusal on the way (a bad code, a
duplicate code, a level gap, duplicate level names, a level the scale lacks,
a weight with three decimals, a position target while Organisation is
unmounted, a company target twice); the admin publishes `operator` from
2026-01-01, is refused `supervisor` over it (`OverlappingProfile`), publishes
`operator` version 2 from 2026-07-01 (version 1 ends 2026-06-30) and retires
it on 2026-12-31; `requirements` answers version 1 on 2026-06-30, version 2
on 2026-07-01 and nothing on 2027-01-01; the policy write is refused for the
manager and out of range for the admin; and PostgreSQL refuses a changed
skill code, an edit, a level or a deletion on a published or retired scale
or profile.

## Dependencies

`base/authz`, `base/database`, `base/module_registry`, `base/party_ids`,
`base/settings`, `base/tenancy`, `core/company`, `people/settings`,
`people/workforce`; the Cargo manifest lists the same edges. PK7 adds
`base/queue` for the reminder worker.

## Assessments and standing (PK2)

An assessor records evidence and a level. Another person verifies the assessment and finalizes it. Finalization updates the employee's score. **A correction is a new assessment**, so the original evidence and decisions remain available.

For example, submit an assessment with an `AssessmentInput`, then pass its returned `id` to `review_assessment(..., ReviewDecision::Verify, None)` and `finalize_assessment` using an independent person's scope. To return it, use `ReviewDecision::Return` and a note. To correct returned or finalized work, submit new evidence with `supersedes_assessment_id` set to its id.

The scope always supplies the assessor or decision maker. No caller can supply another account id or bypass reach. Capability keys below have the prefix `people.skills.`.

| Function | Capability | Result and limits |
| --- | --- | --- |
| `submit_assessment(scope, company, &AssessmentInput)` | `assessments.submit` | `Assessment`: a working employee reached through the supervisor chain, or anyone with `assessments.manage`; never the signed-in employee |
| `review_assessment(scope, company, id, decision, note)` | `assessments.review` | Pending work within the same reach becomes verified or returned; a return requires a note |
| `finalize_assessment(scope, company, id)` | `assessments.approve` | Verified work becomes finalized and refreshes its subject's score; this capability has company-wide reach, as Bilimbi's does |
| `assessable` | `assessments.submit` or `reassessments.submit` | Working employees within reach, sorted by name, excluding the signed-in employee |
| `list_assessments(scope, company, &AssessmentFilters)` | `assessments.view` | Newest 200 within reach, optionally filtered by status, employee or skill; each `AssessmentView` includes `assessment`, `employee_name`, `skill_code` and `skill_name` |
| `assessment_queue` | Any assessment view, submit, review or approve key | Oldest 200 per bucket: independent pending reviews within reach, independent verified work company-wide, and the actor's uncorrected returned submissions; review and finalize buckets require their respective grants |
| `assessment_decisions` | `assessments.view` | The append-only trail in id order, only within reach |
| `gaps` | `assessments.view` | Up to 500 scores within reach with a positive gap, ordered by mandatory first, descending priority, next due date and id |
| `standing` | `self.view` | The signed-in working employee's scores (up to 500), actions other than proposed or cancelled (up to 500), and pending reassessment requests; re-proves the account link each call |
| `coverage(scope, company, as_of)` | `assessments.view` and `assessments.manage` | Every active critical skill, its competent working holder count, the backup minimum and whether that minimum is met |

**Neither the assessor nor the assessed employee may review or finalize the work.** The reviewer may also finalize it. Subject independence uses the account's current link, including an employee who has since stopped working. Workforce supplies `linked_employee`, `lock_employee_subject` and `lock_working_employee` so Skills can check this inside its company transaction without reading private Workforce tables. The lock order is Company → Employment → Assessment; the company lock also serializes employment changes and account relinking.

### Submission snapshots and retries

`AssessmentInput` accepts employee, skill, assessed level, evidence, optional assessment date (today in UTC when omitted), optional validity date, method, notes, request key and optional predecessor. Text is trimmed: evidence is required and at most 4,000 characters, request key required and at most 80, method at most 80, notes at most 2,000. An assessment cannot be in the future, and validity cannot precede it. The level must exist on the pinned scale.

On the assessment date, Skills resolves the employee's substantive position and the one requirement profile that covers them. It snapshots the profile id, scale id, required level, criticality, weight and mandatory flag. No matching skill requirement, or ambiguous profiles, is `NoRequirement`. The gap is `max(required − assessed, 0)`. Priority is the gap multiplied by the company's policy value for that criticality. Zero gap is `meets` or `exceeds`; gaps of one, two or more are `minor_gap`, `major_gap` or `critical_gap`.

The next review date is the explicit validity date, otherwise the assessment date plus the skill's interval or the policy default. Adding months clamps a month-end day to the target month's last day. Later catalog and policy changes do not change these snapshots.

A request key is unique per company and assessor. A retry with the same employee, skill, level, assessment date and evidence returns the existing assessment with its current decision state and writes nothing. A change to any of those five facts is `KeyConflict`. Optional metadata follows Bilimbi's retry comparison and does not change the original row.

### Corrections and current scores

A predecessor must be returned or finalized work for the same company, employee and skill. Only its original assessor can correct returned work. Each assessment has at most one successor, including a successor still pending review.

Finalization excludes every ancestor of every finalized correction, including returned assessments in the chain. From the remaining finalized assessments it chooses the newest assessment date, then the newest id. A backdated correction can therefore replace its newer predecessor. A pending or returned correction leaves the score alone. The score copies its chosen assessment exactly and carries its original profile version and scale in `ScoreView`.

Standing reports `expired` after the validity date, `overdue` on or after the next review date, and `current` otherwise. Coverage counts scores at or above their own snapshotted requirement with validity on or after `as_of` (or no validity), and only for employees currently working. A due date alone does not expire coverage.

### Database and proof

`people-skills-002-assessments` creates assessments, decisions, scores, reassessment requests, action types, actions, action events and reminders. It preserves Bilimbi's constraints and guard triggers: immutable assessment facts, the allowed review/finalize transitions, independent assessor checks, one correction per predecessor, append-only decisions and action events, scores that mirror finalized work, and terminal request/action/reminder guards. The score guard also checks tenant and subject identity. All writes report their assessment, decision and score changes through `MutationBoundary` in the same transaction.

The PK2 block in `fixtures/walkthrough/src/assessments.rs` creates records through the facades and stops on an unexpected result. Run the People walkthrough on a fresh disposable database to exercise submissions, five-fact retries, transitive supervisor reach, self-assessment and decision refusals, snapshot policy, correction chains, score ordering, expired standing, coverage, grant revocation and account unlinking. It also attempts forbidden SQL updates and deletes to prove the guards. Assessment and My skills pages belong to PK5; PK2 adds no route or menu leaf.
