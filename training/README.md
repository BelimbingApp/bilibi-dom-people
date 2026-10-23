# Training

Module ID: `people/training`, crate `bilibi-people-training`. Training manages a company's course catalog, events, sessions, learning requests, team plans and budgets. Other People modules and pages use its public methods; the schema stays private. The behaviour reference is [Bilimbi Training](https://github.com/BelimbingApp/b-dom-people/blob/main/training/docs/README.md).

The catalog, attendance records, private evidence and governance methods are implemented, along with the six Learning pages below. The Courses and Sessions & calendar pages are available, and Training records arrive in PT7. Evaluation, passports and insights arrive in their own slices. An administrator grants Training access; no Training grant is added to a system role.

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
| `update_course_versioned` | `people.training.courses.manage` | Updates inline drafts only when their stored-value snapshot still matches under the course lock |
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
| `people.training.records.view` | Reads attendance revisions, sessions and private evidence |
| `people.training.records.manage` | Records attendance and corrections; lists working employees |
| `people.training.evidence.manage` | Attaches PDF evidence |
| `people.training.retention.manage` | Purges expired evidence, lists purge holds and retries cleanup |

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

## Attendance records

Use `Training::record(scope, company_id, &NewAttendance)` to confirm attendance or record an absence. For example, confirming an employee with key `register-12` creates revision 1. Sending the same session, employee, status and trimmed reason with that key returns the original fact. Changing any of those values with that key returns `ImportConflict`. Use a new key for a correction; history keeps both revisions.

`employee_id` is a Workforce employment ID. A new fact requires a current working employee in that company. Status is `AttendanceStatus::Confirmed` or `Absent`; the trimmed reason must contain 1–2,000 characters, and the import key 1–160 characters. Import keys are company-wide and are not trimmed.

The company writable lock and session row lock protect the revision and capacity decisions. Only each employee's latest revision counts towards capacity. A confirmed correction uses the existing place; an absence releases it. A new confirmation when the session is full returns `AttendanceCapacityExceeded`. Exact replays return the original fact even after later corrections or employment changes, and create no audit mutation.

| Function | Result | Permission |
| --- | --- | --- |
| `record` | `AttendanceFact` with its revision, authorship and recorded UTC instant | `records.manage` |
| `history` | Every revision for a session, ordered by employee then newest revision | `records.view` |
| `sessions` | At most 500 sessions, newest first | `records.view` |
| `participation_employees` | Current employee IDs and directory names | `records.manage` |

All permission suffixes above start with `people.training.`. Reads and writes recheck company access. Migration `people-training-002-participation` creates the attendance and evidence tables. Its triggers refuse changed or deleted facts, skipped revisions, over-capacity confirmations and shrinking a session below confirmed attendance. Evidence provenance also refuses updates, deletes and truncation. Composite foreign keys keep sessions, facts and evidence within their tenant and company.

## Private evidence

Call `attach(scope, company_id, fact_id, bytes)` with `evidence.manage`. The fact must exist in the company, and the bytes must start with `%PDF-`. This matches Bilimbi's upload header gate; it does not claim to scan or fully validate an uploaded PDF.

Base Artifacts publishes bytes before Training opens the transaction that links the evidence row. Its concrete `documents::DocumentOwner` has stable owner ID `people/training`; **keep the type and owner ID stable**, because Artifacts persists both. Its callbacks use `workforce::authorization::authorize_company_proof` on the supplied transaction and recheck the permission, active company and fact. Create is checked again before publication. Training then rechecks `evidence.manage` under its company writable lock before recording provenance through `MutationBoundary`. If linking fails, Base still tracks the artifact for expiry and recovery; no untracked file cleanup runs.

`evidence(scope, company_id, fact_id)` lists permanent provenance, including documents whose bytes have expired. `read_evidence` calls Artifacts on every request, so expiry, integrity checks and changed permissions apply immediately. Both require `records.view`.

The `DOWNLOADS` contribution serves `/people/training/evidence/{company_id}/{id}`, where `id` is the artifact UUID. It returns an `evidence.pdf` attachment with `Cache-Control: private, no-store` and `X-Content-Type-Options: nosniff` through shared `FileBody`. An unavailable document returns 404. There is no public file path or public storage URL.

An operator configures `artifacts.storage_root`, `artifacts.retention_days`, size limits and purge pacing in Base Settings. `purge`, `purge_holds` and `retry_purge` require `retention.manage` and delegate to Artifacts. Base keeps failed cleanup attempts, waits between retries and holds repeated failures for operator review. Retrying a hold rechecks deletion authority and leaves Training's provenance intact. Training contributes no worker or schedule.

The PT2 block in `fixtures/walkthrough/src/participation.rs` runs against the same disposable database as the existing walkthrough. It creates its records through `Training`, demonstrates a two-connection race for the last place, replay/conflict and corrections, permission revocation with an old scope, PDF round trips, expiry and cleanup failure followed by hold retry. Raw SQL attempts demonstrate the permanent-history guards. Set `BILIBI_TRAINING_PROOF_ROOT` to an absolute private task directory to keep its artifacts for a browser download check; otherwise the block uses a fresh directory under the system temporary folder.

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

All registers use the shared FlexTable with column controls, compact rows and URL pagination (`page`, `perPage`; 25, 50, 100 or 300 rows). The reference has no record sorting or search on these pages. History and plan items also use FlexTable. The company chooser defaults to the first company Workforce lets the account select; an explicitly unavailable company does not silently select another.

The live owner re-reads company reach and capabilities before every event. A confirmation retains its record, action and reason on the server and rechecks the specific action's grant on confirmation. The facade repeats its checks under the company lock. Refusals stay on the page as error messages. Budget corrections retain the prior currency; plan amendments create a new draft. Every form preserves entered values on refusal.

The page service is `people/training.learning_pages`, implemented in `src/web_learning.rs`. It declares the facade writes through `PageWrites::pending`, following the port plan. The My evaluations section belongs to the evaluation page slice (PT9).


For a mounted-page refusal proof, prepare the disposable database with `cargo run -p bilibi-people-walkthrough`, then keep the Learning page open. With the same `DATABASE_URL` and `BILIBI_ENV=development`, run `cargo run -p bilibi-people-walkthrough --bin learning-control -- deny-submit`, then press the stale request control. `allow-submit` restores the grant. The command also supports `deny-budget-manage`, `allow-budget-manage`, `unlink-staff`, `relink-staff` and `archive-company-three`. Company 002 is already archived by an earlier proof; archiving Company 003 is permanent, so use it last. These commands call the owning facades and belong only on the disposable walkthrough database.

## Evaluation and effectiveness

Employees evaluate completed sessions, and their current direct managers record what changed at work. The service follows Bilimbi `Training.Evaluation`; My evaluations and Effectiveness follow in PT9.

For example, publish a policy with a one-day evaluation deadline and checkpoints at 30 and 90 days. After a session ends, prepare its reviews. Each current learner with confirmed attendance receives an evaluation and two manager reviews. Every review keeps the published criteria and timing, even after settings change.

### Settings and publication

`evaluation_settings` reads the six company settings below. `put_evaluation_settings` validates all six before saving them together. Both require `people.training.evaluation.policy.manage`. Base Settings exposes the same keys with that editor permission. The key prefix is `people.training.evaluation.`.

| Suffix | Accepted value | Default |
| --- | --- | --- |
| `evaluation_days` | Whole days, 0–3,650 | Unset |
| `checkpoints` | 1–24 distinct whole-day offsets, each 1–3,650 | Unset |
| `reminder_days` | Whole days before due, 0–3,650 | Unset |
| `minimum_cohort` | Distinct employees, 2–1,000 | Unset |
| `report_months` | 1, 2, 3, 4, 6 or 12 months | 3 |
| `report_grace_days` | Whole days after the reporting period, 0–3,650 | Unset |

`publish_evaluation_policy` takes `NewEvaluationPolicy`: inclusive ISO dates, separate evaluation and effectiveness criteria, and a reason. Publication captures the current evaluation days, checkpoints and reminder lead. Policies append numbered versions; their effective periods cannot overlap. `evaluation_policies` reads newest versions first. Both require the policy permission above.

A criterion has exactly `code`, `label`, `minimum` and `maximum`. Each list contains 1–50 unique codes matching `^[a-z][a-z0-9_]{0,79}$`. Labels and reasons must be nonblank and at most 4,000 bytes. Bounds are ordered integers between −10,000 and 10,000.

### Preparing and answering reviews

| Method | Required permission under `people.training.` | Behavior |
| --- | --- | --- |
| `prepare_evaluation_reviews(scope, company, session, now)` | `evaluation.reminders.manage` | Uses the session's local completion date and the policy covering it; returns obligations and the number of unavailable learners as `unknown` |
| `evaluation_reviews(..., EvaluationKind::Evaluation)` | Both `requests.submit` and `evaluation.submit` | Own current employee's evaluations |
| `evaluation_reviews(..., EvaluationKind::Effectiveness)` | Both `effectiveness.view` and `effectiveness.answer` | Current direct reports' effectiveness reviews |
| `answer_evaluation(scope, company, review, values, reason)` | Both permissions for that review's kind | Stores one permanent answer after checking the employee and reporting line again |
| `evaluation_reminders(scope, company, today)` | `evaluation.reminders.manage` | Creates a worklist entry once per unanswered review at its reminder date; counts missing recipients as `unknown` |

Only latest confirmed attendance qualifies. A confirmed correction preserves existing obligations and answers; an absent correction hides them until reconfirmed. Leavers retain attendance history but receive no new obligations. Preparation refuses an unfinished session or a missing effective policy. Repeated preparation and reminder runs create no duplicate rows or audit changes.

Answer values are a JSON object containing **every criterion code and no other key**. Each value is a bounded integer or explicit JSON `null` for unknown. Missing keys, fractions, booleans and out-of-range scores are refused. Answers need a reason, have no deadline cutoff, and cannot be edited or replaced. Self-service writes re-prove the signed-in employee under Workforce's employment lock, so unlinked accounts cannot use old access.

Reminders are on-screen worklist records. They do not send mail or notifications, and Training contributes no unattended schedule.

### Frozen summaries

`freeze_effectiveness_summaries(scope, company, today)` requires `evaluation.reminders.manage`. It freezes each completed fixed period only when today is **strictly after** the period end plus its grace days. The first period contains the earliest effectiveness due date. Later periods begin the day after the last frozen period, including empty periods. Changing `report_months` changes only future period lengths. `effectiveness_period_start` shows that next date with `evaluation.policy.manage`, or returns none before any effectiveness obligation exists.

At freeze time, only current employees whose latest attendance is confirmed count. The whole period, each policy/checkpoint group, and each criterion's distinct answered employees must meet `minimum_cohort`. A suppressed period has no groups; a suppressed group has no criteria; a suppressed criterion exposes neither answered count nor mean. Unknown answers do not count towards the answered minimum. Disclosed means round half up and retain exactly two decimal places, including `1.50`.

`effectiveness_summary` requires only `effectiveness.summary.view` and returns frozen snapshots newest first. It never exposes employee IDs, arbitrary cohort filters, population totals or complementary counts. **Later answers, employment changes and setting changes do not recompute frozen results.**

Migration `people-training-004-evaluation` creates five tables and guards policy publication, review identity, exact answer bounds and immutable history in PostgreSQL. All writes use the company writable lock and `MutationBoundary`; every row produces an audit change. No-op maintenance returns without committing an empty mutation. The executable PT4 proof block is [training_evaluation.rs](../fixtures/walkthrough/src/training_evaluation.rs), run through the existing disposable walkthrough.

## Passports and learning insights

Employees can read their own training passport; managers can read passports for their current direct reports. Each passport shows the latest attendance revision for each session. Evidence stays attached to its original revision, including after a correction.

For example, call `passport_employees(scope, company, PassportAudience::Team)` to populate the employee chooser. Pass a returned employment ID to `passport` with `page` and `perPage` parameters. The result contains the employee, Workforce freshness, and a `TrainingPage<PassportRow>`. Team membership and grants are checked again on every call. The manager's own employee never appears in their team.

| Method | Permission under `people.training.` | Result |
| --- | --- | --- |
| `passport_employees`, `passport` with `SelfService` | `passport.my.view` | Own working employee and latest attendance |
| `passport_employees`, `passport` with `Team` | `passport.team.view` | Current direct reports and their latest attendance |
| `generate_passport` | Audience permission plus `passport.generate` | Private PDF snapshot of at most 1,000 records |
| `read_passport` | Stored passport's audience permission | Original PDF bytes, after checking current membership |
| `read_passport_evidence` | `records.view`, or own/team passport permission for the fact's employee | Evidence bytes from any original revision |
| `learning_insight_periods`, `learning_insights` | `insights.view` | Frozen period choices and suppressed course counts |
| `learning_insight_drill` | Both `insights.view` and `records.view` | Latest attendance rows for one course |

Passport and insight pages use `page` and `perPage` (25, 50, 100 or 300; default 25). Out-of-range pages clamp to the last page. Passports sort by completion and fact ID descending. Course totals sort by course name and ID; drills sort by completion descending, employee ID, then fact ID descending.

### PDF generation and downloads

`generate_passport` creates a complete snapshot through the existing `DocumentOwner`, `PdfOwner` and `artifacts::pdf::render`. **More than 1,000 records refuses generation; it never truncates the PDF.** Self and team publication require current Workforce data. Native Workforce is always current; the read result retains freshness so a future stale self read can show its warning. Team reads refuse stale data.

The owner keeps the existing persisted adapter identity. It checks the audience grant, current account link, employee status and direct-report relationship before reading or publishing. Publication also needs `passport.generate`. Training holds the company lock and re-proves the signed-in employee under its employment lock for the whole passport file operation. A rollback-only `MutationBoundary` holds those locks while Artifacts commits its own reservations, publication and audit through a separate plain client. The owner checks the subject on the Artifacts transaction without trying to lock those rows again. Account relinking, archive and employment changes serialize with this boundary. An unlinked or relinked account cannot download a former employee's document. Retention operators can purge passport and evidence bytes; provenance remains.

Four `DOWNLOADS` routes retain Bilimbi's addresses. Replace `{audience}` with `my` or `team` and `{id}` with the artifact UUID:

- `/people/training/records/{audience}/document/{company_id}/{id}` downloads `passport.pdf`.
- `/people/training/records/{audience}/evidence/{company_id}/{id}` downloads `evidence.pdf`.

Every request checks its route permission and the document owner's current authorization. The shared `FileBody` supplies `private, no-store`, `nosniff` and attachment headers. Missing, expired, purged and refused documents answer 404. Base Settings controls private storage, byte limits and retention. Characters unsupported by the built-in PDF font produce a refusal; records stay available on screen.

### Disclosure rules

Record viewers choose inclusive ISO UTC dates with `from` and `until`, spanning at most 366 days. The end day's midnight boundary is included correctly by querying up to the following midnight. Drills use the same window and latest attendance revisions. Neither response contains evaluation scores.

Readers without `records.view` must select `period`, exactly matching a frozen Effectiveness period's start date. Arbitrary dates are ignored for them; an unavailable period is refused. This prevents overlapping arbitrary windows from revealing small attendance cohorts by subtraction.

The company must configure `people.training.evaluation.minimum_cohort` to at least two. Below that many distinct employees, a course exposes no employee, confirmed or absent count. When the overall cohort is large enough but either attendance count is below the minimum, **both attendance counts are withheld**. Frozen periods supply the allowed boundaries; attendance counts still use the latest attendance revisions, matching Bilimbi. Effectiveness scores remain the separate immutable summaries.

The passport/records pages are the PT7 slice and the insights page is PT9. This slice supplies their facade, types, permissions and download handlers.

Run the PT5 disposable proof after the default walkthrough with `cargo run -p bilibi-people-walkthrough --bin training-passports`. It creates its own records through the owners, checks corrections and original evidence, both suppression rules, period-only access, revoked grants, unlinking, reporting-line changes, pagination, and complete PDFs at 1,000 and 1,001 records. A disposable SQL reservation gate proves that account unlinking waits for in-flight passport publication, then the old account scope is refused. It prints all four download addresses for browser checks. The same command accepts `deny-my`, `allow-my`, `unlink-staff` and `relink-staff` to change access while a browser session remains open.

## Courses and scheduling pages

Open **Development → Courses** to add a course, edit its name or description in place, or deactivate it. Deactivation keeps existing events and sessions; Reactivate makes the course available again. Course codes stay fixed. Readers without course management access see the catalog and the read-only explanation.

Open **Development → Sessions & calendar** to create an event for an active course, then schedule sessions for that event. Enter the delivery zone and local start and end times. The form starts with the company zone; invalid zones, clock changes and excess capacity return a refusal without closing the form.

Both pages use shared FlexTables with column controls and compact rows. Their company selection and pagination stay in the URL: Courses uses `page`; Events uses `event_page`; Sessions uses `session_page`. All use `perPage` with 25, 50, 100 or 300 rows. Scheduling also keeps `date` (normalized to the first day of its month) and `view=list|calendar`. Each calendar day includes sessions occupying that company-local day, including sessions crossing midnight and excluding an end exactly at midnight. A warning appears at the 500-session limit.

The page checks access before each write-shaped event and the facade checks again inside the company lock. Company selection comes from the authorized URL state; forms cannot supply a different company. Status controls carry their intended active flag, so repeated clicks cannot invert the result. Inline editors carry `Course::edit_version()` and compare it inside the course lock, so a stale draft cannot overwrite a later edit. Writes still use the declared `PageWrites` allowance until their operation contracts are reviewed.
