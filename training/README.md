# Training

Module ID: `people/training`, crate `bilibi-people-training`. A company's
course catalog, delivery events and session calendar, and later its
attendance records, evidence, learning requests, plans, budgets,
evaluations, passports and insights. Other People modules and the Training
pages reach it only through this module's facade; none of them reads its
tables. This README is the owner contract; the Bilimbi source it ports is
[`training/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/training/docs/README.md).

This is the catalog slice (PT1 of the port plan): migration 001, the
`Training` facade for courses, events, sessions and the calendar, and the
four catalog capabilities. PT2 adds participation and evidence, PT3
requests, plans and budgets, PT4 evaluation and effectiveness, PT5
passports and insights; the Courses and Sessions & calendar pages, with
their menu leaves, land in PT6. Until a page lands the sidebar shows no
Training entry, and an administrator grants the catalog capabilities.

## Public API

Resolve `Training` from the registry as `people/training.training`
(`bilibi_people_training::SERVICE`) and call it with the sealed `Scope` you
were handed and an explicit platform company id. Results are typed records;
the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `courses` | `people.training.courses.view` | The company's courses (`Course`), active or not, by name |
| `create_course` | `people.training.courses.manage` | Adds an active course from a `NewCourse` (code, name, optional description) |
| `update_course` | `people.training.courses.manage` | Changes a course's name, description or active flag (`CourseUpdate`: each `Some` replaces, `None` keeps; an empty description clears it); a course of another company or missing is `NotFound` |
| `set_course_active` | `people.training.courses.manage` | Deactivates or reactivates a course through `update_course` (the page's Deactivate and Reactivate actions) |
| `events` | `people.training.sessions.view` | The company's events (`Event`) with their course names, by name |
| `event_courses` | `people.training.sessions.manage` | The active courses an event may deliver, as `(id, name)` choices, so a session manager picks a course without course-management permission |
| `create_event` | `people.training.sessions.manage` | Adds an event (`NewEvent`: an active course of the company, a name, a positive capacity) |
| `create_session` | `people.training.sessions.manage` | Adds a session (`NewSession`: an event of the company, a name, a capacity at most the event's, an IANA zone and the local start and end wall clocks in it) |
| `calendar` | `people.training.sessions.view` | The sessions overlapping a half-open UTC window, by instant then id, at most 500 |
| `company_zone` | `people.training.sessions.view` | The company's own zone (`localization.timezone`), `UTC` when unset |
| `company_day_bounds` | `people.training.sessions.view` | The UTC bounds of the company's local days from one date up to another, as a window for `calendar`, with the zone |
| `allowed` | | Whether the actor currently holds a capability for a live company, for deciding which controls a page renders; never authority for an operation |
| `local_instant`, `day_start` | | The clock rules below, as free functions |

Every operation authorizes through the Workforce `Authorization` helpers
(Bilimbi `authorize/3`): the capability now, and the company axis through
Core Company (the actor's own company, or any company of the tenant with
`admin.company.tenant-wide.manage`). A denied or unknown capability or a
company outside the reach is `Unauthorized` ("You cannot do that for this
company."); a company the scope cannot see is `NotFound`; an archived
company is `CompanyUnavailable` ("Choose an active company."); and the
Workforce company read must be current, else `WorkforceNotCurrent` with the
freshness. Manage does not silently grant view. Every `TrainingError`
displays as the sentence Bilimbi's pages show for it
(`Training.Web.Support.message/1`), so a page flashes `error.to_string()`;
`Invalid` carries the field errors in Ecto's words beside the general
sentence.

## Catalog

A course is the company's own: its code (`^[a-z0-9][a-z0-9_-]*$`, at most
80 characters) is stable and unique in the company ("has already been
taken"), its name is trimmed to 1 to 160 characters, its description holds
at most 4,000. Operators supply every name and description; nothing is
seeded. A new course is active; deactivating one stops new events and keeps
the events and sessions it already has.

`create_event` requires an active course of the same tenant and company
(`CourseUnavailable`, "Choose an active course in this company.") and an
explicit positive whole-number capacity ("must be greater than 0", at most
2,147,483,647); no capacity default is installed. `create_session`
requires an event of the company (`EventUnavailable`) and a capacity at
most the event's (`CapacityExceeded`, "Session capacity cannot exceed event
capacity."), judged with the event row locked for the transaction. Each
session is one delivery of the same event cohort, so capacities are not
summed across sessions; enrolment, seats and attendance belong to
Participation (PT2). Events and sessions are create-only, and the sealed
actor's identity (`actor_user_id`, `impersonator_id`) is recorded on every
fact; a caller cannot supply it.

## Time and calendar contract

A session is entered as `starts_local` and `ends_local` (`YYYY-MM-DDTHH:MM`,
seconds optional, as a `datetime-local` control posts them) in an explicit
IANA `time_zone`. `local_instant(local, zone)` places the wall clock in the
zone through `jiff`'s bundled database and refuses what the zone cannot
place rather than guessing: a clock inside a DST gap is `NonexistentTime`
("That local time does not exist. Choose a time after the clock change."),
one inside an overlap `AmbiguousTime` ("That local time occurs twice.
Choose a time outside the clock overlap."), and an unknown zone or
unparsable clock `InvalidTimeZoneOrTime` ("Enter valid local times and an
IANA time zone."). The end must be strictly after the start
(`InvalidTimeRange`, "The end must be after the start."). The UTC instants
(`timestamp(0)`, bound and read as text) and the delivery zone are stored;
no server-local or country-specific zone is assumed.

`calendar(scope, company, from, until)` answers sessions overlapping the
half-open UTC window: `starts_at < until` and `ends_at > from`. It includes
sessions starting before the window, orders by instant and id, refuses a
window that is empty, backwards or longer than 366 days
(`InvalidCalendarRange`) and returns at most 500 rows; a page reports that
bound when the count reaches it. Calendar days use the company's own zone,
Base DateTime's `localization.timezone` as Core Company manages it:
`company_day_bounds(scope, company, from_date, until_date)` bounds a month
at the first instant of each local day, and `day_start(date, zone)`
resolves a midnight DST gap to the first instant after it and a midnight
overlap to the earlier of the two, so a clock change at local midnight
never blanks a page. A session form starts with the company zone, which an
operator can change for that delivery.

## Schema

Migration `people-training-001-catalog` creates `people_training_courses`,
`people_training_events` and `people_training_session_runs` as Bilimbi's
`20261001060101_create_people_training_catalog` leaves them (plus a company
foreign key on courses). Composite foreign keys keep an event in its
course's tenant and company and a session in its event's. Checks enforce
positive capacity and ordered instants. Two triggers enforce what a check
cannot: `people_training_session_guard` refuses a session whose capacity
exceeds its event's (locking the event row) or whose zone PostgreSQL does
not know, and `people_training_event_guard` refuses shrinking an event
below a session it already has. Every write goes through
`MutationBoundary`, recorded as a `people_training_course`,
`people_training_event` or `people_training_session_run` change, under the
company's writable lock (`organisation::require_writable`), so an archived
company refuses and a company's writes serialize.

## Capabilities

The Authz domain is `people`. Bilimbi grants no Training key to a system
role: an administrator grants them.

| Capability | Does |
| --- | --- |
| `people.training.courses.view` | Reads the catalog (`/people/training/courses`, PT6) |
| `people.training.courses.manage` | Adds and changes courses |
| `people.training.sessions.view` | Reads events and the calendar (`/people/training/sessions`, PT6) |
| `people.training.sessions.manage` | Adds events and sessions, and picks among the active courses |

The remaining 27 keys, the verbs `recommend`, `answer` and `generate`, the
eight menu leaves and the `people.training.currencies` and
`people.training.evaluation.*` settings land with their slices.

## Walkthrough

The Training block of `fixtures/walkthrough` (`src/training.rs`) has the
admin grant `manager@example.test` the four catalog capabilities, then
creates courses, an event and sessions through the facade with the
refusals beside each write, sets Company 001's zone to `Asia/Kuala_Lumpur`
and reads the November and December 2026 calendars bounded in it (a
session crossing midnight on 30 November appears in both), shows the
calendar window refusals, resolves `day_start` across a midnight gap
(`America/Santiago`, 6 September 2026) and a midnight overlap
(`America/Havana`, 1 November 2026), and ends with PostgreSQL refusing an
event shrunk below its session, an oversized session, an unknown stored
zone, a session ending when it starts, a zero capacity, a cross-company
event and a duplicate code. Run it as the repository README says, with
`BILIBI_ENV=development` set: the block grants capabilities through Base
Authz, whose step-up the Platform enforces outside development mode.
