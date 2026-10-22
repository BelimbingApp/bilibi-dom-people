# Payroll

Payroll freezes company pay definitions, calculates attested contributions, and records an independent approval or rejection. It mirrors Bilimbi `people/payroll`; no country, currency, statutory pack or sample record is installed.

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
| `people.payroll.approve` | Independently approve or reject a calculation with a reason |
| `people.payroll.attendance-mappings.manage` | Read and add Attendance mappings |

Bilimbi's Tenant Owner grants cover view, manage and approve; Attendance mappings require an explicit grant. Every call checks current company reach through Workforce's authorization helpers. Impersonated sessions and archived companies are refused. Every write rechecks inside its `MutationBoundary`, holds the writable Company lock and a private company advisory transaction lock, and captures each affected row. PostgreSQL additionally refuses definition updates/deletes, deleting any run, changing a draft except its initial lock, and every change to a locked run. Composite foreign keys retain tenant/company ownership.

`PayrollError` carries the source page's refusal sentences. `Invalid` holds field errors; pages should show these outcomes without returning fatal transport errors.

## Accept contributions and calculate

Give `intake` a `NewContribution` with an employment ID, frozen item ID, company-unique `source_key`, evidence reference, date, exact units and direction. Parse units from decimal text into `rust_decimal::Decimal`. Units must be greater than zero, below `100000000000000`, and have at most six nonzero fractional places. Directions are `earning`, `deduction` and `employer`. Evidence is a nonblank reference of up to 500 characters; Payroll does not verify the underlying source or infer eligibility.

The employee must currently be working in the company. The date must fall within both the frozen period and item version. Contributions can arrive before or after locking, until calculation. Repeating the same key and facts returns the original row; different facts or another run under that key return `ReplayConflict`. Replays after calculation are refused, matching Bilimbi.

`intake_mapped` selects an item from the frozen Leave or Claims mapping using its source kind, source key and date. The contribution keeps its own replay key. `intake_attendance_allowance` selects a frozen allowance mapping by rule code and always records an earning at the mapped pay-item rate. An empty direction means earning; another direction returns `DirectionConflict`. The input item ID is ignored by both mapped calls. Neither call re-reads live source formulas.

After `lock_run`, call `calculate`. At least one contribution is required. Payroll rechecks distinct employee IDs through Workforce in pages of 1,000, on the same transaction and under the writable company lock. A missing or no-longer-working employee refuses calculation. All result entries and the calculation commit together; contributions and result entries then close permanently.

For example, a frozen rate of `12.345678` multiplied by `1.234567` units produces `15.241566651426`. Every result amount and employee total has exactly twelve places. Employer contributions are reported separately; net is earnings minus deductions. Intake refuses a product that cannot fit `rust_decimal` at twelve places. Calculation and approval also refuse overflowing totals. No intermediate value is rounded or passed through floating point. This intentionally bounds the port below PostgreSQL's `numeric(40,12)` capacity.

`Calculation.snapshot` contains `setup`, source-key-sorted `contributions`, and `result`. The result contains `version: 1`, source-key-sorted `lines`, and employee-ID-sorted `totals`. Each line retains the accepted contribution and adds `item_code`, `rate` and `amount`; totals contain `employee_id`, `earning`, `deduction`, `employer` and `net`. The digest is lowercase SHA-256 over compact JSON with recursively sorted object keys and preserved array order. Later live settings never enter the calculation. A calculation retry returns the same stored ID, author, snapshot and digest, without another audit mutation.

`run_output` returns the run, contributions, calculation, decision, persisted `entries` and document records from one consistent read snapshot. Exact numeric fields are strings.

## Review a calculation

Call `decide_run` with `approved` or `rejected` and a nonblank reason of up to 500 characters. `approve_run` and `reject_run` are convenience calls. The reviewer must have `people.payroll.approve` and must be someone who neither created nor locked the run, calculated it, nor authored any contribution. Both outcomes are final. The database enforces reviewer independence and refuses all output updates and deletes, contributions or entries after calculation, calculation before locking, and documents before approval.

## Approval event: version 1

Payroll declares `people/payroll.run_approved` with `EventType { version: 1 }`. Approval stores this event in the same transaction as its decision. Rejection emits no event. Other Domains read this contract through Base Events; they never read People's tables or call Payroll directly.

The subject is `run:<run_id>` and the tenant-unique publisher key is `run_approved:<run_id>`. The payload is:

```json
{
  "company_id": 1,
  "run_id": 42,
  "reference": "period-reference",
  "currency": "XAA",
  "pay_on": "2026-10-31",
  "components": [
    {"code": "pay-item-code", "direction": "earning", "amount": "15.241566651426"}
  ]
}
```

IDs are JSON integers. `reference` is the frozen period code, `currency` is the run currency, and `pay_on` is the frozen inclusive ISO payment date. Each component aggregates all frozen result lines for one item code and direction, including zero amounts. Components sort by code then direction; amounts are exact nonnegative decimal strings with twelve places. The event contains no employee names, identities, contribution evidence or approval reason. It represents a final approved calculation. Consumers map `currency` and `pay_on` to their own accounting terms.

Base Events limits payloads to 64 KiB and arrays to 100 entries. Approval returns `ApprovalTooLarge` if the aggregated components exceed either limit; the decision remains absent. `DecimalRange` likewise refuses approval before any decision when a component sum exceeds the exact range. Event or audit failure rolls back the decision with it.

## Verification and next slices

`fixtures/walkthrough` holds the disposable-database Payroll proof. Run its default binary first, then `cargo run -p bilibi-people-walkthrough --bin payroll`. Then run `cargo run -p bilibi-people-walkthrough --bin payroll_calculation`. The proofs create records through public facades and check exact values, canonical digest, concurrent retries, reviewer exclusions, event publication, grant revocation and PostgreSQL guards. Use only a disposable database.

PY1 contributes no page or menu leaf. Setup and Attendance mapping pages follow in PY4. PY2 supplies contribution intake, exact calculation, independent decisions and events; private documents follow in PY3, then run pages in PY5. No Platform change is required.
