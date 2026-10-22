# Leave

Module ID: `people/leave`, crate `bilibi-people-leave`. A company's leave
types, effective-dated entitlement policies, the append-only balance ledger
and the balances derived from it; later slices add requests with approval
(PL2), year-end carry-forward (PL2) and the three pages (PL3, PL4). This
README is the owner contract; the Bilimbi source it ports is
[`leave/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/leave/docs/README.md).

This is slice PL1 of the port plan: migrations 001 to 003, the three
settings, and the facade for the leave year, request rules, types, policy
versions, `grant_entitlements`, `record_entry` and `balances`. PL2 adds
`submit_request`, `decide_request`, `cancel_request`, the self reads,
`carry_forward`, `carry_forward_skipped`, `enqueue_carry_forward` with the
`people-leave/carry-forward` worker, and `requested_type_ids`; PL5 exposes
`record_entry` as the agent operation `people.leave.entry.record`.

## Public API

Resolve `Leave` from the registry as `people/leave.leave`
(`bilibi_people_leave::SERVICE`) and call it with the sealed `Scope` you
were handed and an explicit platform company id. Company and employee
identity come from `people/workforce` and must be current: a stale or
unavailable workforce read is `NotCurrent`. Missing, sibling-company and
cross-tenant records are indistinguishable (`NotFound`).

| Function | Capability | Does |
| --- | --- | --- |
| `rules` | none | The month the company's leave year begins (`Rules`; `leave_year`, `year_range`) |
| `put_rules` | `people.leave.policies.manage` | Sets that month; `YearInUse` once the ledger has entries, `InvalidRules` outside 1 to 12 |
| `request_rules` | none | The working weekdays and backdating limit (`RequestRules`) |
| `put_request_rules` | `people.leave.policies.manage` | Saves them: one or more ISO weekdays 1 to 7 (stored distinct, sorted) and 0 to 366 days, else `InvalidRules` |
| `list_types` | none | The company's types, active first then by name (`LeaveType`) |
| `create_type` | `people.leave.policies.manage` | Adds an active type (`NewLeaveType`); refused values are `Invalid` field errors in Bilimbi's words |
| `set_type_status` | `people.leave.policies.manage` | `active` or `archived`, else `InvalidStatus` |
| `list_policies` | none | Every version, by type and newest first (`Policy`) |
| `add_policy` | `people.leave.policies.manage` | The next version of an active type (`NewPolicy`): `NotAfterLatest`, `GrantedAfterEffectiveDate`, `InvalidPolicy`, `NotFound` |
| `policy_on` | none | The version in force on a date, else `NotFound` |
| `grant_entitlements` | `people.leave.policies.manage` | Grants a leave year to every working employee (`GrantCounts`: granted, existing, closed); `InvalidYear` outside 1900 to 9998 |
| `record_entry` | `people.leave.policies.manage` | An `opening` or `adjustment` entry for one working employee (`NewEntry` → `LedgerEntry`): `EntryKeyConflict`, `YearClosed`, `InvalidEntry`, `NotFound` |
| `balances` | none | Per-type totals of one working employee and leave year (`Balance`), with `pending` and `available` |
| `entries` | none | That employee's entries of the year, latest date first |

- **Leave year.** The company-scoped Base Setting
  `people.leave.year_start_month` (default January) sets when each leave year
  starts. A leave year is labelled by the calendar year it starts in. It
  cannot change once the company has ledger entries, because each entry
  stores its leave year.
- **Request rules.** `people.leave.working_weekdays` (ISO weekday numbers,
  default Monday to Friday) and `people.leave.request_backdate_days` (default
  30, 0 to 366) hold the rules PL2's requests count dates by. All three
  settings are editable under Administration › System › Settings by an
  account holding `people.leave.policies.manage`.
- **Types.** A company's types have a unique lowercase code (1 to 40
  characters, `[a-z0-9][a-z0-9_-]*`), a name, a unit (`day` or `hour`), a
  paid flag and whether requests need an available balance
  (`balance_required`, default true). Archived types take no new policy
  versions, grants, requests or entries, but keep their history and balances.
- **Policy versions.** Each type has effective-dated entitlement versions. A
  new version must start after the latest one, which it closes on the previous
  day, and after every entitlement already granted for that type. Versions are
  never edited. An optional carry-forward cap is the most of a closing balance
  carried into the next leave year; without one the type does not carry
  forward. Entitlement and cap are 0 to 9999.99 in hundredths of the unit.
- **Ledger.** Entries are append-only, enforced by a database trigger;
  corrections are new entries. `grant_entitlements` gives every current
  workforce employee each active type's entitlement from the version in force
  on the first day of the leave year, once per employee, type and year, and
  records the policy version; it skips, and counts as `closed`, an employee and
  type whose year is already carried forward. `record_entry` writes `opening`
  and `adjustment` entries, idempotent by company, source and key; a
  conflicting replay is refused, and so are the module's own sources `policy`,
  `request` and `carry_forward`. Quantities are in the type's unit with two
  decimals (`rust_decimal`, bound to `numeric`). `balances` sums entries per
  type and also reports the quantity `pending` requests reserve and the
  `available` balance after that reservation.

## Authorization and writes

Every write authorizes the scope's signed-in actor for
`people.leave.policies.manage` when it runs, through
`bilibi_people_workforce::authorization` at the facade, so pages, the
carry-forward worker and any other adapter share one boundary. The reads
of types, policies, rules, balances and entries need no capability, as
Bilimbi's do (PL2's request reads add `people.leave.requests.approve` and
`people.leave.self.view`). A system scope cannot reach the helpers, a grant
revoked while a page stays open refuses the next call, and the performer is
the scope's actor, never an argument.

Every write runs in a `MutationBoundary` transaction that first holds the
company's writable lock (`organisation::require_writable`, locked), so an
archived company is `CompanyArchived` and a company's leave writes are
serialized; it then locks the rows it decides on (`FOR UPDATE` on the types,
on the latest policy), decides every refusal with plain reads, and only then
writes, returning each row as a `Change`. A grant run that adds nothing
rolls back and returns its counts. Refusals are typed `LeaveError`
variants carrying Bilimbi's refusal atoms, each displayed in the sentence
the Policies page shows; `Error::Forbidden`/`Error::Internal` never leave
the facade for a refusal.

Every employee column keys on the employment id
(`people_workforce_employments.id`), with a foreign key on
`(employee_id, company_id)`, so an entry can only name an employment of its
company; Bilimbi keyed on Core Employee, which Bilibi does not have.

## Schema

`people-leave-001-core` creates `people_leave_catalog_types`,
`people_leave_policies` and `people_leave_ledger_entries` with its
append-only trigger; `people-leave-002-applications` creates
`people_leave_applications`, `people_leave_application_dates` with the two
partial unique indexes on active morning and afternoon slots, and the
append-only `people_leave_request_events`; `people-leave-003-carry-forward-skips`
creates `people_leave_carry_forward_skips`. They are the end state of
Bilimbi's three migrations (`balance_required` and `carry_forward_cap`
folded into 001). Dates are bound and read as text (`$n::text::date`,
`to_char`), since the workspace's `tokio-postgres` has no date feature.

## Walkthrough

`fixtures/walkthrough` (`src/leave.rs`) grants `manager@example.test`
`people.leave.policies.manage` directly, then shows every operation above
beside its refusals: the leave year (unauthorized, month 13, in use), the
request rules, three types and the field errors, policy versions (not after
the latest, out of range, archived type, granted after the date, the closing
of the previous version), grants (first run, repeat, bad year, unauthorized,
a closed year), entries (replay, conflicting replay, reserved source, zero,
too fine, wrong type, archived type, another company's employee), balances
with a pending request standing in as one SQL row, and PostgreSQL refusing
an edited or deleted entry, a negative entitlement, an unknown entry type, a
second live request on the same morning slot and a cross-company employee.
