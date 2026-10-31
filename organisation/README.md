# Organisation

Module ID: `people/organisation`, crate `bilibi-people-organisation`. A
company's positions, their immutable effective-dated title versions and the
effective-dated assignments of employees to them. This README is the owner
contract; the Bilimbi source it ports is
[`organisation/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/organisation/docs/README.md).

Open `/people/organisation` from **People → Team → Organisation** to see a company's positions and holders on a chosen date. Choose an active company and an **As of** date; the table shows title, parent, holders and vacancy. Pages contain 50 positions by default, with 25, 50 and 100 available. Table settings let you choose columns and compact rows.

With `people.organisation.manage`, use **Assign** to choose an employee, assignment kind and dates. Use **End** beside a holder to enter their last day. Both actions ask for confirmation before changing anything. The page retains the chosen company, record and dates on the server and checks both view and manage permission again on each write event.

A position is a stable identity of one company; a vacancy leaves it and its
title in place. Substantive assignments cannot overlap for one position;
acting and concurrent cover can. Both interval ends are inclusive, and
PostgreSQL exclusion constraints enforce the non-overlap rules even for
concurrent writers. Parent links are company-bound and cycle-checked under
the company lock.

Every write checks `people.organisation.manage` through the Workforce authorization helpers, using the signed-in account and the target company. Inside its mutation transaction, it locks the company and checks the grant again. A revoked grant or a company outside the account's reach returns `Unauthorized`; an archived company returns `CompanyArchived`. `assign` checks working membership after taking the lock, so a change while it waited takes effect. Reads need no capability, matching Bilimbi: they expose only a live company of the scope's tenant, with position codes, titles and holder references.

## Public API

Resolve `Organisation` from the registry as `people/organisation.organisation`
(`bilibi_people_organisation::SERVICE`) and call it with the sealed `Scope`
you were handed and an explicit platform company id. Results are typed
records; the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `create_position(scope, company, NewPosition)` | `manage` | A stable position identity: a code unique in the company (trimmed, 1 to 255 characters) under an optional parent position of the same company. Returns the `Position` |
| `set_parent(scope, company, position, Option<parent>)` | `manage` | Moves a position below another position of the company, or to the root. Under the company lock a parent that is the position or one of its descendants is `Cycle`; a position or parent not of the company is `NotFound` |
| `record_version(scope, company, position, NewVersion)` | `manage` | Appends a title version; existing versions never change. A positive number unique for the position, a title of 1 to 255 characters, an inclusive interval no existing version overlaps |
| `assign(scope, company, position, NewAssignment)` | `manage` | Appends a `substantive`, `acting` or `concurrent` assignment of a working employee of the company (an employee the Workforce `employee` read does not expose is `NotFound`). A new substantive assignment first ends every earlier substantive holder who is no longer working, on the day before it starts, recording `people.organisation.assignment_released` for each; a still-working substantive holder over the interval refuses it |
| `end_assignment(scope, company, assignment, effective_to)` | `manage` | Ends one assignment of the company on an inclusive day, only ever earlier, recording `people.organisation.assignment_ended` as the actor's action |
| `positions(scope, company, as_of, page, page_size)` | none | One offset page of the company's positions as of a day, vacancies included, by id: `page` from 1, `page_size` from 1 to 100 (`InvalidOptions` otherwise), as `PositionProjection`s |
| `scan_positions(scope, company, as_of, page_size, cursor)` | none | One step of the high-water keyset scan (below), as a `PositionScan` |
| `count_positions(scope, company)` | none | How many positions the company has, for an explorer's pagination |

## Agent operations

The three actions that have no page in Bilimbi are available as reviewed
operations. Each requires `people.organisation.manage`; the person reviews
the plan and approves the exact inputs before Bilibi commits it. The handler
calls the Organisation facade, which rechecks the live permission and company
lock in the commit transaction.

| Key | Required input | Optional input | Result |
| --- | --- | --- | --- |
| `people.organisation.position.create` | `company_id`, `code` | `parent_id` | The new position |
| `people.organisation.position.version` | `company_id`, `position_id`, `version`, `title`, `effective_from` | `effective_to` | The appended title version |
| `people.organisation.position.parent` | `company_id`, `position_id` | `parent_id`; omit it to place the position at the company root | The moved position |

The full schemas and instructions for choosing values are in
[`position-guide.md`](position-guide.md).

A `PositionProjection` is Bilimbi's `Workforce.Position`: `reference`
(`people/native`, type `position`), `company_reference`, both company axes,
`code`, `parent_reference` (absent at the root), the `title` and `version`
in force on the day (absent when no version covers it), `assignments` (the
working holders on the day: an `assignment` reference, the `employee`
reference and the kind), `assignments_incomplete` and `vacant` (no working
substantive holder on the day). Reads apply the working rule at read time:
an assignee who later leaves the company's working statuses is omitted from
the holders and no longer occupies the position, while the assignment row
stays as history. A page checks only its own holders through Workforce's
`working_employee_ids_on`, never the whole workforce; a Workforce read that is not
current is `NotCurrent`. A page reads at most 500 effective assignments;
past that bound the later positions of the page report
`assignments_incomplete`.

Refusals are typed `OrganisationError` variants; refused values come back as
`Invalid(Vec<FieldError>)` in Bilimbi's changeset words: "can't be blank",
"should be at most 255 character(s)", "has already been taken" (a code in
the company, a version number for the position), "must be greater than 0"
(a version number), "is invalid" (an assignment kind), "must be on or after
the start" (an end before its start), "must be before the current end" (an
end that is not earlier), and "violates an exclusion constraint" on
`effective_from` (an overlapping version, or a substantive assignment over a
working holder), decided on the transaction before the write exactly as the
constraints would decide them.

### The keyset scan and its high-water mark

`scan_positions` with `cursor: None` starts a scan and fixes the high-water
mark: the last position id the id sequence issued (zero before any position
exists), returned as `high_water_id` on every step. It is monotonic, so
deleting the highest position never lowers it, and every step reads only ids
at or below it, so positions created during the scan are never mixed in and
no position is skipped or duplicated. Pass each `next_cursor` back with the
same scope, company and day until it is `None`; the page size (1 to 100) may
differ between steps. A cursor is an opaque URL-safe string bound to the
tenant, company and day; a malformed one, or one minted for another tenant,
company or day, is `InvalidCursor`. A cursor grants nothing: every step
reproves the live company first. A reconciling consumer (the People
connector) deactivates only the native position ids **at or below
`high_water_id`** it did not see after completing every step, and leaves
ids above it alone.

## The `people/organisation.positions` port

Bilimbi's Organisation application registered its read with
`Workforce.register_position_reader/1` at start, and `Workforce.positions/4`
answered `{:error, :unavailable}` without it. Bilibi keeps the direction and
drops the runtime registration: Organisation provides the optional port
`people/organisation.positions` (`base/ports`), Workforce's `positions` scan
(PW1) and, through it, the People connector's native adapter call it on
their own client and sealed scope, and a caller gets `PortError::Unavailable`
when Organisation is not mounted. No edge runs from Workforce to
Organisation. The provider runs the facade's own read: it proves the live
company and filters holders through Workforce, needs no capability, and runs all reads on the caller's client. Workforce's `company` and `working_employee_ids_on` keep both company and holder facts in that same view, without renting nested pool connections. This documented shape is the whole contract.

- Request: `{"company_id": <integer>, "as_of": "<YYYY-MM-DD>"}` with an
  optional `"page_size"` (1 to 100, default 50) and **exactly one** of
  `"page"` (an integer from 1: one offset page) or `"cursor"` (a scan step:
  `null` starts the scan, a string resumes it). Any other key, both or
  neither of `page` and `cursor`, or a malformed value is `Invalid`.
- Answer: `{"positions": [...], "next_cursor": <string or null>,
  "high_water_id": <integer or null>}`. For an offset page both bounds are
  `null`; for a scan step `high_water_id` is the mark and `next_cursor` is
  `null` once the scan is complete. Each position is the projection above:
  `{"reference": {"source_id": "people/native", "type": "position",
  "stable_id": "<id>"}, "company_reference": {.. "type": "company" ..},
  "platform_company_id", "workforce_company_id", "code", "parent_reference":
  {.. "type": "position" ..} or null, "title": <string or null>, "version":
  <integer or null>, "assignments": [{"reference": {.. "type": "assignment"
  ..}, "employee_reference": {.. "type": "employee" ..}, "kind":
  "substantive" | "acting" | "concurrent"}], "assignments_incomplete":
  <bool>, "vacant": <bool>}`.
- Refusals: a company the scope cannot see is `NotFound`; a page, page size
  or cursor the facade refuses is `Invalid`; a Workforce read that is not
  current is `Invalid`.

## Audit

Two actions are recorded through `bilibi_audit::record_action` inside the
write's own mutation transaction, attributed to the scope's actor (and the
operator behind an impersonation), retained, with the payload
`{"company_id", "assignment_id", "position_id", "employee_id",
"previous_effective_to", "effective_to"}`:

| Event | When |
| --- | --- |
| `people.organisation.assignment_released` | `assign` ended an earlier substantive holder who is no longer working, on the day before the new assignment starts |
| `people.organisation.assignment_ended` | `end_assignment` ended an assignment |

Every row the writes touch is also a `Change` of the mutation
(`people_position`, `people_position_revision`, `people_position_placement`,
an empty object on the absent side of a creation).

## Capabilities

| Capability | Tenant Owner |
| --- | --- |
| `people.organisation.view` | |
| `people.organisation.manage` | |

Bilimbi grants neither to a system role; an administrator grants them. Both
live under the Authz domain `people`; `view` and `manage` are verbs Base
declares. The explorer opens under `view`, offers its assign and end
controls to an actor holding `manage`, and refuses the event, with Bilimbi's
sentence "You no longer have permission to change this company's
assignments.", when the grant is gone; the facade checks once more.

## Database

`migrations/001-positions.sql`, applied as `people-organisation-001-positions`,
the end state of Bilimbi's `20260930060000_create_people_positions` with the
assignment's employee keyed on People's employment record. It creates the
`btree_gist` extension, which the exclusion constraints need.

| Table | Columns | Constraints |
| --- | --- | --- |
| `people_positions` | `id`, `company_id` → `companies`, `code` (255), `parent_id`, timestamps | `(company_id, code)` unique; `(id, company_id)` unique; not its own parent; `(parent_id, company_id)` → `people_positions (id, company_id)` so a parent is of the same company |
| `people_position_revisions` | `id`, `position_id`, `version` (integer), `title` (255), `effective_from`, `effective_to`, `inserted_at` | `(position_id, version)` unique; `version > 0`; end on or after start; `EXCLUDE USING gist (position_id WITH =, daterange(effective_from, effective_to, '[]') WITH &&)` |
| `people_position_placements` | `id`, `position_id`, `employee_id` → `people_workforce_employments`, `kind`, `effective_from`, `effective_to`, `inserted_at` | kind in the three; end on or after start; the same exclusion `WHERE kind = 'substantive'` |

Indexes: `(company_id, parent_id)` and `(company_id, id)` on positions,
`(position_id, effective_from)` on revisions and placements,
`(employee_id, effective_from)` on placements. Dates are bound and read as
ISO text (`$n::text::date`, `to_char`): the workspace's `tokio-postgres` has
no date feature, and a Domain must not turn one on.

## Walkthrough

`fixtures/walkthrough` runs the Organisation block after the Workforce block
(`fixtures/walkthrough/src/organisation.rs`): the admin grants the manager
both capabilities, the manager builds CEO › COO › OPS-1 and a GM at Company
002, and the walkthrough prints every refusal beside the write (the viewer
without the grant, a duplicate or blank code, a parent of another company,
a cycle, a repeated or overlapping version, an employee of another company,
an unknown kind, a substantive assignment over a working holder, an end that
is not earlier), releases Sam Staff's OPS-1 seat once probation stops
counting as working, lists the two audit actions, reads the explorer's pages
as of four days, walks the keyset scan two positions at a time and refuses a
cursor for another day, company or a tampered one, calls the port as
Workforce will, and ends with PostgreSQL refusing an overlapping substantive
holder, an overlapping version, an own or cross-company parent, an unknown
kind, an end before the start and an unknown employment. It also queues a position write behind a company lock, revokes the grant through Authz, then proves that the queued write refuses without creating a position.

The walkthrough also reserves every pool connection except one and proves that a position page can still read its company and holders.

## Dependencies

`base/agent_api`, `base/audit`, `base/authz`, `base/database`, `base/menu`, `base/module_registry`,
`base/ports`, `base/tenancy`, `base/ui`, `core/company`, `people/settings`, `people/workforce`; the Cargo
manifest lists the same edges. Workforce reaches this module only through
the port above, never the other way round.

## Explorer browser proof

The walkthrough adds 51 positions for pagination and a title that changes on 1 July 2026. Sign in as `manager@example.test`, open `/people/organisation`, and select 25 rows per page. Follow Next and Back, choose a company with no positions, and compare **As of** dates before and after the title change. Use **Assign**, cancel its confirmation, then confirm an acting assignment. End that assignment on a valid last day and confirm the change. `viewer@example.test` has view permission only and sees neither action.

The company picker uses Workforce's `authorization::company_choices` for active companies and the already-selected pending or suspended company while still authorized; archived companies are never offered. Core supplies both names and lifecycle status through Workforce; the page never queries Core's company table.
