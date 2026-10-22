# Performance

Performance keeps position descriptions, individual KPI targets, evidence, reviews and employee responses as permanent history.

For example, a manager proposes a target for a direct report. Another person reviews it with a reason and communicates it. The manager then records observations and drafts a review using those exact records. An independent approver releases the review, and the employee can respond once. Corrections create successor versions; they leave the original wording, evidence and response readable.

This module mirrors `Bilimbi.People.Performance` in `b-dom-people`. Its service is `people/performance.performance`; callers use `Performance` and the input and record types exported by `src/lib.rs`. SQL, validation and transitions are private. The `/people/performance` workspace and `/people/performance/my` page call the same service.

## Workflow

| Operation | Rule |
| --- | --- |
| `draft_description`, `publish_description` | Complete, immutable content; pin an exact Organisation position version and published Skills profile versions. Publication rechecks those versions and refuses overlapping inclusive dates for a position. An open interval is never shortened automatically. |
| `define_kpi` | Record the unit, measure, source, calculation version, precision, direction and interpretation. A rubric direction needs rubric text. Calculation versions describe provenance; they do not execute code. |
| `propose_target`, `review_target`, `publish_target` | Propose only for a current direct report. Review requires a reason and someone other than the author or employee. Communicate only a reviewed, non-confidential target, with the same independence rule. |
| `amend_target` | Preserve the definition, subject, period and confidentiality. A reason and later effective date begin a new review and publication cycle. Each version has at most one successor. |
| `record_observation`, `correct_observation` | Record attributable evidence for a current direct report. Only its author can append a correction, with a reason. Employee, window and source reference stay fixed. |
| `draft_review`, `release_review` | A published description and its exact position version must apply to the employee at both period boundaries. Pin at least one observation and communicated target, for the employee and period, recorded by the explicit UTC cutoff. Release rechecks the assignment, requires an independent approver and refuses a future cutoff. |
| `correct_review` | Only the original author can create one successor to a released version. Keep the employee and period, state a reason and retain or advance the cutoff. The successor needs independent release again. |
| `respond` | Resolve and lock the signed-in employee again on every call. Accept one append-only response or dispute per account and released review version. |

Inputs use ISO `YYYY-MM-DD` dates and UTC instants ending in `Z` or `+00:00`. Reads return UTC instants ending in `Z`. Text is trimmed and limited to 8,000 characters. Evidence selections contain at most 100 positive IDs each; duplicates are removed. KPI precision is 0–8 and directions are `higher`, `lower`, `band`, `rubric`.

## Access and reads

Every call takes the sealed `Scope` and an explicit company ID. Workforce authorization checks the current grant and company reach. Writes refuse impersonation and recheck authorization after taking the company's writable lock. Archived companies refuse writes. Proposal and evidence authors re-prove their employment link under its lock; response writers do the same. An approver's current employment link is checked even if that employment is no longer working.

| Capability | Operations |
| --- | --- |
| `people.performance.view` | `reviews`, `review`, `planning_records`, `planning_choices` |
| `people.performance.descriptions.manage` | Draft and publish descriptions |
| `people.performance.kpis.submit` | Define KPIs, propose and amend targets |
| `people.performance.kpis.review` | Review target proposals |
| `people.performance.kpis.approve` | Communicate reviewed targets |
| `people.performance.reviews.submit` | Record and correct observations; draft and correct reviews |
| `people.performance.reviews.approve` | Inspect candidates and release reviews |
| `people.performance.self.view` | `my_records`, `respond` |

`allowed` is for showing controls; each operation authorizes again. No performance capability is granted automatically.

`reviews` returns only the caller's authored reviews. `review` permits the author or a company-authorized release approver. Drafts about the caller's own linked employee are excluded even with approval permission. `planning_records` excludes that employee from targets, observations, release candidates and prior reviews. Observations and prior reviews remain author-only; target reviewers and approvers can see company targets for other employees. Each list filters before taking its latest 100 rows.

`planning_choices` names employees through Workforce, offers current direct reports, uses a bounded Organisation page, and lists published Skills profiles. Published descriptions and communicated targets are selected separately, so newer drafts cannot crowd them out. `my_records` resolves the account's current working employee and returns only communicated, non-confidential targets and released reviews with their original pinned records and responses. An unlinked account receives `Unavailable`.

Review indexes use `PageOptions`: page starts at 1; page sizes are 10, 25, 50 and 100 (default 25). An oversized page is clamped to the last page. Missing, cross-company and cross-tenant records share `NotFound`. `PerformanceError` retains Bilimbi's refusal sentences for live pages.

## Storage and proof

Migration `people-performance-001-performance` creates eight empty tables: `people_performance_descriptions`, `people_performance_kpi_definitions`, `people_performance_kpi_targets`, `people_performance_evidence`, `people_performance_appraisals`, `people_performance_review_evidence`, `people_performance_review_targets` and `people_performance_responses`. It ports all eight trigger registrations from Bilimbi's migration (the dispatch plan counts ten). Composite foreign keys keep owned links within the tenant and company. External identities and versions are proved through Workforce, Organisation and Skills; no sibling schema is queried.

Database guards refuse content edits, deletion, invalid transitions, invalid successors, changed review rationale, unreviewed or confidential publication, evidence outside the review window, missing release pins and new pins after release. All application writes use `MutationBoundary`; each record and review pin returns a `Change`, and the deciding transaction captures audit evidence with the caller's attribution. A failure rolls back the entire operation.

Run `DATABASE_URL=… BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin performance` against an empty disposable PostgreSQL database. The Performance block in `fixtures/walkthrough` migrates it, installs the Platform fixture and creates its own records through owner facades. It exercises the facade workflow and its refusals; there are no unit tests or production seed rows. Run the mounted Platform's `scripts/domain-ci.sh` for the composition gate. See [Workforce](../workforce/README.md), [Organisation](../organisation/README.md) and [Skills](../skills/README.md) for the owner contracts this module uses.

## Pages

Open **Development › Performance reviews** to work with reviews, position descriptions, KPIs and evidence. Each task has a FlexTable with column selection, compact rows and URL pagination. Review pagination keeps `page` and `page_size`; the other bounded lists use `release_`, `descriptions_`, `definitions_`, `targets_` and `evidence_` prefixes. Page sizes are 10, 25, 50 and 100. The planning lists and pickers retain the facade's 100-record bounds.

Forms offer direct reports, current positions, published profiles and exact record versions. A correction selects a prior version and states its reason. Every planning write opens a confirmation with its input held on the server; Cancel discards the pending operation. Confirmation rechecks the current grant, then the facade rechecks company reach, employment links and workflow rules. A changed or duplicated confirmation payload cannot replace the selection.

**My performance** shows only communicated targets and released review versions. Each review retains its pinned evidence, targets and responses. Employees can record one response or dispute per released version. An unlinked account sees the existing unavailable message. This page is linked from My standing, as in Bilimbi.

Run `DATABASE_URL=… BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin performance-pages` on an empty disposable database to exercise page mounts and live events. It proves all task tables, pagination, cancellation, forged and duplicate confirmations, revoked grants, long evidence, target review and communication, independent release and employee responses.
