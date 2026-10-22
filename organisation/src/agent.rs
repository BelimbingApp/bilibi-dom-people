//! Reviewed agent operations for Bilimbi Organisation's three facade-only
//! position writes. They call Organisation again for every commit.
use crate::{
    authorization, positions, NewPosition, NewVersion, Organisation, OrganisationError, MANAGE,
    MODULE,
};
use bilibi_agent_api::{
    Channels, Definitions, Error as AgentError, Guide, Handler, Operation, OperationClient,
    OperationFuture, WriteHandler,
};
use bilibi_database::{Error, MutationTransaction};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

static FACADE: OnceLock<Arc<Organisation>> = OnceLock::new();

pub(super) fn bind(facade: Arc<Organisation>) -> bilibi_module_registry::Result<()> {
    FACADE.set(facade).map_err(|_| {
        bilibi_module_registry::RegistryError::Service(
            "duplicate Organisation operation binding".into(),
        )
    })
}

fn facade() -> bilibi_agent_api::Result<&'static Organisation> {
    FACADE
        .get()
        .map(Arc::as_ref)
        .ok_or(AgentError::Backend(Error::Internal))
}

fn refusal(error: OrganisationError) -> AgentError {
    match error {
        OrganisationError::Unauthorized => AgentError::Forbidden,
        OrganisationError::NotFound => AgentError::NotFound,
        OrganisationError::Invalid(errors) => {
            let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for error in errors {
                fields
                    .entry(error.field.to_owned())
                    .or_default()
                    .push(error.message);
            }
            AgentError::InvalidInput(fields)
        }
        OrganisationError::Failed(error) => AgentError::Backend(error),
        other => AgentError::InvalidInput(BTreeMap::from([(
            "position".into(),
            vec![other.to_string()],
        )])),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    company_id: i64,
    code: String,
    parent_id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionInput {
    company_id: i64,
    position_id: i64,
    version: i32,
    title: String,
    effective_from: String,
    effective_to: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentInput {
    company_id: i64,
    position_id: i64,
    parent_id: Option<i64>,
}

fn parse<T: for<'de> Deserialize<'de>>(input: &Value) -> bilibi_agent_api::Result<T> {
    serde_json::from_value(input.clone()).map_err(|_| Error::Invalid.into())
}

fn version_input(input: &Value) -> bilibi_agent_api::Result<(VersionInput, NewVersion)> {
    let input: VersionInput = parse(input)?;
    let effective_from = input.effective_from.parse::<Date>().map_err(|_| {
        AgentError::InvalidInput(BTreeMap::from([(
            "effective_from".into(),
            vec!["is invalid".into()],
        )]))
    })?;
    let effective_to = input
        .effective_to
        .as_deref()
        .map(str::parse::<Date>)
        .transpose()
        .map_err(|_| {
            AgentError::InvalidInput(BTreeMap::from([(
                "effective_to".into(),
                vec!["is invalid".into()],
            )]))
        })?;
    let new = NewVersion {
        version: input.version,
        title: input.title.clone(),
        effective_from,
        effective_to,
    };
    Ok((input, new))
}

async fn review_create(
    client: &dyn OperationClient,
    scope: &Scope,
    input: &CreateInput,
) -> bilibi_agent_api::Result<Value> {
    let facade = facade()?;
    let db = facade.db.get().await.map_err(Error::from)?;
    authorization::authorize(&**db, scope, input.company_id, MANAGE)
        .await
        .map_err(OrganisationError::from)
        .map_err(refusal)?;
    facade
        .live_company(&**db, scope, input.company_id)
        .await
        .map_err(refusal)?;
    let new = NewPosition {
        code: input.code.clone(),
        parent_id: input.parent_id,
    };
    let valid = positions::validate_position(&new)
        .map_err(|errors| refusal(OrganisationError::Invalid(errors)))?;
    if let Some(parent) = valid.parent_id {
        positions::find(&**db, input.company_id, parent)
            .await
            .map_err(refusal)?
            .ok_or(AgentError::NotFound)?;
    }
    positions::check_code(&**db, input.company_id, &valid.code)
        .await
        .map_err(refusal)?;
    let _ = client;
    Ok(
        json!({"effect":"create", "company_id":input.company_id, "code":valid.code, "parent_id":valid.parent_id}),
    )
}

async fn review_parent(
    client: &dyn OperationClient,
    scope: &Scope,
    input: &ParentInput,
) -> bilibi_agent_api::Result<Value> {
    let facade = facade()?;
    let db = facade.db.get().await.map_err(Error::from)?;
    authorization::authorize(&**db, scope, input.company_id, MANAGE)
        .await
        .map_err(OrganisationError::from)
        .map_err(refusal)?;
    facade
        .live_company(&**db, scope, input.company_id)
        .await
        .map_err(refusal)?;
    let position = positions::find(&**db, input.company_id, input.position_id)
        .await
        .map_err(refusal)?
        .ok_or(AgentError::NotFound)?;
    if let Some(parent_id) = input.parent_id {
        positions::find(&**db, input.company_id, parent_id)
            .await
            .map_err(refusal)?
            .ok_or(AgentError::NotFound)?;
        positions::reject_cycle(&**db, input.company_id, position.id, Some(parent_id))
            .await
            .map_err(refusal)?;
    }
    let _ = client;
    Ok(
        json!({"effect":"move", "company_id":input.company_id, "position_id":position.id, "code":position.code, "previous_parent_id":position.parent_id, "parent_id":input.parent_id}),
    )
}

async fn review_version(
    client: &dyn OperationClient,
    scope: &Scope,
    input: &VersionInput,
    new: &NewVersion,
) -> bilibi_agent_api::Result<Value> {
    let facade = facade()?;
    let db = facade.db.get().await.map_err(Error::from)?;
    authorization::authorize(&**db, scope, input.company_id, MANAGE)
        .await
        .map_err(OrganisationError::from)
        .map_err(refusal)?;
    facade
        .live_company(&**db, scope, input.company_id)
        .await
        .map_err(refusal)?;
    let position = positions::find(&**db, input.company_id, input.position_id)
        .await
        .map_err(refusal)?
        .ok_or(AgentError::NotFound)?;
    let valid = positions::validate_version(new)
        .map_err(|errors| refusal(OrganisationError::Invalid(errors)))?;
    positions::check_version(&**db, position.id, &valid)
        .await
        .map_err(refusal)?;
    let _ = client;
    Ok(
        json!({"effect":"append_version", "company_id":input.company_id, "position_id":position.id, "code":position.code, "version":valid.version, "title":valid.title, "effective_from":valid.effective_from.to_string(), "effective_to":valid.effective_to.map(|date| date.to_string())}),
    )
}

struct Create;
impl WriteHandler for Create {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn preview<'a>(
        &'a self,
        client: &'a dyn OperationClient,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            review_create(client, scope, &parse(input)?)
                .await
                .map(Into::into)
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let input: CreateInput = parse(input)?;
            let position = facade()?
                .create_position_in(
                    tx,
                    scope,
                    input.company_id,
                    &NewPosition {
                        code: input.code,
                        parent_id: input.parent_id,
                    },
                )
                .await
                .map_err(refusal)?;
            Ok(json!(position).into())
        })
    }
}

struct Version;
impl WriteHandler for Version {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn preview<'a>(
        &'a self,
        client: &'a dyn OperationClient,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let (input, new) = version_input(input)?;
            review_version(client, scope, &input, &new)
                .await
                .map(Into::into)
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let (input, new) = version_input(input)?;
            let version = facade()?
                .record_version_in(tx, scope, input.company_id, input.position_id, &new)
                .await
                .map_err(refusal)?;
            Ok(json!(version).into())
        })
    }
}

struct Parent;
impl WriteHandler for Parent {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn preview<'a>(
        &'a self,
        client: &'a dyn OperationClient,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            review_parent(client, scope, &parse(input)?)
                .await
                .map(Into::into)
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let input: ParentInput = parse(input)?;
            let position = facade()?
                .set_parent_in(
                    tx,
                    scope,
                    input.company_id,
                    input.position_id,
                    input.parent_id,
                )
                .await
                .map_err(refusal)?;
            Ok(json!(position).into())
        })
    }
}

static CREATE: Create = Create;
static VERSION: Version = Version;
static PARENT: Parent = Parent;
static OPERATIONS: [Operation; 3] = [
    Operation {
        key: "people.organisation.position.create",
        title: "Create a position",
        summary: "Create a position with a company-unique code and an optional parent.",
        keywords: &["organisation", "position", "create"],
        capability: MANAGE,
        input: create_schema,
        output: || json!({"type":"object","description":"The created position: id, platform_company_id, code and parent_id."}),
        handler: Handler::SafeWrite(&CREATE),
        channels: Channels::Every,
        step_up: &[],
    },
    Operation {
        key: "people.organisation.position.version",
        title: "Add a position title version",
        summary: "Append an effective-dated title version to a position.",
        keywords: &["organisation", "position", "title", "version"],
        capability: MANAGE,
        input: version_schema,
        output: || json!({"type":"object","description":"The appended position title version, including its id and effective dates."}),
        handler: Handler::SafeWrite(&VERSION),
        channels: Channels::Every,
        step_up: &[],
    },
    Operation {
        key: "people.organisation.position.parent",
        title: "Move a position",
        summary: "Set a position's parent in the same company, or move it to the root.",
        keywords: &["organisation", "position", "parent", "reporting tree"],
        capability: MANAGE,
        input: parent_schema,
        output: || json!({"type":"object","description":"The position with its new parent_id."}),
        handler: Handler::SafeWrite(&PARENT),
        channels: Channels::Every,
        step_up: &[],
    },
];
static GUIDES: [Guide; 1] = [Guide {
    key: "people.organisation.position", title: "Manage positions",
    summary: "Create a position, place it in the reporting tree, and add an effective-dated title version.",
    keywords: &["position", "organisation", "title", "version", "parent"],
    capability: MANAGE, body: include_str!("../position-guide.md"),
}];
pub(super) static DEFINITIONS_VALUE: Definitions = Definitions {
    operations: &OPERATIONS,
    guides: &GUIDES,
};

fn create_schema() -> Value {
    json!({"type":"object","required":["company_id","code"],"properties":{"company_id":{"type":"integer","minimum":1},"code":{"type":"string","minLength":1,"maxLength":255},"parent_id":{"type":["integer","null"],"minimum":1}}})
}
fn version_schema() -> Value {
    json!({"type":"object","required":["company_id","position_id","version","title","effective_from"],"properties":{"company_id":{"type":"integer","minimum":1},"position_id":{"type":"integer","minimum":1},"version":{"type":"integer","minimum":1},"title":{"type":"string","minLength":1,"maxLength":255},"effective_from":{"type":"string","format":"date"},"effective_to":{"type":["string","null"],"format":"date"}}})
}
fn parent_schema() -> Value {
    json!({"type":"object","required":["company_id","position_id","parent_id"],"properties":{"company_id":{"type":"integer","minimum":1},"position_id":{"type":"integer","minimum":1},"parent_id":{"type":["integer","null"],"minimum":1}}})
}
