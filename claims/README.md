# Claims

Claims manages employee expenses from submission through approval and reimbursement. It also groups approved claims by currency for hand-off and CSV export.

Module ID: `people/claims`; crate: `bilibi-people-claims`; registry service: `people/claims.claims`. Behaviour mirrors Bilimbi's `Bilimbi.People.Claims`. My claims is available at `/people/claims`, and Claim operations at `/people/claims/operations`. Claim policies is available at `/people/claims/setup`.

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
`people.claims.submit`; `people.claims.approve` guards decisions and private queues; `people.claims.reimburse` guards payments, hand-off and export.
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

`self_service_employee` and `self_open_claim_types` call `Workforce::authorize_self` on every operation. Workforce re-proves the account link and resolves its working employment in this company. An unlinked or relinked account cannot keep reading its former employee's open types through an old scope. Submission and withdrawal call Workforce's `lock_self_employee` inside their mutation transaction.

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

## Requests and decisions

Call `submit_request(scope, company_id, &NewRequest)` for the signed-in employee. The input names the claim type, expense date, amount, currency, description, receipt number and optional duplicate confirmation. It never names an employee or submitter.

The write holds Company → Employment locks and re-proves the account link under the employment lock. It checks that the claim type and category are active, any assignment covers the expense date, and the date is not in the future in the company's time zone. The policy must cover that expense date. Its currency must match the request and remain in the company's allowed currencies.

Amounts are positive, have at most two decimal places and fit `numeric(14,2)`. Receipt rules are `always`, `never`, or strictly `above_threshold`. Receipt references collapse whitespace and uppercase before comparison. A reference already used by this employee's live claim refuses with `DuplicateReceipt`. A claim of the same type, date, amount and currency refuses with `PossibleDuplicate` until `confirm_duplicate` is true; only a real match records `duplicate_confirmed = true`.

Calendar-month and calendar-year limits count every live claim of that type and currency, across policy versions. An approved amount replaces its claimed amount in the sum. Rejected and withdrawn claims release both receipt references and limit usage. The company and employment locks prevent concurrent requests from passing the same remaining limit.

| Call | Permission | Rule |
| --- | --- | --- |
| `self_requests`, `self_request_events` | `people.claims.submit` | Resolve the linked employee again on every read. |
| `withdraw_request` | `people.claims.submit` | Only the linked employee's submitted claims. |
| `employee_requests`, `request_events`, `claim_queue` | `people.claims.approve` | Private claims and history for this company. |
| `approve_request` with `Approval` | `people.claims.approve` | Submitted only; absent amount means full approval. A lower amount needs a reason. |
| `reject_request` with `Rejection` | `people.claims.approve` | Submitted only; a reason is required. |
| `reimburse_request` with `Reimbursement` | `people.claims.reimburse` | Approved only; payment reference is optional, at most 100 characters. |
| `requested_claim_type_ids` | Company configuration read | Distinct live types incurred in the inclusive date range, for Payroll. |

An account cannot approve, reject or reimburse a claim it submitted, or a claim of the employee it is linked to now. `Workforce::owns_employee` checks the current account link under the employment lock, including inactive employments. Refusals use `OwnClaim` and `NotDecidable`. Every transition saves one `people_claim_request_events` row, including the decision reason for approval and rejection. A refusal rolls back the request, history, audit and outbox together.

The operator queue accepts `submitted`, `approved`, `rejected` or `reimbursed`. It returns at most 500 records, with submitted claims oldest first and other statuses newest first. Queue rows include employee numbers and names through Workforce and category/type labels through this module's catalog. Employee claim lists sort by newest expense date and then newest id. Returned timestamps are ISO 8601 UTC strings; render them through Base DateTime in a page.

## Hand-off and CSV download

`handoff_waiting` groups approved, unbatched claims by currency. `create_handoff_batch` requires `people.claims.reimburse` and claims every waiting approved request in the chosen currency under the company lock. The stored batch records its creator, time, count and exact approved total. It refuses empty queues with `NothingToHandOff`. A total that cannot fit the batch's `numeric(14,2)` column is a `total_amount` field error.

`handoff_batches` needs `people.claims.approve` and returns the newest 500 batches. `reimburse_batch` needs `people.claims.reimburse`; it pays every still-approved claim in one transaction. One own claim refuses the entire batch before any payment is recorded. A fully paid batch returns `NothingToReimburse`. Already reimbursed claims are skipped when the remaining claims are paid.

Download a batch at **`GET /people/claims/handoff/{company_id}/{id}`**. The `DOWNLOADS` contribution and `Claims::handoff_export` both recheck `people.claims.reimburse`; another company's batch returns not found. The filename is `claim-handoff-{id}.csv`. `FileBody` supplies UTF-8, attachment disposition, `private, no-store`, `nosniff` and the download security policy.

The export re-reads stored claims and their current statuses. It uses Bilimbi's 17 columns: `batch_id`, `claim_id`, `employee_number`, `employee_name`, `category`, `claim_type_code`, `claim_type_name`, `incurred_on`, `currency`, `claimed_amount`, `approved_amount`, `receipt_number`, `description`, `status`, `approved_at`, `reimbursed_at`, `payment_reference`. It uses CRLF rows and RFC 4180 quoting. Formula prefixes (`=`, `+`, `-`, `@`, tab, carriage return or newline), including formulas after leading whitespace, receive an apostrophe before quoting. CSV timestamps are UTC without a zone suffix, as in Bilimbi.

Migration `people-claims-003-workflow-history` makes history rows and batch records immutable. Once handed off, a claim cannot change its batch, approved amount or currency; a batch holds one company and currency. Reimbursement can still add payment facts.

## Events for other Domains

Approval records **`people/claims.request_approved` v1** and payment records **`people/claims.request_reimbursed` v1**, using Base Events inside the same deciding transaction. This includes every individual claim paid through `reimburse_batch`. A refusal publishes nothing. Repeated decisions refuse by status and create no duplicate event.

Both events have subject `claim:{id}`. Their keys are `claim:{id}:approved` and `claim:{id}:reimbursed`, unique within the publisher and tenant. The envelope carries the tenant and version. The JSON payload shared by both is:

```json
{
  "company_id": 12,
  "claim_id": 34,
  "reference": "CLM-34",
  "currency": "<company currency>",
  "approved_amount": "80.00",
  "approval_date": "2026-10-22",
  "category_code": "<company category code>"
}
```

IDs are JSON integers. `reference` is the stable `CLM-{claim_id}` reference. The approved amount is an exact decimal string, never a floating-point number. `approval_date` is the UTC date of the approval. The reimbursement event also includes `reimbursed_at` (ISO 8601 UTC string) and `payment_reference` (string or null). Neither payload includes employee names, contacts or receipts. Finance consumes these recorded facts into its own projection without a dependency on the People container.

## Workflow proof

The appended PC2 block in `fixtures/walkthrough/src/claim_workflow.rs` runs on the same disposable database. It creates its own catalog and policies through the facade, submits and decides claims, groups currencies, checks CSV quoting and formula neutralisation, and verifies exact event payloads and ordered history. It also proves period limits, duplicate receipts, confirmed duplicates, own-claim refusals, batch rollback, grant revocation, account unlinking and relinking to an inactive employee. PostgreSQL refuses changes to the stored history and hand-off records.

## Claim operations

Open `/people/claims/operations` to decide claims, record payments, and group approved claims for hand-off. The Time and expenses menu shows Claim operations with `people.claims.approve`.

Choose a company and one of five tabs: Awaiting decision, Approved, Reimbursed, Rejected, or Hand-off batches. Each tab is a `Tab::patch` link, so a copied address and Back restore the selected company and queue. The queues keep Bilimbi's 500-record bound and order. Every list uses `FlexTable` with `ListState` and `ListColumns`, including waiting currency totals and batch records.

Approve and Reject open a reason dialog. Approval accepts an optional lower amount; the owner requires a reason for a reduction. Rejection always needs a reason. Mark reimbursed accepts an optional payment reference. Review opens `ConfirmDialog`; only its confirmation runs the write. Cancel and Escape discard it. The server holds the company, record, action and reviewed values; a confirmation payload cannot replace them. Changing company or tab discards an unfinished action.

The Approved tab can create one hand-off batch per waiting currency. Hand-off batches lists stored batches, links directly to the authorized CSV download, and can mark the batch's remaining approved claims reimbursed. Without `people.claims.reimburse`, payment, batch and download controls are absent, and the batches tab says “Hand-off batches need the reimbursement permission.”

Every write-shaped event rechecks the queue permission and the action's permission for the company. The Claims facade checks its permission again in the transaction. A revoked grant, stale claim, own claim or archived company returns a page message and closes the confirmation. Flash messages survive table and tab events until dismissed or replaced.

For browser proof, first run the default People walkthrough on a disposable database. Initialize its stored Dev role with `BILIBI_ENV=development ./target/debug/bilibi-web seed-production`, then run `BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin claim-operations`. It creates separate records through the Claims facade and gives the fixture viewer queue access with reimbursement explicitly denied. Sign in as the fixture manager to exercise all five tabs, or as the viewer to check withheld controls.

While a manager review is open, run the same fixture binary with `revoke-reimburse` or `revoke-approve` to prove that the next event refuses it. Restore the grant with `restore-reimburse` or `restore-approve`. These commands call Authz and refuse a database without the walkthrough accounts.

## My claims

Open **People → My work → My claims** to submit an expense and follow its decision. The page uses the signed-in account’s company and working employee; neither can be chosen in the form.

The open claim types table shows each category, currency, limits and receipt rule. The submission form asks for the type, expense date, amount, currency, receipt number and description. When the same type, date and amount already exist, it keeps the entered details and asks you to confirm that this is a separate expense. A repeated receipt number is refused even when that confirmation is checked.

Submitted claims show their status, approved amount and decision reason. A reimbursed claim also shows its payment date. **Withdraw** first asks for confirmation; Cancel and Escape leave the claim unchanged. The server keeps the selected claim, and confirmation checks ownership and status again through `Claims::withdraw_request`.

Both lists use `FlexTable` with `ListColumns` for column selection, order and compact rows. Bilimbi’s page has no sorting, filters or pagination, so those controls are absent here. `ListState` validates the page’s URL state. The page is available in a workspace tile and declares its facade writes through `PageWrites` with a pending operation-contract allowance.

Every page update resolves the account’s current employee link again. Every write event checks `people.claims.submit` before parsing input, and the facade checks again under its locks. A revoked grant or removed link clears the private rows, form and pending withdrawal. Refusals stay on the page as messages; field validation also appears beside the fields and in an error summary.

For browser checks on a disposable database, first run the default People walkthrough, then run `cargo run -p bilibi-people-walkthrough --bin claims-page -- relink` followed by `cargo run -p bilibi-people-walkthrough --bin claims-page -- prepare`. The relink step restores the staff account after other slices’ account-link refusal proofs. Sign in as `staff@example.test` to see submitted, partly approved, rejected and reimbursed expenses, plus three open receipt rules. The same binary accepts `revoke`/`restore` and `unlink`/`relink` to change that account while its page is open.

## Claim policies

Open **People → Settings → Claim policies** at `/people/claims/setup` to set up expenses for a company. Choose a company, save its currency codes, then add categories, claim types, assignments and effective-dated policies.

The five lists use `FlexTable` with `ListState` and `ListColumns`, including the currency list. Each table has column selection, ordering and compact rows. Bilimbi's setup page has no sorting, search or pagination, so those controls are absent. Company selection stays in the address and works in a workspace tile.

An assignment opens assigned-only claim types to selected employees. Its row has two shared multi-selects and **Save members**. Empty selections remove all members. The page reads the event's exact keys and bounds both arrays at 1,000 entries; selections after the first 64 fields are retained. Compact rows show member counts; return to normal row height to edit members or end dates.

**End** on an open policy or assignment reviews its end date in `ConfirmDialog`. Cancel and Escape leave the record unchanged. The server keeps the selected company, record and date; confirmation cannot replace them. The facade refuses an end before the start or, for a policy, before an existing claim's expense date. End dates cannot be changed after saving.

Every write-shaped event rechecks `people.claims.manage` for the selected company, and the Claims facade checks again under the company lock. Switching company discards unfinished forms and confirmations. Revoked permission clears private setup rows and prevents the next write. An already-selected archived company remains readable, with a read-only notice and write controls withheld; a stale write receives the archived-company refusal. Form refusals retain entered values, with validation beside the fields and in an error summary. Messages survive table events until dismissed or replaced.


The PC4 walkthrough runs the registered live page against those records: `cargo run -p bilibi-people-walkthrough --bin claim-setup -- proof`. It creates 70 claim types at Company 002, posts membership arrays after 80 unrelated fields, checks empty and malformed selections, and exercises cancellation, revoked permission, company changes and tampered confirmations. `revoke` and `restore` change the manager's setup permission while the browser stays open. `archive` archives disposable Company 002; `readonly` then proves its setup remains readable and forged stale writes cannot mutate it.
