# People settings

Module ID: `people/settings`, crate `bilibi-people-settings`. The People
navigation outline: the `people` menu root and the seven groups every other
People module hangs its leaves on. It owns no table, page, setting or
service. This README is the owner contract; the Bilimbi source it ports is
[`settings/docs/README.md`](https://github.com/BelimbingApp/b-dom-people/blob/main/settings/docs/README.md).

## Menu

Bilimbi's `Bilimbi.People.Settings.Contributions`, verbatim (`src/lib.rs`):

| ID | Label | Icon | Order | Parent |
| --- | --- | --- | --- | --- |
| `people` | People | `users` | 50 | — |
| `people.my_work` | My work | `users` | 10 | `people` |
| `people.team` | Team | | 20 | `people` |
| `people.time_and_expenses` | Time and expenses | | 30 | `people` |
| `people.development` | Development | | 40 | `people` |
| `people.payroll` | Payroll | | 50 | `people` |
| `people.reports` | Reports | | 80 | `people` |
| `people.settings` | Settings | `cog-6-tooth` | 90 | `people` |

Every item is a `MenuItem::container`: no route and no capability. A People
module places a leaf with `MenuItem::page(..).under("people.team")` (or any
other group) and the capability it requires; the group, and the root above
it, appear only while the signed-in account may see at least one leaf, because
Base Menu hides a container without a visible child
(`apps/base/menu/src/lib.rs`). With this module alone mounted the sidebar
therefore shows no People entry at all, which is the expected state of the
scaffold.

Menu icons are bare Heroicon names (`users`, `cog-6-tooth`), rendered as inline SVG by the Platform (see [Icons](https://github.com/BelimbingApp/bilibi/blob/main/apps/base/ui/README.md#icons)). An unknown name shows a missing-icon glyph and logs a warning.

## Dependencies

`base/menu` and `base/module_registry` only. Every other People module
depends on this one so the groups it hangs leaves on are always present.
