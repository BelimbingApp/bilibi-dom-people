# People walkthrough

These executables create their own records and exercise the running modules on a disposable PostgreSQL 18 database. Never run them on an installation that holds real accounts or records.

## Sweep B

Mount People in a Bilibi checkout. To check delivery of payroll and claim approvals, also mount Finance at a revision containing its People event subscriber. The proof uses Base Events receipts and has no Finance crate dependency or SQL reference to Finance tables.

Set `DATABASE_URL` for an empty database, `BILIBI_ENV=development`, and private absolute output directories in `BILIBI_TRAINING_PROOF_ROOT` and `BILIBI_PAYROLL_PROOF_OUT`. Configure the usual Base Settings encryption key when your local setup requires one. From the Platform root, run:

```sh
cargo run -p bilibi-people-walkthrough
cargo run -p bilibi-people-walkthrough --bin attendance
cargo run -p bilibi-people-walkthrough --bin payroll
cargo run -p bilibi-people-walkthrough --bin payroll_calculation
cargo run -p bilibi-people-walkthrough --bin payroll_documents
cargo run -p bilibi-people-walkthrough --bin training-passports
cargo run -p bilibi-people-walkthrough --bin sweep-b
```

The first commands exercise the owners' workflow and database refusals, including assessor review and finalization, locked payroll runs, exact calculations, immutable results, document generation and private evidence. `sweep-b` mounts the My work, Development, Training and Payroll page services against those records. It queues carry-forward and Skills reminders through their confirmation events, runs the real Queue executor, and checks that both jobs retain the enqueuing operator and complete. It also checks the claim CSV's private response headers and refuses another download after its grant is revoked.

With Finance mounted, the same executor delivers the three real version 1 approval and reimbursement events. The proof requires a committed delivery receipt for every event type. A receipt commits with the subscriber's projection. Inspect that projection separately through Finance's own pages or walkthrough. Without Finance the proof prints the omitted delivery check explicitly. It never fabricates events or writes projection rows.

Start `bilibi-web run` against the resulting database for browser checks. Sign in with the Platform walkthrough accounts and password documented in the Platform's fixture. Screenshots, downloaded bytes and logs belong outside the repository. The walkthrough does not replace the mounted `scripts/domain-ci.sh` gate or browser interaction checks.

Individual page proofs that install their own Platform fixture need a separate empty database; see each binary's source comment. In particular, `payroll-runs`, `payroll_pages`, `performance-pages`, `training-effectiveness-pages`, and `leave-pages` must not be chained onto this populated database.
