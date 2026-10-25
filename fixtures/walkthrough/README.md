# People walkthrough

These executables create their own records and exercise the running modules on a disposable PostgreSQL 18 database. Never run them on an installation that holds real accounts or records.

## Add a proof

**Keep each slice's proof and its registration in its own `src/<slice>.rs` file.** For example, `src/skills_catalog.rs` owns the catalog proof and declares when it runs:

```rust
walkthrough_step!("skills_catalog", after = ["entry_operation"], |ctx| async move {
    let client = ctx.db.get().await?;
    skills_catalog(
        &ctx.services,
        &client,
        &ctx.admin,
        &ctx.manager,
        &ctx.viewer,
        ctx.manager_employment.id,
    )
    .await?;
    Ok(())
});
```

`build.rs` discovers every top-level `src/*.rs` proof file, including new files. Each file registers its default steps with `walkthrough_step!` and any standalone commands with `walkthrough_command!`. Helper files need no registration. Generated module declarations stay in Cargo's build directory. **Do not add a `mod` declaration or a proof call to `lib.rs`, or a command branch to `main.rs`.** The existing manifest already includes every People module.

The default run creates the shared Workforce records first. A step receives the database pool, installed services, signed-in accounts and those employments through `ctx`. Declare predecessor IDs in `after`; use `[]` when only the shared setup is needed. Independent steps run in alphabetical ID order. Duplicate IDs, missing predecessors and cycles refuse the run before database setup. The migrated steps retain their previous execution order because some proofs deliberately change account links and grants for later checks.

A standalone command registers beside its implementation:

```rust
walkthrough_command!("my-new-proof", |url, argument| async move {
    run(url, argument.unwrap_or("prepare")).await
});
```

Run it through the default binary; no new binary wrapper or manifest entry is needed:

```sh
cargo run -p bilibi-people-walkthrough -- my-new-proof prepare
cargo run -p bilibi-people-walkthrough -- --list
```

`--list` validates and prints the default order and available commands without reading `DATABASE_URL`. Standalone commands run in separate processes because Platform identity bindings install once per process. Existing `--bin` commands and their argument defaults still work, including the child-process setup used by Progression. For a new command, put all its setup in its own handler.

Format discovered files explicitly, since `cargo fmt` does not follow generated module declarations:

```sh
rustfmt --edition 2021 apps/domains/people/fixtures/walkthrough/build.rs apps/domains/people/fixtures/walkthrough/src/*.rs apps/domains/people/fixtures/walkthrough/src/bin/*.rs
```

Then run the walkthrough against a fresh disposable database and run `scripts/domain-ci.sh` from the Platform root. Use `--check` with the same `rustfmt` command to verify formatting; the repository CI runs that check too.

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

## Browser accounts

Run `cargo run -p bilibi-people-walkthrough --bin browser-accounts` after installing the default People walkthrough on a disposable database. It gives `manager@example.test` every People permission and `viewer@example.test` the permissions to read records, with People writes and self-service explicitly denied. The reader can reach all companies in the fixture's tenant. Both accounts keep their existing records and employment links.

Sign in with the Platform walkthrough password. Check each People page as the manager, then as the viewer. Pages that require a write permission should refuse the viewer; record pages should hide their write controls. Use each account in light and dark themes at desktop and phone widths. Screenshots and interaction logs belong outside this repository.

To try Add employment, run `cargo run -p bilibi-people-walkthrough --bin browser-accounts -- candidate`. It also creates a human Person named Browser employee and an affiliation with Company 001 through Core's services. Select that person in the dialog and finish the employment there. Each run with `candidate` creates a new person.
