# Leave

Leave manages a company's leave balances, requests, approvals and year-end carry-forward.

Module ID: `people/leave`, crate `bilibi-people-leave`. This README is the owner contract. The parity reference is [Bilimbi Leave](https://github.com/BelimbingApp/b-dom-people/blob/main/leave/docs/README.md). PL1 provides the schema, policies and ledger; PL2 provides requests and carry-forward; PL3 provides My leave and Leave approvals; PL5 provides the reviewed entry operation. PL4 provides the Leave policies page.

## Pages

Open **People › My work › My leave** (`/people/leave/my`) to see your current leave year, balances, pending reservations and available quantities. Choose an active leave type, dates and duration, then submit your request. Leave “To” empty for one date. Morning, afternoon and hour requests cover a single date. My requests includes decision notes and cancellation; cancellation asks for confirmation and is available for pending requests or approved leave that has not started. Balance history shows the year's ledger entries.

Open **People › Team › Leave approvals** (`/people/leave/requests`) to choose an active company and decide its earliest 200 pending requests. Enter a note, choose Approve or Reject, and confirm the decision. Rejection requires a note. You cannot decide your own leave request. The selected company is kept in the address as `company_id`.

The four lists use shared FlexTables with column settings and compact rows. Bilimbi does not sort, filter or paginate these bounded lists. Request and cancellation events recheck `people.leave.self.view`; decisions recheck `people.leave.requests.approve` for the selected company. The facade checks again during the write. Every read resolves the account's current employee link; an unlinked account sees the unavailable state. Request retry keys and pending confirmations live only in the server's mounted page state. A confirmation never takes its target from the confirmation payload.

For a fresh disposable database, run `BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin leave-pages`. It installs the Platform fixture and creates linked employments, day and hour leave types, opening balances and a pending request through the owning services. Sign in as `manager@example.test` to request or cancel leave and as `admin@example.test` to decide independently. `contractor@example.test` sees a company without leave types; `staff@example.test` has self-service access without an employee link; `viewer@example.test` cannot open either page. All accounts use the Platform walkthrough password. This fixture is separate from the default walkthrough, which deliberately unlinks an account while proving the facade.

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
| `put_request_rules` | `people.leave.policies.manage` | Saves them: one or more ISO weekdays 1 to 7 (stored distinct, sorted) and 0 to 366 days, else `InvalidRequestRules` |
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
| `today` | none | Today's date in the company's `localization.timezone` |
| `self_summary` | `people.leave.self.view` | Current linked employee's balances and ledger, for the leave year containing the optional date |
| `self_requests` | `people.leave.self.view` | Latest 50 own requests, newest start first |
| `submit_request` | `people.leave.self.view` | `NewRequest` → `Request`, with current employment proved under its lock |
| `cancel_request` | `people.leave.self.view` | Cancels one's own pending request, or approved leave before its start |
| `pending_requests` | `people.leave.requests.approve` | Earliest 200 pending requests, with Workforce employee names |
| `decide_request` | `people.leave.requests.approve` | `Decision::Approve` or `Decision::Reject`, with an optional note |
| `requested_type_ids` | none | Distinct type IDs with pending or approved requests overlapping the inclusive date range |
| `carry_forward` | `people.leave.policies.manage` | Closes an ended leave year; returns `CarryForwardCounts` |
| `carried_forward_count` | none | Number of balances carried from the named year |
| `carry_forward_skipped` | `people.leave.policies.manage` | At most 200 skips, oldest first, and the full `total` |
| `enqueue_carry_forward` | `people.leave.policies.manage` | Queues an operator-delegated job; returns its committed `JobRef` |

- **Leave year.** The company-scoped Base Setting
  `people.leave.year_start_month` (default January) sets when each leave year
  starts. A leave year is labelled by the calendar year it starts in. It
  cannot change once the company has ledger entries, because each entry
  stores its leave year.
- **Request rules.** `people.leave.working_weekdays` (ISO weekday numbers,
  default Monday to Friday) and `people.leave.request_backdate_days` (default
  30, 0 to 366) hold the rules requests count dates by. All three
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

Every write authorizes the signed-in account for its self-service, approval or policy capability when it runs, through
`bilibi_people_workforce::authorization` at the facade, so pages, the
carry-forward worker and any other adapter share one boundary. The reads
of types, policies, rules, balances and entries need no capability, as
Bilimbi's do (request reads require `people.leave.requests.approve` or `people.leave.self.view`). A system scope cannot reach the helpers, a grant
revoked while a page stays open refuses the next call, and the performer is
the scope's actor, never an argument.

Every write runs in a `MutationBoundary` transaction that first holds the
company's writable lock (`bilibi_company::require_writable`, locked), so an
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


## Reviewed entry operation

`people.leave.entry.record` records opening balances and adjustments through the same validation as `Leave::record_entry`. Discover its input schema and the `people.leave.entry` guide through Base Agent API. The full workflow and retry rules are in [the entry guide](entry-guide.md).

The input takes `company_id`, `employee_id` (employment ID), `leave_type_id`, `entry_type` (`opening` or `adjustment`), `quantity` (exact decimal string), `occurred_on` (ISO date), `source`, `entry_key` and an optional `note`. Unknown fields are refused. The result is the existing `LedgerEntry` shape, with quantity as a string and dates in ISO format. The review includes company and employee names, the leave type and unit, the derived leave year, normalized quantity and note, and whether the entry will be created or returned.

The handler calls the facade on the plan's `MutationTransaction`; it never commits a separate transaction. Leave serializes its reviews and writes with a transaction advisory lock per tenant and company, taken before the Core company lock. This keeps the reviewed type, year rules and entry key stable until the owner commits. The facade then rechecks company access, writable status, current employment and entry rules under the company lock. Identical retries add no ledger mutation. A failure in any operation of a multi-operation plan rolls back the entry and its audit capture together.

The PL5 block in `fixtures/walkthrough/src/entry_operation.rs` uses the real operation registry and plan service. It shows discovery, direct-write refusal, a preview without ledger writes, opening and negative adjustment commits, identical retries, malformed input and reserved sources, cross-company and cross-tenant refusals, a conflicting second operation rolling back the whole plan, competing plans from separate sessions, permission revocation and an archived type after review. It checks ledger and captured-mutation counts without exposing plan codes or session tokens.

## Requests

For example, a morning request reserves `0.50` days. An afternoon request on that date can reserve the other half. A full-day or hour request cannot take a slot already held by either request. Pending and approved requests hold their slots; rejection and cancellation release them.

Call `submit_request` with `NewRequest::new(type_id, date, request_key)`. The default covers one full date. Set `ends_on` for a longer range, `day_part` to `am` or `pm` for half a date, or `day_part` to `hours` with `hours` for an hour-based type. Only configured working weekdays outside Reference Data calendar exceptions count. A range contains at most 366 dates and stays within one leave year. Hours are above zero and at most 24, with at most two decimals. Reasons and decision notes contain at most 500 characters. Keys contain 1 to 160 bytes.

**Pending requests reserve their quantity.** If the type requires balance cover, the ledger less other pending reservations must cover submission and approval. An identical key replay returns the saved request, even after it is decided. Changed facts under that employee's key return `RequestKeyConflict`.

Approval writes one negative `taken` entry. The requester and the account currently linked to the subject employment cannot approve or reject it. Rejection requires a note. An employee can cancel a pending request at any time, or an approved request strictly before its start. Cancelling approved leave writes a positive `cancelled` entry. Every transition appends a request event. All request, slot, event and ledger changes commit together and each has audit evidence.

Self-service resolves the signed-in account on every call and uses `Workforce::lock_self_employee` inside the Leave transaction before touching request rows. Unlinked accounts return `NotLinked`. Revoked grants refuse the next call. No caller supplies the requester or subject employee.

Other typed refusals include `InvalidRequest`, `SpansLeaveYears`, `TooFarBack`, `NoWorkingDays`, `OverlappingRequest`, `InsufficientBalance`, `YearClosed`, `NoteRequired`, `NotPending`, `SelfApproval`, `NotCancellable`, `EmployeeUnavailable`, `LeaveTypeUnavailable` and `Unavailable`.

## Carry-forward

For example, a closing balance of `8.00` with a cap of `3.00` creates a `3.00` carried entry in the next year and a `-5.00` expired entry in the old year. A negative balance carries zero and keeps its deficit. Types without a cap do not carry. The policy in force on the year's last date supplies the cap and the recorded policy version.

**Years close in order for each employee and type.** A current employee with pending requests in that year is skipped as `pending`. An earlier open year with pending requests or entries under a capped policy causes `previous_year_open`. The report names the earliest blocking year. Each run replaces that year's report. Closing a year clears older reports and any later report whose blocking year was resolved.

Every processed employee and type receives a `carried_forward` entry, including zero. Its key makes repeats harmless. That row closes the source year and every earlier year to grants, new requests, approvals, approved cancellations and manual entries. The result counts `carried`, `existing`, `pending` and `previous_year_open`. Supported source years are 1900 to 9997; a year that has not ended returns `YearNotEnded`.

The worker ID is `people-leave/carry-forward`. Its arguments are exactly `company_id` and `from_year`; unknown keys are refused. It runs with five attempts as the operator who queued it. Base Queue re-proves that identity and grant on every attempt. The worker also checks company access and active status inside its lease-fenced business transaction. Stale workforce reads retry; a revoked grant or year still open cancels. No system principal or schedule is contributed.

The PL2 walkthrough block is `fixtures/walkthrough/src/leave_requests.rs`. It covers request replay, half-day and hour slots, calendar rules, cover, concurrent submissions, decisions, cancellations, Payroll discovery, ordered carry-forward, skip replacement, caps, expiry and revoked permission.

From the Platform checkout, set `DATABASE_URL` to a fresh disposable database and run:

```sh
cargo run --release -p bilibi-people-walkthrough --bin bilibi-people-walkthrough
BILIBI_ENV=development cargo run --release -p bilibi-web -- seed-production
BILIBI_ENV=development BIND_ADDR=127.0.0.1:32172 APP_ORIGIN=http://localhost:32172 cargo run --release -p bilibi-web -- run
```

Use a private loopback port you own. The development production seeds configure the copy as Dev; setting the environment alone does not enable HTTP on a freshly installed walkthrough database. Keep `APP_ORIGIN` equal to the address you open in Chrome. The running app executes the walkthrough's three jobs: the operator's job completes with capped carry and expiry entries, the revoked actor's job cancels as `actor_refused`, and the unfinished-year job cancels as `year_not_ended`. Stop your app and drop the disposable database when finished.


## Leave policies page

Open People › Settings › Leave policies to set a company's leave year, request rules, types and entitlement versions. The company chooser offers active companies you may manage and retains an already-selected pending or suspended company while still authorized; its selection stays in the address so reload and Back restore it. If the selected company becomes unavailable while the page is open, the chooser asks you to select an available company.

Add a leave type, then add its first policy version. Each version shows its effective dates, yearly entitlement and optional carry-forward cap. The type, policy-version and skipped-balance lists use the shared FlexTable and ListColumns for column selection, arrangement and compact rows. They keep Bilimbi's row order and do not add sorting or pagination.

Grant entitlements for a leave year, or queue carry-forward from an ended year. Confirmations keep the company, year and action on the server; Cancel and Escape change nothing. Archive and Restore also ask for confirmation. The carry-forward section shows the previous year's carried count and the latest skips, oldest year first, naming the pending or earlier year to resolve. When more than 200 balances were skipped, the notice shows both the displayed and full counts. Refresh the page after the worker runs to see its result.

Every write-shaped event checks the current policy-management permission and company access before calling Leave, which checks them again inside its transaction. A grant revoked while the page is open refuses the next action and clears employee names and write controls. Form refusals preserve submitted values. The page uses the shared flash dismissal event and keeps messages through table changes.


For the PL4 browser checks, run `cargo run -p bilibi-people-walkthrough --bin leave-policy-scenarios -- deny` while the manager's page is open, then try its next action. Run the same command with `allow` to restore access. After installing the walkthrough, `skips` creates 201 types with earlier unclosed balances through Leave and carries the next year, so the page can show the cut-short notice. Run `skips` once per disposable database. These commands use `DATABASE_URL` and `BILIBI_ENV=development`, like the walkthrough above.


Company choices use Workforce's `authorization::company_choices`, which reads the full lifecycle status from Core. Pending and suspended companies are hidden unless already selected; archived companies are always hidden. Keeping a selection does not change the records' read and write rules.

## Queued policy writes

Policy and catalog writes recheck `people.leave.policies.manage` after the company lock through the private `begin_policy` helper. Self-service requests and approval decisions retain their own permission checks. The `people-sweep-a` walkthrough command queues a leave-type creation behind the lock and revokes its grant before releasing it.
