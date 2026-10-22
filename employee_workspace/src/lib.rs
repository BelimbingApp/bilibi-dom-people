//! People EmployeeWorkspace, mirroring Bilimbi's EmployeeWorkspace facade.
mod contributions;

pub const MODULE: &str = "people/employee_workspace";
pub const SERVICE: &str = "people/employee_workspace.employee_workspace";
pub const VIEW: &str = "people.employees.view";
pub const MANAGE: &str = "people.employees.manage";
pub const REVIEW: &str = "people.employees.review";
pub const CAPABILITIES: [&str; 3] = [VIEW, MANAGE, REVIEW];
