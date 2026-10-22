# Reference data

Module ID: `people/reference_data`, crate `bilibi-people-reference-data`. A
company's People reference entries, their aliases and its calendar
exceptions. The Bilimbi source it ports is
[`reference_data/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/reference_data/docs/README.md).
Other People modules read through the `ReferenceData` facade; none reads its
tables. It depends on `people/workforce` (its `Authorization` helpers and
error types) and `people/settings` (the menu group).

New installations have no entries, aliases or exceptions. Kinds, codes,
labels and dates are a company's data: nothing here names a kind. The
`employment_type` kind is the one other modules read: Workforce's
employment form lists a company's types from `list(company, "employment_type")`.

## Public API

Resolve `ReferenceData` from the registry as
`people/reference_data.reference_data`
(`bilibi_people_reference_data::SERVICE`) and call it with the sealed `Scope`
you were handed and an explicit platform company id. Errors are Workforce's
`WorkforceError`: `NotFound` for a company the scope cannot see, `Unauthorized`,
`CompanyArchived`, and `Invalid(Vec<FieldError>)` for refused values, worded as
Bilimbi's changesets word them (`can't be blank`, `should be at most 80
character(s)`, `has already been taken`).

| Function | Capability | Does |
| --- | --- | --- |
| `list(company, kind)` | none | The company's entries of one kind by label, active or not |
| `list_entries(company)` | none | Every entry, by kind and label |
| `list_aliases(company)` | none | Every alias, by entry and label |
| `list_calendar_exceptions(company)` | none | Every calendar exception in date order, for Leave's counted dates |
| `create_entry(company, NewEntry)` | `people.references.manage` | Adds an entry: kind (80 characters), code (100) unique per company and kind, label (200) |
| `add_alias(company, entry_id, label)` | `people.references.manage` | Adds an alias (200); it takes the entry's kind and is unique per kind within the company |
| `create_calendar_exception(company, NewCalendarException)` | `people.references.manage` | Adds a date with a label (200); a date and label are unique within the company |

The reads are company configuration and need no capability, so Leave can read
the calendar exceptions without a grant; they cover any company of the tenant,
archived or not. Every write authorizes `people.references.manage` for the
actor and the company when it runs (Workforce's `authorization::authorize`),
so a revoked grant refuses the very next write, takes the company's writable
lock inside its mutation transaction (an archived company is
`CompanyArchived`) and records the row as a created `Change`. Values are
trimmed before they are checked. No role holds `people.references.manage`, as
in Bilimbi: an administrator grants it.

## Schema

`people-reference-data-001-references` creates Bilimbi's three tables with
their names, columns and unique indexes: `people_reference_data_entries`
(`kind`, `code`, `label`, `active`; unique per company, kind and code),
`people_reference_data_aliases` (unique per company, kind and label; the
foreign key to the entry restricts a delete and, with the entry's company and
kind, keeps an alias under its own entry's kind) and
`people_reference_data_calendar_overrides` (`on_date`; unique per company,
date and label). Each has a foreign key to the company. A `date` is bound and
read as text.

## Pages

| Route | Capability | Shows |
| --- | --- | --- |
| `/people/references` | `people.references.manage` | The authorized company chooser (`Company`, `Open references`) and, with no company chosen, its empty state; no company at all is a second empty state |
| `/people/companies/{company_id}/references` | `people.references.manage` | The company's reference entries, aliases and calendar exceptions, each a FlexTable, with the New reference, New alias and New calendar exception forms |

Both routes are one live page. The menu leaf is Settings › People references
(`people.settings.references`, icon `clipboard-document-list`, order 10).
Bilimbi's lists carry no sort, filter or paging, so the FlexTables keep the
column settings and row heights and nothing else. The page re-reads the
actor's capability and company reach before every event, so a company the
actor lost is withdrawn with its controls and records, and refuses a write
event before it reaches the facade. Flash texts are Bilimbi's: `Reference
added.`, `Alias added.`, `Calendar exception added.`, `You cannot change this
company's references.`, `You no longer have permission to change this
company's references.`, `This company is unavailable to you.`, `Check the
reference fields and unique code.`, `Choose a reference and a unique alias.`
and `Check the date and label.`. An archived company refuses a write with
`This company is archived; nothing in it can change.`, which Bilimbi has no
counterpart for.

The page writes through the facade, not an operation contract; its
`PageWrites` declaration says so with a `Pending::Every` allowance.

## Walkthrough

`fixtures/walkthrough` grants the manager the capability, enters a company's
own `employment_type` entries, aliases and calendar exceptions through the
facade, and prints the refusals: a viewer without the grant, a taken code, a
blank code and an overlong kind, a taken and a foreign alias, a repeated
calendar exception, another tenant's company, a revoked grant and an archived
company, then PostgreSQL refusing a duplicate key, deleting an entry that has
an alias, and an alias under another kind or company.
