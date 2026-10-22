//! People settings (`people/settings`, Bilimbi `Bilimbi.People.Settings`):
//! the People navigation outline. It contributes the `people` menu root and
//! the seven groups every other People module hangs its leaves on, with
//! Bilimbi's IDs, labels, icons and orders verbatim. It owns no table, no
//! page and no service.
//!
//! Base Menu hides a container without a visible child, so the People root
//! and its groups stay out of the sidebar until a module contributes a leaf
//! the signed-in account may see.
use bilibi_menu::MenuItem;
use bilibi_module_registry::{Layer, Module};

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/settings";

/// The People root (`people`), the parent of every group below.
pub const ROOT: &str = "people";
/// The seven groups under the root, in Bilimbi's order: `(id, label, order)`.
pub const GROUPS: [(&str, &str, i32); 7] = [
    ("people.my_work", "My work", 10),
    ("people.team", "Team", 20),
    ("people.time_and_expenses", "Time and expenses", 30),
    ("people.development", "Development", 40),
    ("people.payroll", "Payroll", 50),
    ("people.reports", "Reports", 80),
    ("people.settings", "Settings", 90),
];

inventory::submit! {
    Module {
        id: MODULE,
        layer: Layer::Domains,
        dependencies: &["base/menu", "base/module_registry"],
        migrations: &[],
        services: &[],
    }
}

// Bilimbi `settings/lib/settings/contributions.ex`: the root carries the
// `users` icon at order 50 beside the other Domain roots; `My work` repeats
// it and `Settings` carries `cog-6-tooth`. Contributions carry bare Heroicon
// names; the host adds the set prefix.
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people", "People")
        .icon("users")
        .order(50)
);
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people.my_work", "My work")
        .icon("users")
        .under("people")
        .order(10)
);
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people.team", "Team")
        .under("people")
        .order(20)
);
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people.time_and_expenses", "Time and expenses")
        .under("people")
        .order(30)
);
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people.development", "Development")
        .under("people")
        .order(40)
);
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people.payroll", "Payroll")
        .under("people")
        .order(50)
);
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people.reports", "Reports")
        .under("people")
        .order(80)
);
bilibi_menu::contribute!(
    "people/settings",
    MenuItem::container("people.settings", "Settings")
        .icon("cog-6-tooth")
        .under("people")
        .order(90)
);
