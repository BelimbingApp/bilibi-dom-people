# Employee workspace

Employee Workspace lists a company's employees and keeps their People work details, portal eligibility, change requests and personal saved views.

For example, open **People → Team → Employees**, search by name or number, choose a status, then save that view. **Add employment** lets you choose a Person and their affiliation with the selected company, then enter the employment number, type, status, dates and supervisor. Types come from the company's active `employment_type` references. The dialog calls Workforce's employment creator, which obtains Core Affiliation's proof inside the write transaction.

The directory includes every employment status. The table supports sorting, column settings, rearrangement and compact rows. Search, status, company and sort stay in the URL. Bilimbi's directory has no pagination; this page likewise lists every matching row. Each employee name is a shared record link to `/people/employees/{id}`. The record opens an Employee workbench with read-only identity and contacts, inline employment and People work facts, profile change requests and History.

## Who can use it?

| Capability | Permits |
| --- | --- |
| `people.employees.view` | Directory, employee work facts, request history and the account's own saved views |
| `people.employees.manage` | Work profile, portal eligibility and submitting change requests |
| `people.employees.review` | Approving or rejecting a pending request |
| `people.workforce.employment.view` | Employment dates and links to Core-owned identity records |
| `people.workforce.employment.manage` | Creating and updating employment through Workforce |
| `admin.person.list`, `admin.affiliation.list` | Choosing the Person and affiliation in the employment dialog |

The first three capabilities are granted to `tenant_owner`, as in Bilimbi. **Each call checks permission again.** A grant revoked while a page is open refuses its next action. Company selection uses Workforce authorization and Core Company's target rules; a company outside the account's reach cannot be selected.

## Editing an employee

Open an employee from the directory. Click a number, type, status, date, supervisor or department to change it. Dates use `YYYY-MM-DD`; clear an optional value to remove it. Employment edits need both Workforce employment view and manage permissions. The supervisor choices contain working employees of the same company. Workforce refuses reporting cycles and an end date before the start date.

Location, arrangement, notes, portal eligibility and its reason also edit in place. Enter or leaving a text field saves it; Escape cancels. Changing a choice saves it. A refused edit keeps the draft so you can correct it. If someone else changes the record first, refresh before saving again.

Person name and contacts, and the affiliation's designation and work contacts, stay read-only. Core's own read permissions and contact restrictions still apply. When employment view is unavailable, the directory's name, number, status, type and permitted work email remain visible. Portal eligibility does not create an account or grant a role.

Use **Record request** to propose an identity change. The requests table keeps all decisions and supports sorting, column settings and compact rows. **Approve** and **Reject** first open a confirmation; the server keeps the selected request and decision until you confirm or cancel. The deciding call checks that the request is still pending.

**History** beside Employment, Work profile and Portal access opens the shared audit disclosure when audit viewing is permitted. Profile and access history use their own stored row ids, rather than assuming those ids equal the employment id. Request decisions remain in the requests table.

## Calling the facade

Resolve `EmployeeWorkspace` from `people/employee_workspace.employee_workspace`. Every method takes the sealed `Scope` and explicit company; employee methods also take the employment id. An employee id always names `people_workforce_employments.id`.

| Method | Result |
| --- | --- |
| `employees`, `employee` | Directory facts through Workforce, including status, type and supervisor |
| `work_profile`, `put_work_profile` | Optional work location, arrangement and notes |
| `work_profile_record`, `update_work_profile` | Stored row id, value and version; change one field at that version |
| `access`, `put_access` | Portal eligibility and reason |
| `access_record`, `update_access` | Stored row id, value and version; change one field at that version |
| `change_requests`, `request_change` | Request history and a new pending proposal |
| `review_change` | One review decision, either `approved` or `rejected` |
| `saved_views`, `save_view`, `delete_view` | Only the signed-in account's views in that company |

Requester, reviewer and saved-view owner come from the scope. Callers cannot supply them. Work location is limited to 200 characters, arrangement to 120 and notes or reasons to 2,000. A change request names `full_name`, `short_name`, `email` or `mobile_number`, with a required proposed value of at most 500 characters. Saved-view names are unique per account and company and limited to 120 characters; search is limited to 200 and status to 40.

**Approving a change request records a decision.** It does not edit Core's Person or Affiliation. An operator applies approved identity changes through the owning Core workflow. Portal eligibility also does not create a login or grant a role. Bilimbi allows a requester to review their own request; a second review is refused with `AlreadyReviewed`.

## How writes stay consistent

The facade authorizes inside each `MutationBoundary` transaction. It locks the writable company and, for employee facts, the employment through Workforce before touching its own rows. Missing employees, employees of another company and archived companies refuse the write. Every affected row produces a `Change`, with an empty object on the absent side for creation or deletion. Saved views use `begin_view_state`, while still enforcing permission, company locks and audit capture. Removing a view in the page requires a server-held selection and `ConfirmDialog`.

Inline profile and access updates take a `FactRecord` version. An absent row has version zero; the first write creates version one. Every write, including `put_work_profile` and `put_access`, advances its stored version. Under the employment lock, an inline update compares that version and changes only its named field; a stale version returns `WorkspaceError::Stale`. Migration `people-employee-workspace-002-inline-versions` adds these fences without changing the applied initial migration.

The private store owns validation and SQL. The migration `people-employee-workspace-001-employee-workspace` creates Bilimbi's four tables: `people_employee_workspace_profiles`, `people_employee_accesses`, `people_employee_change_requests` and `people_employee_saved_views`.

## Running the proof

Mount this repository at `apps/domains/people` in Bilibi, discover modules, and run `BILIBI_ENV=development cargo run -p bilibi-people-walkthrough` against a disposable database. Its PE1 block exercises profile and eligibility writes, single review, saved-view ownership, revoked grants and company refusals. The PE2 block proves version checks, field merging, validation, read-only access, request review and audit capture. Then run the application and use `/people/employees` and an employee record.

## Where to go next

See [Workforce](../workforce/README.md) for employment creation and directory ownership, and [Reference Data](../reference_data/README.md) for the company's employment-type choices.

## Queued writes

Profile, portal-access, change-request and review writes check their permission again after Workforce locks the employee and company. Saved-view writes recheck after the company lock too. A permission revoked while a write waits cannot authorize the queued write. The `people-sweep-a` walkthrough command proves this with a queued profile write.
