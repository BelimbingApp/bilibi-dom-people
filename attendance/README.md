# Attendance

Module ID: `people/attendance`, crate `bilibi-people-attendance`. Time and attendance: a company's clock facts and the days projected from them, its shift templates, its clocking locations and its attendance rules. It also plans and publishes rosters, handles missing-punch requests, and stores allowance rule versions. Pages follow in PA3 to PA5. The device operation is available through Agent API. This README is the owner contract; the Bilimbi source it ports is [`attendance/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/attendance/docs/README.md).

## Public API

Resolve `Attendance` from the registry as `people/attendance.attendance` (`bilibi_people_attendance::SERVICE`) and call it with the sealed `Scope` you were handed and an explicit platform company id. Companies and employees are read only through `people/workforce`; a stale or unavailable workforce read refuses the operation (`Unavailable`). Results are typed records; the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `rules` | none | The company's five rules (`Rules`) |
| `put_rules` | `people.attendance.rules.manage` | Stores the given rules (`RuleChanges`; `None` keeps a value); a bad value is `InvalidRules`; returns the rules as they now read |
| `record_clock` | none, see below | Records one clock fact for a working employee and projects the day; returns the `ClockEvent` |
| `list_days` | none | The 31 most recent days of one working employee, newest first (`Day`) |
| `self_days` | `people.attendance.self.view` | The same for the signed-in account's own linked working employee, resolved now; an unlinked account is `NotLinked` |
| `list_shift_templates` | none | The company's shift templates, active first then by code |
| `create_shift_template` | `people.attendance.rules.manage` | Adds one (`NewShiftTemplate`); refusals are `Invalid` field errors |
| `set_shift_template_status` | `people.attendance.rules.manage` | `active` or `retired`, else `InvalidStatus`; another company's or missing is `NotFound` |
| `list_clocking_locations` | none | The company's clocking locations, active first then by code |
| `create_clocking_location` | `people.attendance.rules.manage` | Adds one (`NewClockingLocation`); refusals are `Invalid` field errors |
| `set_clocking_location_status` | `people.attendance.rules.manage` | `active` or `retired`, else `InvalidStatus` |

Every operator write authorizes its capability on every call through the Workforce `Authorization` helpers with the scope it was handed, so a grant revoked while a page stays connected refuses the next event (`Unauthorized`); a company the scope cannot see is `NotFound`. Inside its mutation transaction a write holds the company's writable lock first (an archived company is `CompanyArchived`), rechecks the operator's grant after waiting for that lock, and records every row it touches as a `Change`. A clock proves working employment through Workforce's `working_employee_ids_on` on its own transaction and resolves rules and active locations under the same lock, so a queued clock observes a preceding rule change or retirement.

`record_clock` has no capability of its own, as in Bilimbi: the caller authorizes what it records. Self-clocking proves the signed-in employee with `people.attendance.self.view` and the company's `self_clock_enabled` rule; an approved adjustment is recorded by its approver; the device operation (PA6) declares its own key. The scope's actor is recorded as the fact's actor. An employee who is not a working employee of the company is `NotFound`.

## Clock facts and days

`record_clock` is idempotent by company, source and key: a replay with the same employee, type, instant and actor answers the recorded event, and one with different facts is `EventKeyConflict`. The key is 1 to 160 bytes and the source 1 to 32 (`InvalidEvent` otherwise); the type is `in`, `out`, `break_in` or `break_out`; the instant is kept to the second.

A fact's local date is its instant in the company's attendance time zone. A shift belongs to the day of its clock-in: a clock-in opens its own local date; any other event closes the previous day's open shift instead when it falls within the company's maximum shift length of that shift's clock-in, else it lands on its own date. The day is projected from its ordered events: the first clock-in and the last clock-out give the raw worked span.

| Day status | When |
| --- | --- |
| `in_progress` | a clock-in and no clock-out yet; reads as `exception_pending` once the maximum shift length has passed |
| `ready_for_review` | a clock-in and a later clock-out within the maximum length, with `worked_minutes` |
| `exception_pending` | no clock-in, a clock-out before the clock-in, or one past the maximum length (recorded, but the shift does not close) |

Breaks, late time and overtime are not computed, and no payroll claim follows from this projection.

A fact may carry coordinates (both or neither, within ±90 and ±180, kept to six decimals; `InvalidEvent` otherwise). The fact records the nearest active clocking location whose radius contains the point. When the company's `location_required` rule is on, a fact without coordinates is `LocationRequired` and one outside every active location is `OutsideClockingLocation`. Browser coordinates are browser-reported evidence, not proof against spoofing.

## Shift templates and clocking locations

A shift template is a code (1 to 40 characters, `^[A-Za-z0-9][A-Za-z0-9_.-]*$`, unique in the company), a name (1 to 120), local start and end times stored as minutes past midnight in the company's attendance time zone (an end at or before the start crosses midnight; `ShiftTemplate::span_minutes` and `shifts::clock` give the span and `HH:MM`), and break minutes shorter than the shift. A clocking location is a code and name as above, a point and a radius of 1 to 100,000 meters. Refused values come back as `Invalid` field errors in Bilimbi's changeset words ("can't be blank", "has invalid format", "has already been taken", "must differ from the start", "must be shorter than the shift", "must be greater than or equal to -90", …), for the page to turn into its flash sentence. Both are retired, never deleted, so published rosters and recorded facts keep their meaning; a retired location places no new fact.

## Company rules

Five company-scoped Base Settings, each with the editor capability `people.attendance.rules.manage` (group "People"), edited together on `/people/attendance/rules` once PA3 lands the page and until then through the Settings editor:

| Setting | Default | Meaning |
| --- | --- | --- |
| `people.attendance.timezone` | `Etc/UTC` | Local date and time for clock events, rosters and requests; a convertible IANA name (`bilibi_datetime::valid_timezone`) |
| `people.attendance.self_clock_enabled` | off | Linked employee accounts may clock themselves |
| `people.attendance.max_shift_hours` | 16 | Hours after clock-in that a shift stays open (1 to 24) |
| `people.attendance.location_required` | off | Clock events need coordinates inside an active clocking location; approved adjustments are exempt |
| `people.attendance.adjustment_window_days` | 7 | Days, counting today, for which an employee may request an adjustment (1 to 366) |

`put_rules` validates the given values (`InvalidRules`: the sentence a page shows is "Enter a valid time zone, a shift length of 1 to 24 hours, and an adjustment window of 1 to 366 days.") and stores each one at the company target in one transaction under the company lock.

## Capabilities

All six live under the Authz domain `people` and are granted by an administrator; Bilimbi grants none of them to a system role. `approve` and `record` are verbs Base does not declare, so this module declares them.

| Capability | Opens |
| --- | --- |
| `people.attendance.self.view` | My attendance, self-clocking and own requests |
| `people.attendance.rules.manage` | Rules, shift templates, clocking locations |
| `people.attendance.roster.manage` | Rosters (PA5) |
| `people.attendance.adjustments.approve` | Adjustment decisions (PA5) |
| `people.attendance.allowances.manage` | Allowance rules (PA3) |
| `people.attendance.clock.record` | Reviewed agent operation for a device or integration clock event |

## Agent operation

`people.attendance.clock.record` records one event through a reviewed Agent API plan. Its input schema requires `company_id`, a current working `employee_id`, a stable `event_key`, `event_type` (`in`, `out`, `break_in` or `break_out`), `source` and offset-bearing `occurred_at`; `latitude` and `longitude` are optional but must be supplied together. Keys and source identifiers are 1–160 and 1–32 bytes respectively. Coordinates are checked against the company's current location rule. Preview validates the current employee, company rules and location; commit repeats those checks after taking the company writable lock. The operation requires `people.attendance.clock.record` on the selected company. Its guide is registered under `people.attendance`.

## Self-service seam

`self_days` uses Workforce's `authorize_self` and the returned `SelfEmployee.employee_id`. Workforce re-proves the account through Base Authentication on each call and resolves its affiliation to a working employment in the requested company. An account unlinked between calls is refused even when a caller retains an earlier scope. PW1 owns this proof and the employment lock used by self-service writes.

## Database

`migrations/001-core.sql`, applied as `people-attendance-001-core`: `people_attendance_daily_summaries` (one row per company, employee and local date; status check; `(company_id, employee_id, on_date)` unique), `people_attendance_clock_facts` (`(company_id, source, event_key)` unique; type check; coordinates both or neither; `day_id` and `clocking_location_id` restrict deletes), `people_attendance_shift_definitions` (`(company_id, code)` unique; status and span checks) and `people_attendance_clocking_locations` (`(company_id, code)` unique; point, radius and status checks). `employee_id` is the employment id (`people_workforce_employments.id`), kept as a plain column as Bilimbi keeps it, because facts outlive an employment's working status; every table references `companies (tenant_id, id)`. Instants are UTC `timestamptz`, bound and read as RFC 3339 text, and dates as ISO text: the workspace's `tokio-postgres` has no date or time feature, and a Domain must not turn one on. Coordinates are `numeric(9,6)` through `rust_decimal`.

## Walkthrough

Run the two walkthrough binaries on the same disposable database:

```sh
BILIBI_ENV=development cargo run -p bilibi-people-walkthrough
BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin attendance
```

The first binary creates the employments. The second proves attendance: the rules with their refusals, shift templates and clocking locations added and retired, clock facts recorded for Sam Staff (replay, conflict, a cross-midnight close, a clock-out past the maximum length, a lone clock-out, the location rule against HQ before and after it is retired), the days as the operator and as `staff@example.test` see them, and PostgreSQL refusing what the facade never attempts. The queued-write proof holds the company lock while a clock or shift creation waits, then commits a location requirement or revoked grant. Both waiting calls refuse and leave no fact or template. It also unlinks the employee account through Core User, refuses a self-read with its earlier scope, then restores the link. Finally, it holds every other pool connection and proves a clock write and self-read can finish with one connection available.

## Dependencies

`base/agent_api`, `base/audit`, `base/authz`, `base/database`, `base/datetime`, `base/module_registry`, `base/party_ids`, `base/settings`, `base/tenancy`, `core/company`, `people/settings`, `people/workforce`; the Cargo manifest lists the same edges, which Bilimbi's descriptor also has (Core account identity is proven through Workforce, so attendance needs no direct `core/user` edge).

## Roster planning and publication

Use `plan_roster_entry(scope, company, employee, date, RosterValue)` to plan a shift, rest day or removal (`Shift(id)`, `Rest`, `None`). Planning requires `people.attendance.roster.manage`, a working employee and an active shift template. Clearing an unpublished entry deletes it immediately. Clearing a published entry keeps its published value until publication.

`publish_roster(scope, company, from, to)` publishes every pending entry in the inclusive period, which must span 1 to 31 days. It returns the count, increments an existing publication's revision, applies removals and records `people.attendance.roster_published`, including when the count is zero. Publication does not require an employee or shift to remain active: it publishes the planner's stored values, as Bilimbi does.

`roster(scope, company, from, days, query)` requires the same capability. It returns dates, at most 200 working employees, all templates, their entries and the company's pending entry count for the whole period. Search matches employee names and numbers without case sensitivity. Employees sort by name then number. `truncated` reports that more employees matched, while `RosterEntry::pending()` compares working and published values. Entries are a vector keyed by their `employee_id` and `on_date` fields.

`self_roster(scope, company, from, days)` requires `people.attendance.self.view` and resolves the linked working employee on every call. It shows only published values, with the shift code, name and local times. The My attendance page requests `SELF_ROSTER_DAYS` (14); the service accepts the reference's 1 to 31 days. Draft changes and pending removals leave that employee view unchanged until published.

## Missing-punch requests and self-clocking

`submit_adjustment(scope, company, &NewAdjustment)` takes a page-minted `request_key`, an `In` or `Out` event, a local date/time string and a reason. It resolves and locks the signed-in employee through Workforce's `lock_self_employee`, then rechecks the capability. The local time must exist unambiguously in the company's attendance timezone, fall within its adjustment window (today counts) and be at or before now. A reason is required and limited to 500 characters; a request key is 1 to 160 bytes.

A matching company/request key returns the existing request when employee, event type and proposed instant match. A changed payload is `RequestKeyConflict`. Another pending or approved request for the same employee, event type and instant is `DuplicateRequest`. Cancelled and rejected requests permit a fresh request with another key.

`cancel_adjustment(scope, company, id)` takes the same self-service lock and cancels only that employee's pending request. `decide_adjustment(scope, company, id, "approve" | "reject", note)` requires `people.attendance.adjustments.approve`. It refuses both the original requester and an account currently linked to the subject employee, even after account relinking. Rejection requires a note; either decision limits the trimmed note to 500 bytes. A request already decided or cancelled is `NotPending`.

Approval proves the employee is still working and ingests the proposed instant through the existing clock projection inside the deciding transaction. Its source is `adjustment`, its event key `request:<id>`, and its actor the approver. Approved adjustments are exempt from the location requirement. The decision and the clock event roll back together if either cannot be recorded. Decisions record `people.attendance.adjustment_approved` or `people.attendance.adjustment_rejected` with the request, employee, event type, proposed instant and applied event ID.

`self_adjustments(scope, company)` returns the linked employee's 20 newest requests. `pending_adjustments(scope, company)` requires the approval capability and returns the 200 oldest proposed times, with names read through Workforce. Requests keep their history when an employee stops working; an unavailable current name is `None`.

`self_clock(scope, company, type, key, latitude, longitude)` accepts only `In` or `Out`. It refuses impersonated and agent sessions, requires a linked working employee, `people.attendance.self.view` and enabled self-clocking. It uses source `web` and the server's current instant; its coordinates follow normal clock ingestion and the company's location rule. As with `record_clock`, replaying a key with a different instant refuses rather than changing the earlier fact.

## Allowance rules

Payroll reads the company-defined versions through `list_allowance_rules(scope, company)`, with no attendance capability required. Results include active and retired versions, sorted by code, newest effective start, then ID. `get_allowance_rule(scope, company, id)` reads one version. Neither read calculates or pays an allowance.

`create_allowance_rule(scope, company, &NewAllowanceRule)`, `end_allowance_rule(scope, company, id, until)` and `retire_allowance_rule(scope, company, id)` require `people.attendance.allowances.manage`. Codes and names use the shift template's validation. A unit is 1 to 32 characters, begins with a letter and otherwise uses letters, digits, `_`, `.` or `-`. Currency is a company-supplied three-letter code, trimmed and uppercased. Values are positive decimals stored as `numeric(14,4)`, with PostgreSQL rounding to four decimal places. A value that rounds to zero or exceeds the stored range is refused before SQL.

Effective dates are inclusive. A later version automatically ends an earlier active open version on the previous day. Any remaining active overlap refuses the whole transaction, including that automatic end and its audit action. A company/code/start date remains unique after retirement. An explicit end must precede the stored end, if present, and cannot precede the start. Ending a version, including an automatic end, records `people.attendance.allowance_rule_ended`. Retirement preserves the version and its dates for history.

All PA2 writes hold the writable company lock before their own rows and recheck their capability after acquiring it. Self writes also hold and re-prove the employment lock. Every inserted, updated or deleted row passes through `MutationBoundary`; audit actions use Base Audit on the same transaction. Migrations `people-attendance-002-rosters-adjustments` and `people-attendance-003-allowance-rules` add roster entries, corrections and allowance policies. Public date fields are ISO text and instant fields RFC 3339 text; input dates use `jiff::civil::Date`.

Run PA2's executable proof after the default People walkthrough has created its employments:

```sh
BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin attendance-workflows
```

It proves draft visibility and publication/removal, search and period bounds, retired shifts, request replay/conflict/duplicates/window/future refusals, approval and rejection, self-approval refusal, cancellation, location enforcement with adjustment exemption, allowance version closure/overlap/retirement/end refusals, revoked grants and account unlinking with an earlier scope, the four audit action kinds and PostgreSQL check constraints. PA3–PA5 own the pages and their browser parity checks.

## Rosters and attendance approvals

`/people/attendance/rosters` lets a roster planner choose a company, search employees and plan seven days from a chosen date. Drafts stay private until **Publish week** is confirmed. Publishing includes every unpublished change for that company and week, including employees hidden by search.

`/people/attendance/approvals` shows the 200 oldest pending adjustment requests for a chosen company. Approve records the requested clock event; Reject needs a note. Both decisions require confirmation, and employees cannot decide their own requests.

Both pages use shared FlexTables and recheck company access before each write. The Attendance facade checks permission again under the company lock.

For a disposable browser proof, run `DATABASE_URL=… BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin attendance-pages`, then start `bilibi-web run` against that database. Sign in as `manager@example.test` with the Platform walkthrough password. Search for `PA5-002`: one employee is shown while the pending count remains two. Confirm Publish week to release both drafts. Attendance approvals starts with three requests; the manager can decide the staff requests, and `admin@example.test` can decide the manager request independently. `viewer@example.test` cannot open either page.
