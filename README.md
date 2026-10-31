# bilibi-dom-people

People Domain for [Bilibi](https://github.com/BelimbingApp/bilibi): a
company's workforce, from employment records and attendance to leave,
claims, skills, training, performance, progression and payroll, ported with
UI parity from Bilimbi's
[b-dom-people](https://github.com/BelimbingApp/b-dom-people). It is one
optional, independently versioned repository that an adopter mounts into a
Bilibi checkout; it does not build on its own.

| Module | Contract |
| --- | --- |
| [`settings/`](settings/README.md) | People navigation outline |
| [`workforce/`](workforce/README.md) | Employment records and workforce boundary |
| [`reference_data/`](reference_data/README.md) | Company People reference data |
| [`employee_workspace/`](employee_workspace/README.md) | Employee directory and workbench |
| [`organisation/`](organisation/README.md) | Positions and employee assignments |
| [`attendance/`](attendance/README.md) | Time, attendance and rosters |
| [`leave/`](leave/README.md) | Leave policies, balances and requests |
| [`claims/`](claims/README.md) | Employee claims and reimbursements |
| [`skills/`](skills/README.md) | Skills catalog and assessments |
| [`training/`](training/README.md) | Courses, sessions and learning |
| [`performance/`](performance/README.md) | Performance targets and reviews |
| [`progression/`](progression/README.md) | Progression policies and standing |
| [`payroll/`](payroll/README.md) | Pay definitions and payroll runs |

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
Company 002 through the manager's explicitly assigned desk. It prints the refusals
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
`bilibi-dev`): the People menu contains the mounted modules' pages. The
Workforce setting appears under Administration › System › Settings for an
account holding `people.workforce.settings.manage`.

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
