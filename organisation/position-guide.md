Use these actions to create a position, add a title, or change where a position sits in its company's reporting tree.

For example, create a position with code `OPS-1`, then add title version 1 starting on `2026-10-01`. If it reports to another position, create it with that position's ID as `parent_id`. You can later move it by calling the parent action; omit `parent_id` to make it a root position.

Before each write, review the plan returned by Bilibi. Commit only after the person approves the exact company, position, title, dates, and parent shown in that plan. A plan code can be used once.

## Create a position

Call `people.organisation.position.create` with `company_id`, `code`, and optional `parent_id`. The code is trimmed and must be unique within the company. A parent must already belong to that company.

## Add a title version

Call `people.organisation.position.version` with `company_id`, `position_id`, `version`, `title`, and `effective_from`. You may also pass `effective_to`, or leave it out for an open-ended version. Use a positive version number that has not been used for this position. Dates are inclusive. The new date range must not overlap another version.

## Move a position

Call `people.organisation.position.parent` with `company_id` and `position_id`. Include `parent_id` to place it below another position. Omit `parent_id` to move it to the company root. The new parent must be in the same company, and the move cannot create a reporting cycle.

## Refusals

The person must have permission to manage positions in the company. Bilibi checks this permission again when the approved change is committed. A company that is archived cannot be changed. Missing records and records from another company are reported as not found.

If Bilibi rejects a field, correct the value it names and make a new plan. Common reasons include a duplicate position code or version number, a title longer than 255 characters, an invalid date, an overlapping title period, or a reporting cycle.
