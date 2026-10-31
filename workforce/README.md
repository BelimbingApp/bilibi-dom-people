# Workforce

Module ID: `people/workforce`, crate `bilibi-people-workforce`. The native
workforce boundary every People module builds on (a company's identity and
its working employees as `ReadResult`s, named through Core), the employment
record People owns, the company's working statuses, the `Authorization`
helpers every People facade authorizes through, self-service, and the
position scan Organisation answers. Other People modules read and write
through this module's facade; none of them reads its table. This README is
the owner contract; the Bilimbi source it ports is
[`workforce/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/workforce/docs/README.md).

The settings page is `/people/workforce/settings`. The People connector reads company identities and bounded employee and position pages through `people/workforce.directory`.

## Public API

Resolve `Workforce` from the registry as `people/workforce.workforce`
(`bilibi_people_workforce::SERVICE`) and call it with the sealed `Scope`
you were handed and an explicit platform company id. Results are typed
records; the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `company` | none | The company's native identity (`Company`: both company axes, name, code) as a current `ReadResult` |
| `employees` | none | The company's working employees by id (`Employee`), named through Core |
| `employees_by_ids` | none | The working employees among at most 1,000 positive, deduplicated ids, without reading the whole workforce; more, or a non-positive id, is `InvalidOptions` |
| `employee` | none | One working employee; another status, another company or missing is `NotFound` |
| `direct_reports` | none | The working employees reporting to one working employee; a supervisor who is not a working employee is `NotFound` |
| `supervisor_chain` | none | A working employee's supervisors, nearest first, through working supervisors only, at most 12 levels (`CHAIN_DEPTH`, Bilimbi's reach depth); an employee who is not working is `NotFound` |
| `in_reach` | none | Whether one working employee is below another in the supervisor chain (Bilimbi `Access.within?/2` for a team reach): never the manager themselves, `false` for an employee the reads do not expose |
| `working_statuses` | none | The statuses the company counts as working staff |
| `put_working_statuses` | `people.workforce.settings.manage` | Replaces them: a non-empty subset of the five, stored in canonical order, else `InvalidStatuses` |
| `employment` | `people.workforce.employment.view` | One employee's private employment facts (`Employment`) at any status; another company or missing is `NotFound` |
| `employment_summary` | `people.workforce.employment.view` | The company's employment counts (`EmploymentSummary`: total, working, and every status with its count) |
| `create_employment` | `people.workforce.employment.manage` | Employs a person's affiliation with the company (`NewEmployment`), returning the `Employment` |
| `update_employment` | `people.workforce.employment.manage` | Changes an employment (`EmploymentChange`, fenced on the version the caller read), returning the `Employment` |
| `self_employee` | none | The working employee the signed-in account is linked to in the company (`SelfEmployee`), resolved now; else `NotLinked` |
| `authorize_self` | the self-service capability given | `authorize`, then `self_employee` |
| `lock_self_employee` | the self-service capability given | The same inside the caller's mutation transaction, with the employment row locked and the link proved again under the lock |
| `with_self_employee_lock` | the self-service capability given | Opens a mutation transaction for the caller's action, takes `lock_self_employee`, runs the caller's write and commits |
| `positions` | none | One page of the company's positions through the `people/organisation.positions` port (`PositionScan` → `PositionPage`); `Unavailable` while Organisation is not mounted |
| `positions_available` | | Whether a mounted module answers that port, for a page that withholds a control |
| `employee_statuses` | | The five statuses: `pending`, `probation`, `active`, `inactive`, `terminated` |

Every read exposes only a live company of the scope's tenant: a missing,
deleted, archived, cross-tenant or malformed company id is `NotFound`, and
they are indistinguishable. (Core Company's capability-free reads tell
whether a company is archived and its status, so a pending or suspended company is
visible here, as its accounts are not frozen either.) The working-staff
reads need no capability, as Bilimbi's do: they reveal only the employees
in the company's working statuses, and only the facts below.

An `Employee` carries `people/native` references (`Reference`: source id,
`type`, immutable id as text) for itself, its company and, only while the
supervisor is a working employee of the same company, its supervisor; both
company axes (`platform_company_id` and `workforce_company_id`, the same
Core Company today); the employee number; `display_name`, the Person's name
as Core names it; and `email`, the affiliation's work email. Employee
identity never implies a login actor.

### Reading on a caller's connection

Use `company_on(client, scope, company)` and `working_employee_ids_on(client, scope, company, ids)` inside a port or transaction. They apply the same live-company and working-status rules on that client's view. The ids read is bounded to 1,000 positive ids and returns a current `ReadResult<Vec<i64>>`; it skips names and contacts when you need only membership. Organisation uses these reads so a position scan does not rent nested pool connections or miss a caller's pending employment changes.

### Names and contacts through Core

Names, contacts and titles stay in Core Person and Affiliation and are not
copied. A read names its employees through Base's Principal Directory
(`base/principal_directory`, the same owner-bound naming every Base screen
uses: capability-free, bounded to the ids asked for, tenant-scoped, a
retired Person left unresolved and shown by its durable label), and reads
the work email through the Affiliation owner's own `search`, which honours
its field restrictions and needs `admin.affiliation.list`: an actor without
that grant sees `email` absent, never a contact Core would have masked.

### `ReadResult`

`ReadResult<T>` is a value with its `Freshness`: `Current` for every native
read, `Stale(last_confirmed_at)` for a cached copy an adapter serves, or
`Unavailable(reason)` with no value. Call `require_current` to obtain the
value only when it is current; a stale or unavailable result is the
`NotCurrent` refusal a caller that must not act on old facts reports. The
People connector is the first consumer that answers anything but current.

## Authorization and company scope

`bilibi_people_workforce::authorization` is the one boundary every People
facade authorizes through, at the public API so pages, jobs and other
adapters share it. Pass the sealed `Scope`, never an actor or actor id; a
system principal acts through `bilibi_authz::SystemScope`, a different type,
so a system scope cannot reach these helpers at all.

| Function | Does |
| --- | --- |
| `authorize(client, scope, company_id, capability)` | The capability now (Base Authz) and the company axis (Core Company's `bilibi_company::authorize_work_target`: an explicit desk in that company). `Unauthorized` for a denied or unknown capability or a company outside the reach; `NotFound` for a company the scope cannot see |
| `authorize_company(..)` | The same, returning the `CompanySummary` (id, name, code, status, archived) for a page that renders it |
| `selectable_companies(client, scope, capability)` | The unarchived companies the actor may target with the capability, by name (Bilimbi `list_selectable_companies/2`): none without the capability, otherwise only the person’s explicit desks |
| `company_choices(client, scope, capability, selected)` | Active companies for a People company chooser, plus the already-selected pending or suspended company if Core still authorizes it. Archived companies are never offered, even when selected or deep-linked. Keep the selection from server state or the addressed record, never a write form. Record discovery keeps using `selectable_companies` so existing records remain reachable |
| `allowed(..)` | Whether `authorize` would succeed, for deciding which controls a page renders; never authority for an operation |

A page rechecks its route capability before acting and a `can_*` only
decides which controls it renders: a grant revoked while the page stays
connected refuses the very next operation, because every write and every
read of private employee facts calls `authorize` with the scope it was
handed. The Tenant Owner role holds the two employment keys, as in
Bilimbi; an administrator grants the settings key:

| Capability | Tenant Owner |
| --- | --- |
| `people.workforce.settings.manage` | |
| `people.workforce.employment.view` | yes |
| `people.workforce.employment.manage` | yes |

The Authz domain is `people` ("People domain modules"); every People
module's keys live under it. `view` and `manage` are verbs Base declares.

### Self-service

Self-service resolves the freshly proved staff Person and its employment in the requested workplace. A staff account has no employment-affiliation facet. The employment retains its own Core affiliation, which Workforce proves through the Affiliation owner.

- `self_employee(scope, company_id)` re-proves the account through `Authentication::reprove`, keeping the impersonator, checks its desk, and finds that Person's employment in the company's working statuses. It returns the employment ID, Person and business affiliation. A missing Person, lost desk, unavailable affiliation or non-working employee is `NotLinked`; ambiguous employments also refuse. A missing company is `NotFound`.
- `authorize_self(scope, company_id, capability)` is `authorize` then
  `self_employee`, for a self-service read.
- `lock_self_employee(&tx, scope, company_id, capability)` is the half of
  Bilimbi's `with_self_employee_lock/4` a sibling facade calls from its own
  `MutationTransaction`: it authorizes, resolves the employee now, holds the
  company's writable lock (an archived company is `CompanyArchived`), locks
  the employment row (`FOR NO KEY UPDATE`, the lock `update_employment` and
  the other self-service writes wait on), then proves the link and the
  working status again under the lock; a link that changed in between is
  `NotLinked`. Take it before the sibling's own row locks: the order is
  Company → Employment → the sibling's rows.
- `with_self_employee_lock(scope, company_id, capability, action, write)`
  opens the transaction itself for the caller's audited action
  (`people.<module>.<verb>`), takes `lock_self_employee`, calls `write(&mut
  tx, &SelfEmployee)` and commits what it returns; an error rolls everything
  back. `write` records its rows through `tx.mutate` as every People write
  does, and returns a boxed `Send` future (`SelfWrite<'_, T>`), the same
  shape `MutationTransaction::mutate` takes.

An account unlinked, relinked or frozen while a page stays connected is
refused on its very next self-service call, and a self-service write that
lost the link between resolving and locking rolls back.

## Employment records

People owns employee numbers, employment types, statuses, employment start
and end dates, the supervisor, an optional Core Company department and free
metadata. One row of `people_workforce_employments` is one employment; its
id is the employee id every other People table keys on. It references the
Core Person (`person_id`) and that person's affiliation with the employer
(`affiliation_id`); the employer company is the affiliation's company,
which the scoped foreign key into `person_affiliations` keeps true for
good. The five statuses are the one fixed classification. Employment types
are a company's reference data (the `employment_type` kind of
`people/reference_data`), kept here as trimmed text of at most 255
characters: Reference Data depends on Workforce, so Workforce cannot read
that kind back, and the Employees page (`people/employee_workspace`), which
depends on both, offers and checks the kind's entries before it calls this
facade.

### `create_employment`

Takes the sealed scope, the explicit company and a `NewEmployment` (person,
affiliation, number, type, status, optional dates, supervisor, department,
metadata). It authorizes `people.workforce.employment.manage` for the
company and validates the shape, then inside its mutation transaction:

1. classifies an archived company as `CompanyArchived` (a read, before any
   lock, because the proof answers every refusal with one word);
2. takes the Affiliation owner's employment proof at this company,
   `AffiliationService::lock_for_employment_at(tx, scope, affiliation,
   company)` (Platform row PP2), which locks in the global order tenant →
   Company → Person → Affiliation and refuses a company outside the actor's
   reach, an archived company, an affiliation of another company, an
   unlinked affiliation and a retired or system Person (`Unauthorized`; a
   missing or other-tenant company is `NotFound`); it needs the owner's
   `admin.affiliation.prove`, which the Tenant Owner holds;
3. checks the proof names this person (`AffiliationMismatch`) and locks the
   Person to refuse an agent (`NotHuman`: employees are humans);
4. holds the company's writable lock explicitly, the lock every People
   write holds (already taken by the proof, so it never waits);
5. decides the refusals with plain reads: an affiliation already employed
   (`DuplicateAffiliation`) and, as `Invalid` field errors in Bilimbi's
   words, a blank or overlong number or type ("can't be blank", "should be
   at most 255 character(s)"), an unknown status ("is invalid"), an end
   before the start ("must be on or after employment start"), a taken number
   ("has already been taken"), a supervisor who is not an employment of the
   company ("is invalid") or not a working one ("must be a working employee
   of the company") and a department that is not one of the company's ("is
   invalid");
6. inserts through `MutationBoundary`, recorded as a created
   `people_workforce_employment` change.

A missing grant is `Unauthorized`; a company the scope cannot see is
`NotFound`. The proof's sentences are the port's: Bilimbi's Core Employee
form had no supervisor or department rule beyond the foreign key.

### `update_employment`

Takes the scope, the company, the employee id and an `EmploymentChange`:
`version`, the row version the caller read, and for the number, type,
status, dates, supervisor, department and metadata either `None` (keep) or
`Some(value)`; the nullable facts take `Some(None)` to clear. The person and
affiliation never change (an employment at another company is a new
employment). It authorizes `people.workforce.employment.manage`, then
inside its mutation transaction holds the company's writable lock, locks the
row (`FOR NO KEY UPDATE`; another company or missing is `NotFound`), refuses
a version that moved (`Stale`: "The record changed. Reload before saving"),
validates the row as it will be (the field errors above, plus "cannot be the
employee themselves" and, for a supervisor who reports to this employee
directly or through others, "would create a reporting cycle"), and updates
through `MutationBoundary` with `version + 1`, recorded as an updated change
with the old and new values. Ending an employment is a change to the status
`terminated` with the last day as `employment_end`; the row stays, readable
through `employment` and counted by `employment_summary`, and leaves the
working-staff reads.

### Positions

`positions(scope, company_id, &PositionScan { as_of, cursor, page_size })`
is Bilimbi's `positions/4` with `cursor:`, served by Organisation through
the port `people/organisation.positions` (the provider is
`people/organisation`; Bilimbi registered a reader at application start).
Workforce checks the live company, then calls the port on its own client
with the caller's scope; the provider checks its own rules. `PositionPage`
carries `positions`, `next_cursor` (`None` after the last page) and
`high_water_id`. Start with `PositionScan::from_start(as_of)` and continue
with `continued(next_cursor)` and the same company and day until the cursor
is `None`; only after reading every page may a consumer treat ids at or
below `high_water_id` it did not see as gone, and ids above it wait for the
next scan. Page size defaults to 50 and accepts 1 to 100 (`InvalidOptions`
otherwise); a malformed cursor or one of another tenant, company or day is
`InvalidCursor`; no mounted provider is `Unavailable`, and
`positions_available()` says so beforehand. Mounting People without
Organisation is a valid composition: the scan is simply unavailable.

The documented shape, which Organisation answers with its own serde types:

```json
request  {"company_id": 1, "as_of": "2026-10-22", "cursor": null, "page_size": 50}
answer   {"positions": [{
             "reference": {"source_id": "people/native", "type": "position", "stable_id": "7"},
             "company_reference": {"source_id": "people/native", "type": "company", "stable_id": "1"},
             "platform_company_id": 1, "workforce_company_id": 1,
             "code": "OPS-LEAD",
             "parent_reference": {"source_id": "people/native", "type": "position", "stable_id": "3"},
             "title": "Operations lead", "version": 2,
             "assignments": [{
                 "reference": {"source_id": "people/native", "type": "assignment", "stable_id": "11"},
                 "employee_reference": {"source_id": "people/native", "type": "employee", "stable_id": "2"},
                 "kind": "substantive"}],
             "assignments_incomplete": false,
             "vacant": false}],
          "next_cursor": "MTox…", "high_water_id": 7}
```

`parent_reference`, `title` and `version` are null for a root position or
a day before any version; `assignments` lists the working holders in effect
on the day (`assignments_incomplete` when the page's holders were cut
short); `vacant` is true when no substantive holder occupies it. The
request's `company_id` is a live company Workforce already proved;
`as_of` is an ISO date; `cursor` is the opaque string of the previous page
or null; `page_size` is 1 to 100. The provider answers `Invalid` for a
cursor it does not accept.

## Working statuses

`people.workforce.working_statuses` is a company-scoped Base Setting
(array, default `["probation", "active"]`, never null, editor group
"People", capability `people.workforce.settings.manage`). A value is a
non-empty list of distinct statuses among the five; `put_working_statuses`
stores it in canonical order. Only employees in the company's working
statuses are exposed by the working-staff reads, both as employees and as
supervisor references, resolved as self-employees, and accepted as
supervisors. Operators holding the settings capability edit it per company
at `/people/workforce/settings` once PW2 lands the page; until then through
the Settings editor.

## Database

`migrations/001-employments.sql`, applied as `people-workforce-001-employments`:

| Column | |
| --- | --- |
| `id` | the employee id |
| `tenant_id`, `company_id` | the employer; `(tenant_id, company_id)` references `companies` |
| `person_id`, `affiliation_id` | the Core Person and affiliation; `(tenant_id, affiliation_id, person_id, company_id)` references `person_affiliations(tenant_id, id, person_id, organisation_id)`; `affiliation_id` unique |
| `employee_number` | unique in the company |
| `employment_type` | text, 255 |
| `status` | one of the five (check) |
| `employment_start`, `employment_end` | dates, end on or after start (check) |
| `supervisor_employment_id` | `(supervisor, company_id)` references an employment of the same company; never the row itself (check) |
| `department_id` | `(tenant_id, department_id)` references `company_departments`; the facade also requires the company's own department |
| `metadata` | jsonb |
| `version` | ≥ 1, moved on by every `update_employment` |
| `inserted_at`, `updated_at` | UTC |

Indexes: `(tenant_id, company_id)`, `(company_id, status)`,
`(supervisor_employment_id)`, `(tenant_id, person_id)`. Dates are bound and
read as ISO text (`$n::text::date`, `to_char`): the workspace's
`tokio-postgres` has no date feature, and a Domain must not turn one on.
The supervisor chain and the cycle check are recursive queries bounded in
depth; the row lock is `FOR NO KEY UPDATE`, so reads and foreign-key checks
from other rows do not wait on it.

## Dependencies

`base/authz`, `base/database`, `base/module_registry`, `base/party_ids`,
`base/ports`, `base/principal_directory`, `base/settings`, `base/tenancy`,
`core/affiliation`, `core/company`, `core/person`; the Cargo manifest lists
the same edges. No People sibling: every other People module depends on
this one, and Organisation answers the position scan through a port, not
an edge.

## Assignment directory

`employees_at_any_status(scope, company_id, employee_ids)` returns directory summaries for company assignment pickers. Pass `None` for the company list, or at most 1,000 positive IDs to validate members. It includes pending, probation, active, inactive and terminated employments. Names and work contacts still come through Core; private employment fields stay behind `employment` and its capability. Claims uses this read so assignments can cover employees before or after their working period. `employees`, `employees_by_ids` and `employee` continue to return only working staff.

### Employee Workspace directory

`directory_employees(scope, company_id)` returns all-status directory summaries as `DirectoryEmployee { id, employee, status, employment_type }`; it excludes private employment dates, Person ids and affiliation ids. Like the existing directory reads, it needs a visible live company and no separate employment-view grant. Employee Workspace authorizes `people.employees.view` before using it. `lock_employee(tx, scope, company_id, employee_id)` holds the company's writable lock and the named employment lock for a sibling's mutation; the sibling must authorize its own operation first.

`employment_types(scope, company_id)` requires `people.workforce.employment.manage` and reads active choices through `people/reference_data.employment_types`. It returns `EmploymentType { code, label }` or `Unavailable` if Reference Data is absent, without a cyclic dependency from Workforce to Reference Data.
## Workflow subject locks

Sibling People workflows can call `lock_working_employee(tx, scope, company, employee)` before accepting evidence about a working employee. Review and finalization can use `lock_employee_subject` to lock an employee at any status. Both hold the writable company lock first and then the employment row, return no private facts, and leave authorization to the caller's own capability check. Lock the subject before the sibling's own rows.

`linked_employee(client, scope, company)` re-proves the signed-in account and returns its employment id at any status, or `None` when unlinked. It is an identity read for subject independence checks, not permission to act. `self_employee` and self-service writes still require a working employee. The Skills assessment workflow uses these reads and locks to refuse self-decisions inside its deciding transaction.

## Independent decisions

`owns_employee(tx, scope, company_id, employee_id, capability)` lets another People facade refuse decisions on the signed-in account's own employee records. It checks the supplied permission, holds Company → Employment locks, and re-proves the account link on the caller's transaction. It returns true for a matching Person at any employment status, false for an unlinked account, and `NotFound` for a missing employee. Claims uses this to keep inactive employees' own claims out of their decision authority. Self-service writes continue to use `lock_self_employee`, which also requires a working status.

`authorization::authorize_company_proof` provides the same company target authorization on an object-safe `ProofClient`, for owner callbacks such as Training's Artifacts adapter. Pass the callback's client so its proof stays on the reservation or publication transaction. The returned company includes its archived flag; owners must refuse an archived company before granting access.

### Employment identity links

`employment_people_on(client, scope, company, affiliation_ids)` matches at most `LOOKUP_LIMIT` positive Core affiliation ids to `(affiliation_id, employment_id)` pairs, at any employment status. The company must be live and reachable. It returns a current `ReadResult` and exposes no employment facts. Skills reminders use it with Core User's proven company accounts to find recipient employment references without requiring employment-page permissions. It grants no authority to act on those employees.

`prove_self_or_direct_report` is the object-safe subject check for an Artifacts owner callback. The owner first checks its own capability and supplies the live scope Artifacts re-proved. The caller holds the company lock across the file operation and the self employment lock for self-service. Workforce resolves the working statuses through Settings, then checks the linked employment and subject on the callback transaction. It avoids taking the caller's locks again on a second connection. The method proves identity and relationship only; it grants no application permission and exposes no employment facts.

## Workforce settings page

Open `/people/workforce/settings` with `people.workforce.settings.manage` to choose a company and the employee statuses it treats as working staff. The company chooser offers active companies you may manage through `authorization::company_choices`. An already-selected pending or suspended company stays named and keeps its existing read and write rules; an archived company is never offered. Its selection is kept in `company_id` in the address, so Back and reload restore it. The page has no menu leaf, matching Bilimbi.

Save requires at least one of Pending, Probation, Active, Inactive and Terminated. The write checks permission again inside its mutation transaction and holds the writable company lock before saving through Base Settings. Success says “Working statuses saved.”; an empty selection says “Choose at least one employee status.” A revoked grant or archived company refuses the next save. Settings and employment writes recheck their permission after obtaining the company lock, including a write that waited while its grant was revoked.

Readers who also hold `people.workforce.employment.view` see employment totals, working staff totals and the five status counts in a FlexTable. The table supports column selection, reordering and compact rows; there is no sort, search or paging in Bilimbi's section. It shows zero counts too. Legacy claiming and exception lists do not apply to Bilibi's fresh schema.

## Connector directory port

The provider is `people/workforce.directory`. Call it through Base Ports with the caller's sealed scope and connection or transaction. **The connector declares only `base/ports`; it never depends on this crate or reads People tables.** The same tenant and live-company proofs as the Workforce read facade apply on every call. These public directory reads need no additional Workforce capability, as in Bilimbi; private employment facts and settings writes keep their own grants. Core still masks contacts the reader may not see.

### Company identity

Request:

```json
{"read":"company","company_id":1}
```

The answer contains `freshness: "current"` and `value`, with the existing `Company` fields: `reference`, `platform_company_id`, `workforce_company_id`, `name` and `code`. Native company axes currently both name the Core company. Keep both axes when validating a connection. A reference is `{ "source_id": "people/native", "type": "company", "stable_id": "1" }`.

### Employee pages

Start a pass with:

```json
{"read":"employees","company_id":1,"page_size":250,"cursor":null}
```

The answer contains:

| Field | Meaning |
| --- | --- |
| `employees` | At most `page_size` working employee summaries in ascending employment ID order |
| `next_cursor` | Opaque string for the next page, or `null` only when the pass is complete |
| `high_water_id` | The company's highest employment ID visible when the first page began; fixed for the pass |
| `watermark` | RFC 3339 UTC observation time minted on the first page; fixed for the pass |
| `snapshot` | `true`: enumerate the working directory on every pass, including a connector changes pass |
| `freshness` | `"current"` for the native directory |

Each employee has `reference`, `company_reference`, both company axes, `employee_number`, `display_name`, nullable `email` and nullable `supervisor_reference`. All stable IDs are immutable employment IDs as text. The supervisor reference is present only for a working supervisor in the same company. Names and contacts are read through their Core owners on the caller's client; a reader without Core's affiliation-list grant receives a null email.

Continue by passing `next_cursor` unchanged with the same company and page size. The default is 250, and the allowed range is 1–1,000. The cursor binds the tenant, company, page size, working statuses, last emitted ID, high-water ID and watermark. Renumbering an employee cannot move it across the cursor. IDs above the high-water mark wait for the next pass. A change to working-status configuration invalidates an unfinished pass, so the connector must restart it.

`snapshot: true` means a full enumeration, not an MVCC snapshot across calls: names, contacts, status and membership are read live. Apply results and deactivate unseen employees only after all pages succeeded. Never treat an unavailable, refused or interrupted pass as an empty directory. The watermark is an observation token, not an incremental change cursor; a new pass starts with `cursor: null`.

The same scan is available to People callers through `employee_page(scope, company, &EmployeeScan)` and `employee_page_on(client, scope, company, &EmployeeScan)`, returning a current `ReadResult<EmployeePage>`. Both use bounded SQL with one lookahead row rather than loading the whole workforce.

### Position pages

Request:

```json
{"read":"positions","company_id":1,"as_of":"2026-10-24","page_size":50,"cursor":null}
```

Workforce passes the scan through `people/organisation.positions` on the caller's client. The default size is 50; the allowed range is 1–100. The answer has the existing `PositionPage` fields (`positions`, `next_cursor`, `high_water_id`), plus `freshness: "current"`, `snapshot: true` and the per-pass `watermark`. Position and assignment fields are the same as in the Positions contract above. The outer cursor preserves the observation time and binds tenant, company, day and page size while carrying Organisation's opaque keyset cursor. Continue with the same request bounds until `next_cursor` is null.

Without Organisation, the answer is `freshness: "unavailable"`, `reason: "organisation_unavailable"`, `positions: []`, `next_cursor: null`, `high_water_id: null`, `snapshot: false` and the observation `watermark`. Stop the whole connector pass without applying anything. When Workforce itself is absent, Base Ports answers `PortError::Unavailable` before any provider is called.

### Refusals

Unknown reads or fields, malformed values, out-of-range page sizes and invalid or mismatched cursors are `PortError::Refused(Error::Invalid)`. Missing, archived, deleted, cross-tenant companies are `NotFound` or `Forbidden`, according to the company owner's proof. No request field can replace the caller's scope or select a different actor. A database failure remains a failure; it never becomes an empty successful page.

## PW2 proof

Run `cargo run -p bilibi-people-walkthrough --bin workforce-settings -- setup` against an empty disposable database. The PW2 block exported at the end of `fixtures/walkthrough/src/lib.rs` exercises this port: both company axes, one-row employee pages, stable watermarks, bounded continuation, mismatched cursors, Core contact permissions, pending Core contact changes on the caller’s transaction, archived-company refusals and the Organisation pass-through. Open the settings page as the fixture administrator to exercise company selection, status saves, empty-selection refusal and the counts table.

While the manager’s page stays open, run the same binary with `revoke` or `restore` to change its settings grant through Authz. The `archive` action archives Company 004 through Core for a mounted-page refusal check. These controls are fixture commands, never application routes.

## People sweep A

Run the default People walkthrough on a fresh disposable database, then run `cargo run -p bilibi-people-walkthrough --bin people-sweep-a` on that database. The sweep keeps pages mounted, revokes each write permission through Authz, sends the next event with the original signed-in scope, and restores permission. A refusal must leave the page usable. It covers the Employees directory and detail page, Workforce settings, People references, Organisation, the four Attendance settings pages, Rosters, Attendance approvals, Leave approvals and policies, and Claim policies and operations.

The same command probes the append-only Leave ledger and Leave and Claim histories, immutable published Skills scales and published profile history, overlapping leave slots, and overlapping substantive assignments. Every probe requires a real record and its expected PostgreSQL error code. Each probe rolls back, so a missing guard fails the proof without changing the records used for browser checks.

The sweep also queues Workforce settings, Reference data, Employee Workspace and Leave catalog writes behind a grant revocation. Use `people-sweep-a queued-grant` to run the isolated Workforce settings regression. It also queues a People settings write behind Core’s company archival and requires `CompanyArchived` after the lock clears. It archives the disposable fixture’s Company 062, so run the command once per fresh walkthrough database.

The command leaves the ordinary manager account ready for Chrome. Compare the Team, Time and expenses, and Settings routes with Bilimbi in light and dark themes. Repeat with a restricted account, then use the individual module walkthrough controls for self-approval, account unlinking and company archival while a page stays open. Screenshot and interaction evidence belongs outside the repository.
