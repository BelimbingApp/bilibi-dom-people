# People composition sweep

Use the same disposable database while removing and remounting People, the People connector and Finance. The records must survive each change to the installed code.

This proof mirrors Bilimbi's optional-module behaviour. It uses the real People facades, connector adapter and Finance event subscriber. It adds no production records or cross-domain dependencies.

## Create the source records

Mount People in a Platform checkout at the revision in `.github/bilibi-revision`. Set `DATABASE_URL` to an empty PostgreSQL 18 database and configure database authentication in your shell. Use a private directory outside the repositories for logs, screenshots and saved data checks.

Run the source fixture from the Platform root, passing the currency of the disposable Finance book:

```sh
python3 scripts/discover-modules.py
cargo run -p bilibi-people-walkthrough --bin composition-source -- MYR
```

`MYR` is the current Finance walkthrough's book currency, not a product default. The fixture creates an employment, approves `FY1-run` independently with `12.345678 × 1.234567`, approves three claims of `80.00`, and reimburses the last claim. All writes use the owning People facades. Its dates match Finance's 2026 walkthrough fiscal year. Use this command once per disposable database.

## Check each composition

Keep one build directory. Stop only the web process you started before changing the mounted folders. Move clones into your private scratch directory to unmount them; move the same clones back to remount. Run discovery and build the app after each change.

| Composition | Check before and after remount |
| --- | --- |
| People alone | Save all People rows and their migration IDs/checksums. Remove People, build and migrate the Platform, and require the People route to answer 404. Remount, migrate again, and compare the saved rows and migration records. Open Employees and Payroll to read the retained records. |
| People with the connector | Run the connector's `sweep -- mounted`, then `sweep -- absent` with People removed and `sweep -- remount` after restoring it. Also remove and remount the connector, comparing its saved rows and migration records. |
| People with Finance | Run Finance's `bilibi-finance-payroll-source prepare`, then `post`. Remove People and run `absent`; restore it and run `retained`. Also remove and remount Finance, comparing both Domains' saved rows and migration records. |

The connector sweep uses `KV1_PROOF_ROOT`, `BILIBI_SETTINGS_KEY` and `KV1_WEBHOOK_SECRET`; see its `connector/sweep.md`. Keep these values private and unchanged across its commands. It checks native sync, full read, files, backup, restore and recovery. Its absent mode must refuse sync with `PeopleUnavailable` while keeping its stored data.

Finance's source walkthrough lives in its own `fixtures/walkthrough/src/bin/payroll-source.rs`. It reads only Finance's public APIs and Base Events receipts. Its `post` mode proves the exact `15.241566651426` source amount, rounds the posting to the book's minor units, posts the run and a claim once, and checks replay. After removing People, `absent` requires retained postings and refused new source writes. `retained` reads the copied sources after remount.

## Restore a release pin

Commit all source changes before pinning. With the intended Domains mounted, resolve the lockfile, pin the composition, restore the Platform lockfile, then restore the saved pin:

```sh
cargo metadata --format-version 1 >/dev/null
python3 scripts/discover-modules.py --pin
git restore Cargo.lock
python3 scripts/discover-modules.py --restore
cargo build --release --locked -p bilibi-web
./target/release/bilibi-web status
```

Require `status` to show the pinned Platform commit and each mounted Domain commit. Save the manifest and lockfile hash outside the repositories. Do not commit the Platform's generated composition or lockfile.

## Run the gates

Run the Platform's `scripts/domain-ci.sh` on a separate empty database for People alone, the connector alone, People with the connector, and People with Finance. Each run must pass discovery, committed CSS, pinning, format, Clippy, the locked release build, migration, production seeds and `GET /`. Set a private `DOMAIN_CI_PORT` for the boot check.

Open the mounted pages and their absent-source states in Chrome using the same disposable records. Preserve logs and screenshots outside the repositories. Check both repositories' forge CI jobs, including the connector's two topologies.

## Where to go next

- [People walkthroughs](README.md) cover the individual modules and permission cases.
- Platform `apps/domains/AGENTS.md` defines mounting and pinning.
- Platform `docs/versions.md` defines release tags and catalogue entries.
