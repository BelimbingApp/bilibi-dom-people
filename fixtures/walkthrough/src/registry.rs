//! Fixture-only collection: each proof owns its registration and ordering edges.
use crate::{Employment, Scope, Services};
use bilibi_database::{Db, Error, Result};
use std::{collections::BTreeMap, future::Future, pin::Pin};

/// Records from the shared Workforce setup. Module-specific proof state stays
/// with its owner; `scopes` holds signed-in accounts kept for later unlink checks.
pub(crate) struct Context {
    pub db: Db,
    pub services: Services,
    pub admin: Scope,
    pub manager: Scope,
    pub viewer: Scope,
    pub contractor: Scope,
    pub manager_employment: Employment,
    pub staff_employment: Employment,
    pub contractor_employment: Employment,
    pub scopes: BTreeMap<&'static str, Scope>,
}

type ProofFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

pub(crate) struct Step {
    pub id: &'static str,
    pub after: &'static [&'static str],
    pub run: for<'a> fn(&'a mut Context) -> ProofFuture<'a>,
}
inventory::collect!(Step);

pub(crate) struct Command {
    pub name: &'static str,
    pub run: for<'a> fn(&'a str, Option<&'a str>) -> ProofFuture<'a>,
}
inventory::collect!(Command);

macro_rules! walkthrough_step {
    ($id:literal, after = [$($after:literal),* $(,)?], |$ctx:ident| $body:expr) => {
        inventory::submit! {
            crate::registry::Step {
                id: $id,
                after: &[$($after),*],
                run: |$ctx| Box::pin($body),
            }
        }
    };
}

macro_rules! walkthrough_command {
    ($name:literal, |$url:ident, $argument:ident| $body:expr) => {
        inventory::submit! {
            crate::registry::Command {
                name: $name,
                run: |$url, $argument| Box::pin($body),
            }
        }
    };
}

fn invalid(message: impl std::fmt::Display) -> Error {
    eprintln!("walkthrough registration: {message}");
    Error::Invalid
}

/// Validate the whole graph before the bootstrap makes any database changes.
/// BTreeMap also gives independent steps a stable alphabetical tie-breaker.
pub(crate) fn steps() -> Result<Vec<&'static Step>> {
    let mut pending = BTreeMap::new();
    for step in inventory::iter::<Step> {
        if step.id.is_empty() || pending.insert(step.id, step).is_some() {
            return Err(invalid(format!("duplicate or empty step ID {:?}", step.id)));
        }
    }
    for step in pending.values() {
        for dependency in step.after {
            if !pending.contains_key(dependency) {
                return Err(invalid(format!(
                    "step {} names missing predecessor {dependency}",
                    step.id
                )));
            }
        }
    }
    let mut ordered: Vec<&Step> = Vec::new();
    while !pending.is_empty() {
        let next = pending.values().find(|step| {
            step.after
                .iter()
                .all(|id| ordered.iter().any(|done| done.id == *id))
        });
        let Some(next) = next else {
            return Err(invalid(format!(
                "step dependency cycle among {:?}",
                pending.keys().collect::<Vec<_>>()
            )));
        };
        let id = next.id;
        ordered.push(pending.remove(id).expect("step was found in pending"));
    }
    Ok(ordered)
}

fn commands() -> Result<BTreeMap<&'static str, &'static Command>> {
    let mut commands = BTreeMap::new();
    for command in inventory::iter::<Command> {
        if command.name.is_empty() || commands.insert(command.name, command).is_some() {
            return Err(invalid(format!(
                "duplicate or empty command name {:?}",
                command.name
            )));
        }
    }
    Ok(commands)
}

pub(crate) async fn command(name: &str, url: &str, argument: Option<&str>) -> Result<()> {
    let commands = commands()?;
    let command = commands
        .get(name)
        .ok_or_else(|| invalid(format!("unknown command {name:?}; use --list")))?;
    (command.run)(url, argument).await
}

pub(crate) fn list() -> Result<()> {
    let steps = steps()?;
    let commands = commands()?;
    println!("Shared Workforce setup, then default steps:");
    for step in steps {
        println!("  {} (after: {})", step.id, step.after.join(", "));
    }
    println!("Standalone commands (each runs in its own process):");
    for name in commands.keys() {
        println!("  {name}");
    }
    Ok(())
}
