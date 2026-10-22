# Employee workspace

Employee Workspace lists a company's employees and keeps their People work details, portal eligibility, change requests and personal saved views.

For example, open **People → Team → Employees**, search by name or number, choose a status, then save that view. **Add employment** lets you choose a Person and their affiliation with the selected company, then enter the employment number, type, status, dates and supervisor. Types come from the company's active `employment_type` references. The dialog calls Workforce's employment creator, which obtains Core Affiliation's proof inside the write transaction.

The directory includes every employment status. The table supports sorting, column settings, rearrangement and compact rows. Search, status, company and sort stay in the URL. Bilimbi's directory has no pagination; this page likewise lists every matching row. Each employee name is a shared record link to `/people/employees/{id}`. The employee detail page belongs to slice PE2.

## Who can use it?

| Capability | Permits |
| --- | --- |
| `people.employees.view` | Directory, employee work facts, request history and the account's own saved views |
| `people.employees.manage` | Work profile, portal eligibility and submitting change requests |
| `people.employees.review` | Approving or rejecting a pending request |
| `people.workforce.employment.manage` | Creating employment through Workforce |
| `admin.person.list`, `admin.affiliation.list` | Choosing the Person and affiliation in the employment dialog |

The first three capabilities are granted to `tenant_owner`, as in Bilimbi. **Each call checks permission again.** A grant revoked while a page is open refuses its next action. Company selection uses Workforce authorization and Core Company's target rules; a company outside the account's reach cannot be selected.

## Calling the facade

Resolve `EmployeeWorkspace` from `people/employee_workspace.employee_workspace`. Every method takes the sealed `Scope` and explicit company; employee methods also take the employment id. An employee id always names `people_workforce_employments.id`.

| Method | Result |
| --- | --- |
| `employees`, `employee` | Directory facts through Workforce, including status, type and supervisor |
| `work_profile`, `put_work_profile` | Optional work location, arrangement and notes |
| `access`, `put_access` | Portal eligibility and reason |
| `change_requests`, `request_change` | Request history and a new pending proposal |
| `review_change` | One review decision, either `approved` or `rejected` |
| `saved_views`, `save_view`, `delete_view` | Only the signed-in account's views in that company |

Requester, reviewer and saved-view owner come from the scope. Callers cannot supply them. Work location is limited to 200 characters, arrangement to 120 and notes or reasons to 2,000. A change request names `full_name`, `short_name`, `email` or `mobile_number`, with a required proposed value of at most 500 characters. Saved-view names are unique per account and company and limited to 120 characters; search is limited to 200 and status to 40.

**Approving a change request records a decision.** It does not edit Core's Person or Affiliation. An operator applies approved identity changes through the owning Core workflow. Portal eligibility also does not create a login or grant a role. Bilimbi allows a requester to review their own request; a second review is refused with `AlreadyReviewed`.

## How writes stay consistent

The facade authorizes inside each `MutationBoundary` transaction. It locks the writable company and, for employee facts, the employment through Workforce before touching its own rows. Missing employees, employees of another company and archived companies refuse the write. Every affected row produces a `Change`, with an empty object on the absent side for creation or deletion. Saved views use `begin_view_state`, while still enforcing permission, company locks and audit capture. Removing a view in the page requires a server-held selection and `ConfirmDialog`.

The private store owns validation and SQL. The migration `people-employee-workspace-001-employee-workspace` creates Bilimbi's four tables: `people_employee_workspace_profiles`, `people_employee_accesses`, `people_employee_change_requests` and `people_employee_saved_views`.

## Running the proof

Mount this repository at `apps/domains/people` in Bilibi, discover modules, and run `cargo run -p bilibi-people-walkthrough` against a disposable database. Its PE1 block exercises profile and eligibility writes, single review, saved-view ownership, revoked grants and company refusals. Then run the application and use `/people/employees`.

## Where to go next

See [Workforce](../workforce/README.md) for employment creation and directory ownership, and [Reference Data](../reference_data/README.md) for the company's employment-type choices.
