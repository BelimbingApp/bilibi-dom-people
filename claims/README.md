# Claims

Module ID: `people/claims`, crate `bilibi-people-claims`, ported from
Bilimbi's `Bilimbi.People.Claims`. Claim catalog, effective-dated claim
policies and assignments (this slice, port plan row PC1); employee claim
requests, decisions, reimbursement, hand-off and the events Finance Payroll
reads land in the next slice (PC2), and the three pages after that (PC3,
PC4, PC5).

## Ownership

The module owns nine tables, all of them created here: the catalog
(`people_claim_catalog_groups`, `people_claim_catalog_types`), policies
(`people_claim_policy_versions`), requests and their status history
(`people_claim_submissions`, `people_claim_request_events`), assignments
(`people_claim_employee_enrolments`, `people_claim_assignment_types`,
`people_claim_assignment_employees`) and hand-off batches
(`people_claim_handoff_batches`). Call `bilibi_people_claims::Claims`
(registry service `people/claims.claims`) with the sealed `Scope` and an
explicit platform company id; nothing else queries these tables. An
employee is the employment id (`people_workforce_employments.id`), and the
tables that name one carry a foreign key to it with the company, so a claim
or an assignment member is always an employment of the same company. Core
Company validates the company and the Workforce facade decides who is a
working employee. A login actor is never treated as an employee by itself.

## Authorization

Every write authorizes its own capability for the scope's signed-in actor at
the moment it runs, through `bilibi_people_workforce::authorization`; a
system principal cannot reach the facade. Catalog, currency, policy and
assignment writes need `people.claims.manage`; self-service reads need
`people.claims.submit`; `people.claims.approve` and `people.claims.reimburse`
(the verb this module declares) guard the decisions and payments of PC2.
Bilimbi grants all four to the Tenant Owner role. No function takes an
actor, employee or company from input except the company id it authorizes
first. Reads of a company's categories, claim types, policies and
assignments are capability-free company configuration, as in Bilimbi, for
any account that can see the company.

Every write runs through `MutationBoundary` and holds the company's writable lock (`organisation::require_writable`, locked), including currency changes. The facade checks the capability again inside that transaction. Archived companies refuse writes (`CompanyArchived`), and the company lock serializes catalog changes. Policy creation reads the allowed currencies after taking the lock. Refusals
are `ClaimsError` variants or `FieldError`s with Bilimbi's changeset words
(`can't be blank`, `has already been taken`, `must not be before the start
date`, `is not an allowed claim currency for this company`, `must have at
most two decimal places`, `is too large`, …).

## Currencies and policies

Claim currencies are the company setting `people.claims.currencies`, edited
through `Claims::put_currencies` or Base Settings (editor capability
`people.claims.manage`). The default is empty: a company accepts no policy
or claim until an operator chooses up to twenty codes of three uppercase
letters. The module ships no currency list.

A claim type belongs to one category and declares a receipt rule (`always`,
`above_threshold`, `never`) and who may claim it (`all_employees`,
`assigned_only`). A policy gives one claim type a period, a currency from the
company's list and optional per-claim, calendar-month and calendar-year
limits, each a two-decimal amount in a `numeric(14, 2)` column. A threshold
receipt rule needs the policy's receipt threshold; any other rule drops it.
`create_policy` locks the claim type's row and refuses a period that overlaps
another policy of the type (`OverlappingPolicy`); `end_policy` closes an
open-ended policy (`AlreadyEnded` otherwise), not before its start and not
before a live claim incurred under it (`RequestsAfterEnd`).

`open_claim_types` lists the active claim types of active categories with a
policy in effect on a date (default: today in the company's
`localization.timezone`, UTC when unset), each with that policy; with an
employee, an assigned-only type appears only when an assignment in effect on
that date covers both. Deactivating a category or claim type closes it to
new claims without deleting anything.

## Assignments

An assignment is a named, effective-dated group with the claim types it opens and the employees it covers. `set_assignment_members` replaces both lists together under the assignment's lock. Every claim type and employee must belong to the company, with at most 1,000 of each. `assignable_employees` includes all employment statuses through Workforce's `employees_at_any_status` directory, matching Bilimbi. Claim submission still requires a working employee.

## Self-service

`self_service_employee` and `self_open_claim_types` call `Workforce::authorize_self` on every operation. Workforce re-proves the account link and resolves its working employment in this company. An unlinked or relinked account cannot keep reading its former employee's open types through an old scope. Requests in PC2 can use Workforce's `lock_self_employee` inside their mutation transaction.

## Schema

`migrations/001-catalog-policies-requests.sql` creates the catalog, policy,
request and event tables in the shape Bilimbi's two migrations leave them,
and `002-assignments-handoff.sql` the assignment and hand-off tables with
the request's batch reference. Every table carries its tenant and company
with a foreign key to `companies`; a child row's foreign key includes the
company, so a claim type, policy, request or member never crosses companies.
Date columns are bound and read as text. Checks keep the receipt rules,
eligibilities, statuses, currency shape, positive limits, decision and
reimbursement columns consistent; a partial unique index keeps a receipt
reference on at most one live claim of an employee.

## Walkthrough

`fixtures/walkthrough` runs the Claims block after the Workforce one
(`fixtures/walkthrough/src/claims.rs`): the manager sets Company 001's
currencies, adds a category and three claim types with the three receipt
rules and both eligibilities, adds, overlaps and ends policies, assigns the
assigned-only type to Sam Staff, reads what is open to each employee and
account, deactivates and reactivates the catalog, and ends with PostgreSQL
refusing what the facade never attempts. The viewer, the unlinked admin,
staff without the grant and the contractor at another company show the
refusals.
