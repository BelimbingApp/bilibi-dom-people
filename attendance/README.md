# Attendance

Module ID: `people/attendance`, crate `bilibi-people-attendance`. Time and attendance: a company's clock facts and the days projected from them, its shift templates, its clocking locations and its attendance rules. Rosters, adjustment requests and allowance rules follow in port plan row PA2; the pages in PA3 to PA5; the device operation in PA6. This README is the owner contract; the Bilimbi source it ports is [`attendance/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/attendance/docs/README.md).

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

`record_clock` has no capability of its own, as in Bilimbi: the caller authorizes what it records. Self-clocking (PA2) proves the signed-in employee with `people.attendance.self.view` and the company's `self_clock_enabled` rule; an approved adjustment is recorded by its approver; the device operation (PA6) declares its own key. The scope's actor is recorded as the fact's actor. An employee who is not a working employee of the company is `NotFound`.

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

All five live under the Authz domain `people` and are granted by an administrator; Bilimbi grants none of them to a system role. `approve` is a verb Base does not declare, so this module declares it.

| Capability | Opens |
| --- | --- |
| `people.attendance.self.view` | My attendance, self-clocking and own requests |
| `people.attendance.rules.manage` | Rules, shift templates, clocking locations |
| `people.attendance.roster.manage` | Rosters (PA2, PA5) |
| `people.attendance.adjustments.approve` | Adjustment decisions (PA2, PA5) |
| `people.attendance.allowances.manage` | Allowance rules (PA2, PA3) |

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

`base/authz`, `base/database`, `base/datetime`, `base/module_registry`, `base/party_ids`, `base/settings`, `base/tenancy`, `core/company`, `people/settings`, `people/workforce`; the Cargo manifest lists the same edges, which Bilimbi's descriptor also has (less `base/audit` and `core/user`, which the rosters and adjustments of PA2 bring).
