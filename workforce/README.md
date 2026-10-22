# Workforce

Module ID: `people/workforce`, crate `bilibi-people-workforce`. The native
workforce boundary every People module builds on (a company's identity and
its working employees as `ReadResult`s, named through Core), the employment
record People owns, the company's working statuses, and the `Authorization`
helpers every People facade authorizes through. Other People modules read
and write through this module's facade; none of them reads its table. This
README is the owner contract; the Bilimbi source it ports is
[`workforce/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/workforce/docs/README.md).

This is the scaffold slice (PS0 of the port plan). PW1 adds
`update_employment`, `employment`, `employment_summary`, self-service
(`self_employee`, `authorize_self`, `with_self_employee_lock`),
`supervisor_chain`, `in_reach` and the `positions` scan over the
`people/organisation.positions` port; PW2 adds the settings page and the
`people/workforce.directory` port the People connector reads.

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
| `working_statuses` | none | The statuses the company counts as working staff |
| `put_working_statuses` | `people.workforce.settings.manage` | Replaces them: a non-empty subset of the five, stored in canonical order, else `InvalidStatuses` |
| `create_employment` | `people.workforce.employment.manage` | Employs a person's affiliation with the company (`NewEmployment`), returning the `Employment` |
| `employee_statuses` | | The five statuses: `pending`, `probation`, `active`, `inactive`, `terminated` |

Every read exposes only a live company of the scope's tenant: a missing,
deleted, archived, cross-tenant or malformed company id is `NotFound`, and
they are indistinguishable. (Core Company's capability-free reads tell
whether a company is archived, so a pending or suspended company is
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
| `authorize(client, scope, company_id, capability)` | The capability now (Base Authz) and the company axis (Core Company's `organisation::authorize_target`: the actor's own company, or any company of the tenant with `admin.company.tenant-wide.manage`). `Unauthorized` for a denied or unknown capability or a company outside the reach; `NotFound` for a company the scope cannot see |
| `authorize_company(..)` | The same, returning the `CompanySummary` (id, name, code, archived) for a page that renders it |
| `selectable_companies(client, scope, capability)` | The unarchived companies the actor may target with the capability, by name (Bilimbi `list_selectable_companies/2`): none without the capability, the own company, or the whole tenant with the tenant-wide reach |
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

## Employment records

People owns employee numbers, employment types, statuses, employment start
and end dates, the supervisor, an optional Core Company department and free
metadata. One row of `people_workforce_employments` is one employment; its
id is the employee id every other People table keys on. It references the
Core Person (`person_id`) and that person's affiliation with the employer
(`affiliation_id`); the employer company is the affiliation's company,
which the scoped foreign key into `person_affiliations` keeps true for
good. The five statuses are the one fixed classification; employment types
are a company's reference data (the `employment_type` kind of
`people/reference_data`, which PD1 adds), kept here as text.

`create_employment` takes the sealed scope, the explicit company and a
`NewEmployment` (person, affiliation, number, type, status, optional dates,
supervisor, department, metadata). It authorizes
`people.workforce.employment.manage` for the company, then inside its
mutation transaction holds the company's writable lock (an archived company
is `CompanyArchived`), takes the Affiliation owner's proof of the
affiliation under the owner's locks and checks it names this person and
this company (`AffiliationMismatch`), refuses an agent Person (`NotHuman`:
employees are humans) and an affiliation already employed
(`DuplicateAffiliation`), and reports as `Invalid` field errors, in
Bilimbi's words, a blank or overlong number or type ("can't be blank",
"should be at most 255 character(s)"), an unknown status ("is invalid"), an
end before the start ("must be on or after employment start"), a taken
number ("has already been taken") and a supervisor who is not an employment
of the company ("is invalid"). The row is recorded through
`MutationBoundary` as a created `people_workforce_employment` change. A
missing grant is `Unauthorized`; a company the scope cannot see is
`NotFound`.

**Interim proof.** The port plan's employment proof is Core Affiliation's
`lock_for_employment_at` (Platform row PP2): the named company must be a
live company of the tenant the actor may target, the affiliation linked and
of that company, the Person not retired or system. The pinned Platform does
not carry it yet, so `create_employment` takes
`AffiliationService::lock_for_account_binding`, which proves the same
person, company and link under the same owner locks (Company → Person →
Affiliation) and refuses an unlinked affiliation, a retired or system
Person and an archived company; the actor's reach over the company is what
`authorize` already proved. It needs the owner's `admin.affiliation.prove`
grant, which the Tenant Owner holds. PW1 replaces it with the PP2 proof and
adds `update_employment` (version-fenced).

## Working statuses

`people.workforce.working_statuses` is a company-scoped Base Setting
(array, default `["probation", "active"]`, never null, editor group
"People", capability `people.workforce.settings.manage`). A value is a
non-empty list of distinct statuses among the five; `put_working_statuses`
stores it in canonical order. Only employees in the company's working
statuses are exposed by the working-staff reads, both as employees and as
supervisor references. Operators holding the settings capability edit it
per company at `/people/workforce/settings` once PW2 lands the page; until
then through the Settings editor.

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
| `department_id` | `(tenant_id, department_id)` references `company_departments` |
| `metadata` | jsonb |
| `version` | ≥ 1, for the version fence of PW1's `update_employment` |
| `inserted_at`, `updated_at` | UTC |

Indexes: `(tenant_id, company_id)`, `(company_id, status)`,
`(supervisor_employment_id)`, `(tenant_id, person_id)`. Dates are bound and
read as ISO text (`$n::text::date`, `to_char`): the workspace's
`tokio-postgres` has no date feature, and a Domain must not turn one on.

## Dependencies

`base/authz`, `base/database`, `base/module_registry`, `base/party_ids`,
`base/principal_directory`, `base/settings`, `base/tenancy`,
`core/affiliation`, `core/company`, `core/person`; the Cargo manifest lists
the same edges. No People sibling: every other People module depends on
this one.
