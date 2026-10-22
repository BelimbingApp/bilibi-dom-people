# Payroll foundation

Payroll keeps company pay definitions and freezes the setup used for one pay period and currency. It mirrors Bilimbi `people/payroll`; no country, currency, statutory pack or sample record is installed.

## Set up a period

Resolve `Payroll` through `people/payroll.payroll`. Give each call the signed-in account's sealed `Scope` and the explicit platform company ID.

1. Call `put_settings` with a country identifier and up to twenty uppercase three-letter currency codes. Both settings start empty.
2. Add a classification with `create_classification`, then an item with `create_item`. The item's dates must fit inside its classification.
3. Add a pay window with `create_period`. Windows cannot overlap; the payment date must be on or after the end.
4. Read `sources` and add Leave or Claims mappings with `create_mapping`. Read `attendance_allowances` and use `create_attendance_allowance_mapping` for allowance rule codes.
5. Call `freeze_setup` (`create_run` is the Bilimbi alias) for the period and currency. Review the returned snapshot's `unmapped_sources`, then call `lock_run`.

**Definitions and mappings are append-only. Locking a run cannot be undone.** Plan bounded dates when a later version will replace a definition: an open-ended version cannot be shortened.

## Versions and exact amounts

Dates are inclusive. Versions of one classification or item code cannot overlap. A mapping must fit within its item's dates. One source or allowance rule code can map once per currency during any date interval; mappings in different currencies can coexist.

Item amounts use PostgreSQL `numeric(20,6)`. `NewItem.amount` is a `rust_decimal::Decimal`; negative amounts, values at or above `100000000000000`, and nonzero digits past six decimal places are refused before SQL. Returned `Item.amount` and frozen amounts are exact six-place decimal strings. Callers must parse decimal text directly; never pass through floating point.

Attendance mappings use rule codes, so subsequent rule versions keep the mapping. At least one active rule version must overlap the mapping's dates, and every overlapping active version must use the item's currency. Future-effective versions are included in the choices. The allowance value does not become an additional payroll amount.

## Frozen setup

Each period and currency has one run. Its country and currency are stored on the run; `snapshot` contains `period`, `classifications`, `items`, `mappings`, `attendance_mappings` and `unmapped_sources`. Version arrays use stable ID order. Only items and mappings in the run currency are frozen. Later settings and versions cannot change that history.

Payroll reads catalogs and request activity through `Leave::list_types`, `Leave::requested_type_ids`, `Claims::claim_types`, `Claims::requested_claim_type_ids` and `Attendance::list_allowance_rules`. It never queries their tables. Unmapped sources include active types plus archived or inactive types with relevant live requests in the period. Active allowance rules effective in the period and run currency are included too. If a later allowance version has another currency, its frozen mapping is omitted and the source reports `reason: "currency mismatch"`.

A Workforce company read must be current before sources can be read or setup frozen. `PayrollError::NotCurrent` retains the freshness cause. The native Workforce currently always returns current data; no remote adapter or simulated stale path is installed.

## Permissions and writes

| Capability | Use |
| --- | --- |
| `people.payroll.view` | Read setup and Leave/Claims choices |
| `people.payroll.manage` | Change settings, add definitions and mappings, freeze and lock |
| `people.payroll.approve` | Reserved for the calculation and decision slice |
| `people.payroll.attendance-mappings.manage` | Read and add Attendance mappings |

Bilimbi's Tenant Owner grants cover view, manage and approve; Attendance mappings require an explicit grant. Every call checks current company reach through Workforce's authorization helpers. Impersonated sessions and archived companies are refused. Every write rechecks inside its `MutationBoundary`, holds the writable Company lock and a private company advisory transaction lock, and captures each affected row. PostgreSQL additionally refuses definition updates/deletes, deleting any run, changing a draft except its initial lock, and every change to a locked run. Composite foreign keys retain tenant/company ownership.

`PayrollError` carries the source page's refusal sentences. `Invalid` holds field errors; pages should show these outcomes without returning fatal transport errors.

## Verification and next slices

`fixtures/walkthrough` holds the disposable-database Payroll proof. Run its default binary first, then `cargo run -p bilibi-people-walkthrough --bin payroll`. The proof creates its own records through the public facades and checks both facade refusals and PostgreSQL guards.

PY1 contributes no page or menu leaf. Setup and Attendance mapping pages follow in PY4. Contribution intake, exact calculation, independent approval and events follow in PY2; private documents follow in PY3, then run pages in PY5. No Platform change is required.
