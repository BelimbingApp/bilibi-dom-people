//! One-door proof on the task's disposable launcher database, after seed-production.
//! Creates its own people and records through the public owners; no fixture accounts.
use bilibi_agent_api::plans::{PlanAuthority, PlanIdentity, PlannedOperation, Plans};
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_workforce::{NewEmployment, Workforce};
use bilibi_tenancy::{ProofFuture, Scope};
use bilibi_user::UserService;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_postgres::Transaction;

struct Browser<'a> {
    users: &'a UserService,
    token: &'a str,
}
impl PlanAuthority for Browser<'_> {
    fn verify<'a>(&'a self, tx: &'a Transaction<'_>) -> ProofFuture<'a, PlanIdentity> {
        Box::pin(async move {
            tx.query_opt(
                "SELECT id FROM sessions WHERE id=$1 AND expires_at>clock_timestamp() FOR UPDATE",
                &[&self.token],
            )
            .await?
            .ok_or(Error::Unauthorized)?;
            Ok(PlanIdentity {
                scope: self.users.sessions.check(tx, self.token).await?,
                credential_binding: format!("one-door:{}", self.token),
            })
        })
    }
}
fn fail(e: impl std::fmt::Display) -> Error {
    eprintln!("one-door proof: {e}");
    Error::Invalid
}
async fn operation(services: &Services, email: &str, key: &str, input: Value) -> Result<()> {
    let users = services.get::<UserService>("core/user.accounts")?;
    let login = users.login(email, "bilibi-dev").await.map_err(fail)?;
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    let client = db.get().await?;
    let scope = users.sessions.check(&**client, &login.token).await?;
    drop(client);
    let authority = Browser {
        users: &users,
        token: &login.token,
    };
    let plans = services.get::<Plans>("base/agent_api.plans")?;
    let operations = [PlannedOperation {
        key: key.into(),
        input,
    }];
    let preview = plans.preview(&authority, &operations).await.map_err(fail)?;
    if preview["valid"] != true {
        return Err(fail(format!("{key}: invalid preview")));
    }
    let code = preview["plan_code"].as_str().ok_or(Error::Invalid)?;
    plans
        .commit(&scope, &authority, code, &operations)
        .await
        .map_err(fail)?;
    println!("agent preview and commit passed: {key}");
    Ok(())
}
async fn account(
    services: &Services,
    admin: &Scope,
    company: i64,
    name: &str,
    email: &str,
    number: &str,
    supervisor: Option<i64>,
) -> Result<(i64, i64)> {
    let persons = services.get::<bilibi_person::PersonService>(bilibi_person::SERVICE)?;
    let person = persons
        .create(
            admin,
            &bilibi_person::PersonInput {
                name: name.into(),
                kind: bilibi_person::PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(fail)?;
    let affiliations =
        services.get::<bilibi_affiliation::AffiliationService>("core/affiliation.affiliations")?;
    let affiliation = affiliations
        .create(
            admin,
            &bilibi_affiliation::AffiliationInput {
                person_id: person.id,
                organisation_id: bilibi_party_ids::OrganisationId::new(company).map_err(fail)?,
                title: None,
                work_email: None,
                work_phone: None,
                dates: bilibi_affiliation::EffectiveDates::new(None, None).map_err(fail)?,
            },
        )
        .await
        .map_err(fail)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let user = users
        .create_account(
            admin,
            &bilibi_user::AccountInput {
                name: name.into(),
                email: email.into(),
                password: "bilibi-dev".into(),
                person: Some(person.id),
                affiliation: Some(affiliation.id),
            },
        )
        .await
        .map_err(fail)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let mut input = NewEmployment::new(
        person.id.get(),
        affiliation.id.get(),
        number,
        "employment",
        "active",
    );
    input.supervisor_employment_id = supervisor;
    let employment = workforce
        .create_employment(admin, company, &input)
        .await
        .map_err(fail)?;
    Ok((user.id, employment.id))
}
async fn login_scope(services: &Services, email: &str) -> Result<Scope> {
    let users = services.get::<UserService>("core/user.accounts")?;
    let login = users.login(email, "bilibi-dev").await.map_err(fail)?;
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    let client = db.get().await?;
    users.sessions.check(&**client, &login.token).await
}
async fn services(url: &str) -> Result<Services> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    Ok(services)
}
async fn run(url: &str) -> Result<()> {
    let services = services(url).await?;
    let admin = login_scope(&services, "ai@agent.my").await?;
    let companies = services.get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?;
    let company = match companies
        .create_company(
            &admin,
            &bilibi_company::NewCompany {
                name: "One-door proof".into(),
                code: Some("onedoor-proof".into()),
                status: Some("active".into()),
                ..Default::default()
            },
        )
        .await?
    {
        bilibi_company::CreateOutcome::Created(c) => c.id,
        bilibi_company::CreateOutcome::Refused(e) => return Err(fail(format!("company: {e:?}"))),
    };
    let (manager_id, manager_employee) = account(
        &services,
        &admin,
        company,
        "One-door reviewer",
        "onedoor-reviewer@example.test",
        "OD-REVIEW",
        None,
    )
    .await?;
    let (staff_id, staff_employee) = account(
        &services,
        &admin,
        company,
        "One-door employee",
        "onedoor-employee@example.test",
        "OD-STAFF",
        Some(manager_employee),
    )
    .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    for key in bilibi_people_workforce::CAPABILITIES
        .into_iter()
        .chain(bilibi_people_claims::CAPABILITIES)
        .chain(bilibi_people_training::CAPABILITIES)
        .chain(bilibi_people_leave::CAPABILITIES)
    {
        grants
            .set_direct(&admin, manager_id, key, Some(true))
            .await?;
    }
    for key in [
        bilibi_people_workforce::EMPLOYMENT_VIEW,
        bilibi_people_claims::SUBMIT,
        bilibi_people_training::LEARNING_VIEW,
        bilibi_people_training::REQUESTS_SUBMIT,
        bilibi_people_leave::SELF_VIEW,
    ] {
        grants.set_direct(&admin, staff_id, key, Some(true)).await?;
    }
    let manager = login_scope(&services, "onedoor-reviewer@example.test").await?;
    let staff = login_scope(&services, "onedoor-employee@example.test").await?;
    let claims = services.get::<bilibi_people_claims::Claims>(bilibi_people_claims::SERVICE)?;
    claims
        .put_currencies(&admin, company, &["USD".into()])
        .await
        .map_err(fail)?;
    let category = claims
        .create_category(
            &admin,
            company,
            &bilibi_people_claims::NewCategory {
                code: "onedoor".into(),
                name: "Proof expenses".into(),
            },
        )
        .await
        .map_err(fail)?;
    let kind = claims
        .create_claim_type(
            &admin,
            company,
            &bilibi_people_claims::NewClaimType {
                category_id: category.id,
                code: "onedoor".into(),
                name: "Proof claim".into(),
                receipt_requirement: "never".into(),
                eligibility: None,
            },
        )
        .await
        .map_err(fail)?;
    claims
        .create_policy(
            &admin,
            company,
            &bilibi_people_claims::NewPolicy {
                claim_type_id: kind.id,
                effective_from: "2026-01-01".parse().map_err(fail)?,
                effective_to: None,
                currency: "USD".into(),
                per_claim_limit: None,
                monthly_limit: None,
                yearly_limit: None,
                receipt_threshold: None,
            },
        )
        .await
        .map_err(fail)?;
    for amount in ["10", "20", "30"] {
        operation(&services,"onedoor-employee@example.test","people.claims.request.submit",json!({"company_id":company,"claim_type_id":kind.id,"incurred_on":"2026-10-30","amount":amount,"currency":"USD","description":"One-door proof"})).await?;
    }
    let own = claims.self_requests(&staff, company).await.map_err(fail)?;
    let paid = own[0].id;
    let withdrawn = own[1].id;
    let pending_claim = own[2].id;
    operation(
        &services,
        "onedoor-reviewer@example.test",
        "people.claims.request.approve",
        json!({"company_id":company,"request_id":paid}),
    )
    .await?;
    operation(
        &services,
        "onedoor-reviewer@example.test",
        "people.claims.request.reimburse",
        json!({"company_id":company,"request_id":paid,"payment_reference":"OD-paid"}),
    )
    .await?;
    operation(
        &services,
        "onedoor-employee@example.test",
        "people.claims.request.withdraw",
        json!({"company_id":company,"request_id":withdrawn}),
    )
    .await?;
    operation(&services,"onedoor-reviewer@example.test","people.claims.request.submit",json!({"company_id":company,"claim_type_id":kind.id,"incurred_on":"2026-10-30","amount":"40","currency":"USD"})).await?;
    let manager_claim = claims
        .self_requests(&manager, company)
        .await
        .map_err(fail)?[0]
        .id;
    if !claims
        .requests(&staff, company, false, "", None, Some(manager_claim))
        .await
        .map_err(fail)?
        .is_empty()
        || claims
            .requests(&staff, company, true, "", None, None)
            .await
            .is_ok()
    {
        return Err(fail("Claims privacy failed"));
    }
    for offset in 0_i64..501 {
        claims
            .submit_request(
                &staff,
                company,
                &bilibi_people_claims::NewRequest {
                    claim_type_id: kind.id,
                    incurred_on: "2026-10-30".parse().map_err(fail)?,
                    amount: rust_decimal::Decimal::from(1_000 + offset),
                    currency: "USD".into(),
                    description: Some(format!("Claim history {offset}")),
                    receipt_number: None,
                    confirm_duplicate: false,
                },
            )
            .await
            .map_err(fail)?;
    }
    let mine = claims
        .requests(&staff, company, false, "", None, None)
        .await
        .map_err(fail)?;
    let team = claims
        .requests(&manager, company, true, "", None, None)
        .await
        .map_err(fail)?;
    let employees = claims
        .request_employees(&manager, company, "")
        .await
        .map_err(fail)?;
    if mine.len() != 504
        || team.len() != 500
        || team.iter().any(|row| row.request.id == manager_claim)
        || !employees
            .iter()
            .any(|employee| employee.id == manager_employee)
    {
        return Err(fail("Claims history or employee filter window failed"));
    }
    let training =
        services.get::<bilibi_people_training::Training>(bilibi_people_training::SERVICE)?;
    training
        .put_learning_currencies(&admin, company, &["USD".into()])
        .await
        .map_err(fail)?;
    training
        .create_budget_policy(
            &admin,
            company,
            &bilibi_people_training::NewBudgetPolicy {
                currency: "USD".into(),
                effective_from: "2026-01-01".into(),
                effective_to: "2026-12-31".into(),
                amount: "1000".into(),
                reason: "Proof budget".into(),
            },
        )
        .await
        .map_err(fail)?;
    for need in ["Agent learning proof", "Browser learning proof"] {
        operation(&services,"onedoor-employee@example.test","people.training.request.create",json!({"company_id":company,"request":{"need":need,"objective":"Learn the workflow","expected_result":"Complete the proof","proposed_on":"2026-11-20","estimated_cost":"10","currency":"USD"}})).await?;
    }
    let requests = training
        .learning_requests(
            &staff,
            company,
            bilibi_people_training::LearningAudience::SelfService,
        )
        .await
        .map_err(fail)?;
    let learning = requests
        .iter()
        .find(|r| r.need == "Agent learning proof")
        .ok_or(Error::NotFound)?
        .id;
    for (email, key) in [
        ("onedoor-employee@example.test", "submit"),
        ("onedoor-reviewer@example.test", "recommend"),
        ("ai@agent.my", "review"),
        ("ai@agent.my", "approve"),
    ] {
        operation(
            &services,
            email,
            &format!("people.training.request.{key}"),
            json!({"company_id":company,"request_id":learning,"reason":"One-door agent proof"}),
        )
        .await?;
    }
    if training
        .learning_requests(
            &staff,
            company,
            bilibi_people_training::LearningAudience::Hr,
        )
        .await
        .is_ok()
    {
        return Err(fail("Learning privacy failed"));
    }
    let leave = services.get::<bilibi_people_leave::Leave>(bilibi_people_leave::SERVICE)?;
    let leave_type = leave
        .create_type(
            &admin,
            company,
            &bilibi_people_leave::NewLeaveType::new("onedoor", "Proof leave", "day"),
        )
        .await
        .map_err(fail)?;
    leave
        .record_entry(
            &admin,
            company,
            staff_employee,
            &bilibi_people_leave::NewEntry {
                leave_type_id: leave_type.id,
                entry_type: "opening".into(),
                quantity: rust_decimal::Decimal::from(20),
                occurred_on: "2026-10-31".parse().map_err(fail)?,
                source: "onedoor-proof".into(),
                entry_key: "staff-opening".into(),
                note: Some("Proof balance".into()),
            },
        )
        .await
        .map_err(fail)?;
    for (date, key) in [
        ("2026-11-20", "agent-leave"),
        ("2026-11-23", "browser-leave"),
    ] {
        operation(&services,"onedoor-employee@example.test","people.leave.request.submit",json!({"company_id":company,"leave_type_id":leave_type.id,"starts_on":date,"ends_on":date,"request_key":key,"reason":"One-door leave proof"})).await?;
    }
    operation(&services,"onedoor-employee@example.test","people.leave.request.submit",json!({"company_id":company,"leave_type_id":leave_type.id,"starts_on":"2026-11-20","ends_on":"2026-11-20","request_key":"agent-leave","reason":"One-door leave proof"})).await?;
    let requests = leave.self_requests(&staff, company).await.map_err(fail)?;
    let agent_leave = requests
        .iter()
        .find(|r| r.starts_on.to_string() == "2026-11-20")
        .ok_or(Error::NotFound)?
        .id;
    let browser_leave = requests
        .iter()
        .find(|r| r.starts_on.to_string() == "2026-11-23")
        .ok_or(Error::NotFound)?
        .id;
    let replayed = leave
        .submit_request(
            &staff,
            company,
            &bilibi_people_leave::NewRequest {
                leave_type_id: leave_type.id,
                starts_on: "2026-11-20".parse().map_err(fail)?,
                ends_on: "2026-11-20".parse().map_err(fail)?,
                day_part: "full".into(),
                hours: None,
                reason: Some("One-door leave proof".into()),
                request_key: "agent-leave".into(),
            },
        )
        .await
        .map_err(fail)?;
    if replayed.id != agent_leave {
        return Err(fail("Leave request replay failed"));
    }
    operation(
        &services,
        "onedoor-reviewer@example.test",
        "people.leave.request.approve",
        json!({"company_id":company,"request_id":agent_leave,"note":"One-door proof"}),
    )
    .await?;
    if leave.request(&manager, company, agent_leave).await.is_ok()
        || leave
            .request(&staff, company, agent_leave)
            .await
            .map_err(fail)?
            .request
            .status
            != "approved"
        || leave
            .request(&manager, company, browser_leave)
            .await
            .map_err(fail)?
            .request
            .status
            != "pending"
    {
        return Err(fail("Leave owner or pending-reviewer visibility failed"));
    }
    operation(
        &services,
        "onedoor-employee@example.test",
        "people.leave.request.cancel",
        json!({"company_id":company,"request_id":agent_leave}),
    )
    .await?;
    if leave
        .request(&staff, company, agent_leave)
        .await
        .map_err(fail)?
        .request
        .status
        != "cancelled"
        || leave.request(&manager, company, agent_leave).await.is_ok()
    {
        return Err(fail("Leave decided history privacy failed"));
    }
    println!(
        "One-door proof ready: {}",
        json!({"company":company,"staff":staff_id,"reviewer":manager_id,"pending_claim":pending_claim,"claim_of_reviewer":manager_claim,"learning_request":learning,"pending_leave":browser_leave})
    );
    Ok(())
}
walkthrough_command!("one-door", |url, _argument| async move { run(url).await });
