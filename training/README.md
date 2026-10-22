# Training

Module ID: `people/training`, crate `bilibi-people-training`. Training manages a company's course catalog, events, sessions, learning requests, team plans and budgets. Other People modules and pages use its public methods; the schema stays private. The behaviour reference is [Bilimbi Training](https://github.com/BelimbingApp/b-dom-people/blob/main/training/docs/README.md).

The catalog and governance migrations and public methods are implemented, along with the six Learning pages below. Participation, evidence, evaluation, passports and insights arrive in their own slices. The Courses and Sessions & calendar pages arrive in PT6. An administrator grants Training access; no Training grant is added to a system role.

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

The ten governance keys and `people.training.currencies` are described below. The remaining keys, the verbs `answer` and `generate`, the remaining menu leaves and `people.training.evaluation.*` settings land with their slices.

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

## Learning requests, plans and budgets

Employees propose learning, managers recommend it, and independent reviewers approve it against a company budget. This service implements Bilimbi's `Training.Governance`; the six learning pages use these methods.

For example, an employee creates a draft with `create_learning_request`, submits it with a reason, and their direct manager recommends it. HR reviews it before an approver reserves the cost. Every decision remains in `learning_histories`.

| Method | Required capability under `people.training` | Result |
| --- | --- | --- |
| `learning_currencies`, `put_learning_currencies` | `budgets.view`, `budgets.manage` respectively | Allowed three-letter uppercase currency codes, initially empty; repeated codes collapse on write |
| `create_budget_policy`, `supersede_budget_policy` | `budgets.manage` | Immutable allocation for an inclusive date period and currency |
| `learning_budgets` | `budgets.view` | All policy versions with successor, committed amount and remaining amount |
| `create_learning_request` | `requests.submit` | Draft owned by the employee currently linked to the signed-in account |
| `learning_requests` | `requests.submit`, `requests.recommend`, `requests.view` | Own, direct-report or HR register, selected through `LearningAudience` |
| `decide_learning_request` | The capability for the current step | Updated request and permanent decision |
| `create_learning_plan`, `amend_learning_plan` | `plans.submit` | Manager-owned draft with 1–300 permanent items |
| `learning_plans` | `plans.submit` for Team; `plans.view` for Hr | Own plans including drafts, or submitted plans for HR |
| `decide_learning_plan` | `plans.submit` for submit/cancel; `plans.approve` for approve/reject | Updated plan and permanent decision |
| `learning_histories` | Same capability and audience as the parent register | Decisions grouped by visible parent ID, oldest first |

`LearningAudience::SelfService` means the current employee's requests, `Team` means current direct reports for requests and the manager's own plans, and `Hr` means the company register. Team and HR request registers hide drafts until a submit decision exists. HR plan registers follow the same rule. Histories apply these visibility rules too; knowing an ID does not expose its draft or decisions. SelfService is not a plan audience.

Requests follow `draft → pending_hod → pending_hr → pending_approval → approved`, through `submit`, `recommend`, `review` and `approve`. A pending step may `reject`; the owner may `cancel` before approval. Every action needs a nonblank reason of at most 4,000 bytes. Each step checks its capability again. Only the current direct manager may recommend, and neither the creator nor an account currently linked to the subject may review or approve.

Plans start as drafts. An owner with a current team may submit or cancel a draft; a separate approver may approve or reject a submitted plan. A plan can link only approved requests of the manager's current direct reports. Amendments create a new version and retain the prior approved version until approval. **Approving an amendment supersedes the prior plan in the same transaction.** A competing amendment cannot then replace it again.

### Budget commitments

`NewBudgetPolicy`, `NewLearningRequest`, `NewLearningPlan` and `NewLearningPlanItem` are the input records. Dates use `YYYY-MM-DD`. Amounts use exact decimal strings: nonnegative, less than `100000000000000`, with at most four decimal places. Output amounts are strings too, preserving the database's four decimal places. Required text is trimmed and bounded to 4,000 characters. The service derives employee, manager, author and impersonator identities itself.

An approval needs an enabled currency and an active policy covering the proposed date. It snapshots that policy's ID and the request's estimated cost; approval above the remaining allocation is refused. Committed totals sum approved costs in the policy's currency and inclusive period. Remaining is allocation minus commitments, calculated together in one SQL snapshot.

A correction retains the prior currency and cannot overlap another active policy, replace an already superseded policy, exclude an earlier commitment's date, or reduce the allocation below commitments. Earlier policy rows and approval snapshots remain unchanged. Removing an enabled currency prevents new requests, policies and approvals in it; it does not erase history.

Migration `people-training-003-learning-governance` creates the six Bilimbi governance tables. Scoped foreign keys keep links in the same company; employee links reference Workforce employment IDs. PostgreSQL guards immutable policies, items and decisions, permanent content, terminal states, valid transitions, effective budgets and commitments. Writes take the company lock, recheck authorization, then use `MutationBoundary`; every inserted or updated record, including histories and superseded plans, produces its own audit change. Self-service writes re-prove the account under Workforce's employment lock. The governance advisory lock also serializes budget trigger checks for the same company.

The PT3 proof block is `fixtures/walkthrough/src/training_governance.rs`. It creates records through these methods on a disposable database and fails if an expected transition or refusal differs.


## Learning pages

Employees can request learning, managers can recommend it and create plans, and independent reviewers can approve both. Budget operators set the accepted currencies and allocations.

Open **My learning**, add a request, then submit it with a reason. The manager recommends it from **Team requests**. HR reviews it from **Learning requests & reviews**, and an approver records the final decision there. Each decision asks for confirmation and remains in History.

| Address under `/people/training` | Task | Entry capability |
| --- | --- | --- |
| `/my` | Own learning requests | `people.training.learning.view` |
| `/team` | Direct-report requests | `people.training.requests.recommend` |
| `/plans` | Create team plans with up to 300 learning items; submit, cancel or amend | `people.training.plans.submit` |
| `/requests` | HR request register and review or approval | `people.training.requests.view` |
| `/plan-reviews` | Submitted plans and independent approval | `people.training.plans.view` |
| `/budgets` | Company currencies, policy versions and totals for each currency and period | `people.training.budgets.view` |

Task links appear only with their own grant. `learning.view` opens My learning without granting request submission: a reader without `requests.submit` sees the authorized task links and the reference's task guidance. Request, review, approval and budget controls each need their separate write grant.

All registers use the shared FlexTable with column controls, compact rows and URL pagination (`page`, `perPage`; 25, 50, 100 or 300 rows). The reference has no record sorting or search on these pages. History and plan items also use FlexTable. The company chooser defaults to the first active company the account may select; an explicitly unavailable company does not silently select another.

The live owner re-reads company reach and capabilities before every event. A confirmation retains its record, action and reason on the server and rechecks the specific action's grant on confirmation. The facade repeats its checks under the company lock. Refusals stay on the page as error messages. Budget corrections retain the prior currency; plan amendments create a new draft. Every form preserves entered values on refusal.

The page service is `people/training.learning_pages`, implemented in `src/web_learning.rs`. It declares the facade writes through `PageWrites::pending`, following the port plan. The My evaluations section belongs to the evaluation page slice (PT9).


For a mounted-page refusal proof, prepare the disposable database with `cargo run -p bilibi-people-walkthrough`, then keep the Learning page open. With the same `DATABASE_URL` and `BILIBI_ENV=development`, run `cargo run -p bilibi-people-walkthrough --bin learning-control -- deny-submit`, then press the stale request control. `allow-submit` restores the grant. The command also supports `deny-budget-manage`, `allow-budget-manage`, `unlink-staff`, `relink-staff` and `archive-company-two`. Archiving Company 002 is permanent, so use it last. These commands call the owning facades and belong only on the disposable walkthrough database.
