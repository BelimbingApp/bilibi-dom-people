# bilibi-dom-people

People Domain for [Bilibi](https://github.com/BelimbingApp/bilibi): a
company's workforce, from employment records and attendance to leave,
claims, skills, training, performance, progression and payroll, ported with
UI parity from Bilimbi's
[b-dom-people](https://github.com/BelimbingApp/b-dom-people). It is one
optional, independently versioned repository that an adopter mounts into a
Bilibi checkout; it does not build on its own.

| Module | Descriptor ID | Crate | Owns |
| --- | --- | --- | --- |
| [`settings/`](settings/README.md) | `people/settings` | `bilibi-people-settings` | the People navigation outline: the `people` menu root and the seven groups (`my_work`, `team`, `time_and_expenses`, `development`, `payroll`, `reports`, `settings`) every other module hangs its leaves on |
| [`workforce/`](workforce/README.md) | `people/workforce` | `bilibi-people-workforce` | the read seam every People module builds on (a company's native identity, its working employees, one employee, employees by id, direct reports, each a `ReadResult`, named through Core Person and Affiliation), the employment record (`people_workforce_employments`: person, affiliation, number, type, the five statuses, dates, supervisor, department), the company setting `people.workforce.working_statuses`, and the `Authorization` helpers (`authorize`, `authorize_company`, `selectable_companies`, `allowed`) over Core Company and Base Authz |

The other eleven People modules of Bilimbi (`reference_data`,
`employee_workspace`, `organisation`, `attendance`, `leave`, `claims`,
`skills`, `training`, `performance`, `progression`, `payroll`) land beside
these as sibling module folders, in later slices of the port, each
depending on `people/settings` and `people/workforce`. Bilibi has no Core
Employee: People Workforce owns the employment record, which references the
Core Person and that person's affiliation with the employer company, and
the Employees pages gain the employment form Bilimbi kept in Core.

With these two modules mounted the sidebar shows no People entry yet: the
root and its groups are containers, and Base Menu hides a container without
a visible child until a module contributes a leaf.

## Mount

Clone this repository into a Bilibi checkout at `apps/domains/people` (a
real nested checkout, never a symlink) and work from the Bilibi root:

```sh
git clone https://github.com/BelimbingApp/bilibi-dom-people.git apps/domains/people
python3 scripts/discover-modules.py
cargo run -p bilibi-web -- migrate
```

Bilibi discovers the mounted container from `bilibi.container.toml`; no list
names it. Removing the folder removes People's code from the next build and
keeps its data: the migration ledger keeps the applied versions, and a
remount replays nothing. The mount rules are in Bilibi's
`apps/domains/AGENTS.md`. Commit and push only inside this repository; the
Platform's `apps/domains/Cargo.toml`, `apps/domains/src/lib.rs` and
`Cargo.lock` change while mounted and are never committed there.

## Walkthrough

Proofs never use a seeded database. `fixtures/walkthrough` is an
example-only crate: against a freshly created disposable database it
migrates the composed app, installs the Platform walkthrough fixture (its
tenants, accounts, Persons, affiliations and companies), gives
`manager@example.test` the Tenant Owner role, and through the Workforce
facade employs the fixture's affiliated accounts: the manager at Company
001, a new human Person affiliated with Company 001 and linked to
`staff@example.test` who reports to the manager, and the contractor at
Company 002 through the manager's tenant-wide reach. It prints the refusals
beside the writes (a viewer without the grant, an affiliation of another
company, an affiliation already employed, an agent Person, a taken number,
an unknown status, an end before the start), reads the company, its working
employees, one employee, employees by id and the manager's direct reports,
narrows the working statuses to `active` (Sam Staff, on probation, drops
out) and restores the default, asks the `Authorization` helpers what each
account may reach, and ends with PostgreSQL refusing an unknown status, a
self-supervisor, a move to another company, an end before the start, a
second employment of one affiliation and a supervisor from another company.

```sh
createdb -h 127.0.0.1 -p 5438 -U bilibi bilibi_people_walkthrough
export DATABASE_URL=postgres://bilibi:bilibi@127.0.0.1:5438/bilibi_people_walkthrough
cargo run -p bilibi-people-walkthrough
APP_ORIGIN=http://localhost:3000 cargo run -p bilibi-web -- run
```

Sign in at <http://localhost:3000> as `manager@example.test` (password
`bilibi-dev`): the sidebar shows no People entry, because the People root
has groups but no leaf yet. The Workforce setting appears under
Administration › System › Settings for an account holding
`people.workforce.settings.manage`.

## Checks

There are no unit tests. The gate is the Platform's `scripts/domain-ci.sh`,
run from the Bilibi root with this repository mounted and `DATABASE_URL`
naming an empty PostgreSQL 18 database: discovery, the committed CSS, the
mounted `Cargo.lock` pin, `cargo fmt`, `cargo clippy -D warnings`, the
release build (`DOMAIN_CI_BUILD=check` boots the debug build instead),
`migrate`, `seed-production`, and a boot that answers `/`.

## CI

`.github/workflows/ci.yml` is Bilibi's reference Domain workflow
(`docs/domain-ci/ci.yml`) with the container ID set. It checks out Bilibi at
the revision in [`.github/bilibi-revision`](.github/bilibi-revision), moves
this repository to `apps/domains/people`, runs `scripts/domain-ci.sh` and
verifies the gates left this repository unchanged. Bump that revision to
build against a newer Platform.

## Naming

Optional Bilibi repositories are named `bilibi-<role>-<id>`: `dom-` for a
Domain. The mount folder is the container ID, so `bilibi-dom-people` mounts
at `apps/domains/people`. Module crates are `bilibi-people-<module>`
(underscores become hyphens), capability keys `people.<module>.*` under the
Authz domain `people`, tables `people_<module>_*` and migrations
`people-<module>-NNN-<slug>`, all as Bilimbi names them; module IDs
(`people/workforce`) are Bilimbi's, kept because IDs are persisted data.

## License

[MIT](LICENSE).
