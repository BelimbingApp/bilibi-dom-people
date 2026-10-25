//! PC5 browser records, created only through the Claims facade.
use super::{ADMIN, COMPANY, MANAGER, STAFF, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::pool;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_claims::{
    Approval, Claims, NewCategory, NewClaimType, NewPolicy, NewRequest, Reimbursement, Rejection,
    APPROVE, REIMBURSE, SUBMIT,
};
use bilibi_tenancy::Authentication;
use jiff::civil::date;
use rust_decimal::Decimal;
use std::sync::Arc;

pub(super) async fn run(services: &Services) -> Result<()> {
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    let client = db.get().await?;
    let admin = auth
        .authenticate(
            &**client,
            TENANT,
            super::ADMIN,
            "claim-operations-proof".into(),
        )
        .await?;
    let manager = auth
        .authenticate(&**client, TENANT, MANAGER, "claim-operations-proof".into())
        .await?;
    drop(client);
    // The default walkthrough intentionally unlinks staff at its end.
    // Read employment facts from Workforce and restore the account through Core.
    let workforce =
        services.get::<bilibi_people_workforce::Workforce>(bilibi_people_workforce::SERVICE)?;
    let staff_employee = workforce
        .employees_at_any_status(&manager, COMPANY, None)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?
        .into_iter()
        .find(|employee| employee.employee_number == "E-002")
        .and_then(|employee| employee.reference.stable_id.parse::<i64>().ok())
        .ok_or(Error::NotFound)?;
    let employment = workforce
        .employment(&manager, COMPANY, staff_employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let users = services.get::<bilibi_user::UserService>("core/user.accounts")?;
    users
        .link_person(
            &admin,
            STAFF,
            bilibi_person::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(
            &admin,
            STAFF,
            bilibi_affiliation::AffiliationId::new(employment.affiliation_id)
                .map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    let client = db.get().await?;
    let staff = auth
        .authenticate(&**client, TENANT, STAFF, "claim-operations-proof".into())
        .await?;
    drop(client);
    let grants = services.get::<Grants>("base/authz.grants")?;
    // Viewer can read the queue but cannot pay, batch, or export.
    grants
        .set_direct(&admin, VIEWER, APPROVE, Some(true))
        .await?;
    grants
        .set_direct(&admin, VIEWER, REIMBURSE, Some(false))
        .await?;
    grants.set_direct(&admin, STAFF, SUBMIT, Some(true)).await?;
    let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
    let check = |error| {
        eprintln!("PC5 fixture: {error:?}");
        Error::Internal
    };
    claims
        .put_currencies(&manager, COMPANY, &["USD".into(), "EUR".into()])
        .await
        .map_err(check)?;
    let category = claims
        .create_category(
            &manager,
            COMPANY,
            &NewCategory {
                code: "operations-proof".into(),
                name: "Operations proof".into(),
            },
        )
        .await
        .map_err(check)?;
    let kind = claims
        .create_claim_type(
            &manager,
            COMPANY,
            &NewClaimType {
                category_id: category.id,
                code: "operations-proof".into(),
                name: "Operations expense".into(),
                receipt_requirement: "never".into(),
                eligibility: None,
            },
        )
        .await
        .map_err(check)?;
    claims
        .create_policy(
            &manager,
            COMPANY,
            &NewPolicy {
                claim_type_id: kind.id,
                effective_from: date(2025, 1, 1),
                effective_to: None,
                currency: "USD".into(),
                per_claim_limit: None,
                monthly_limit: None,
                yearly_limit: None,
                receipt_threshold: None,
            },
        )
        .await
        .map_err(check)?;
    for (amount, status) in [
        (11, "submitted"),
        (12, "approved"),
        (13, "rejected"),
        (14, "reimbursed"),
        (15, "batch"),
        (16, "submitted"),
    ] {
        let request = claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    claim_type_id: kind.id,
                    incurred_on: date(2025, 1, 15),
                    amount: Decimal::new(amount, 0),
                    currency: "USD".into(),
                    description: Some("Browser interaction proof".into()),
                    receipt_number: None,
                    confirm_duplicate: false,
                },
            )
            .await
            .map_err(check)?;
        if matches!(status, "approved" | "reimbursed" | "batch") {
            claims
                .approve_request(&manager, COMPANY, request.id, &Approval::default())
                .await
                .map_err(check)?;
        }
        if status == "rejected" {
            claims
                .reject_request(
                    &manager,
                    COMPANY,
                    request.id,
                    &Rejection {
                        decision_reason: "Outside the policy".into(),
                    },
                )
                .await
                .map_err(check)?;
        }
        if status == "reimbursed" {
            claims
                .reimburse_request(
                    &manager,
                    COMPANY,
                    request.id,
                    &Reimbursement {
                        payment_reference: Some("proof payment".into()),
                    },
                )
                .await
                .map_err(check)?;
        }
        println!(
            "PC5 browser claim {}: {status}, amount {amount}",
            request.id
        );
    }
    let own_claim = claims
        .submit_request(
            &manager,
            COMPANY,
            &NewRequest {
                claim_type_id: kind.id,
                incurred_on: date(2025, 1, 15),
                amount: Decimal::new(17, 0),
                currency: "USD".into(),
                description: Some("Own-claim refusal proof".into()),
                receipt_number: None,
                confirm_duplicate: false,
            },
        )
        .await
        .map_err(check)?;
    println!("PC5 manager own claim {}: submitted", own_claim.id);
    let batch = claims
        .create_handoff_batch(&manager, COMPANY, "USD")
        .await
        .map_err(check)?;
    println!(
        "PC5 browser batch {}: /people/claims/operations?company_id=1&tab=batches",
        batch.id
    );
    Ok(())
}

async fn command(url: &str, command: Option<&str>) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let client = db.get().await?;
    let fixture: bool = client.query_one(
        "SELECT EXISTS(SELECT 1 FROM users WHERE tenant_id=1 AND id=1 AND email='manager@example.test') AND EXISTS(SELECT 1 FROM users WHERE tenant_id=1 AND id=4 AND email='admin@example.test')",
        &[],
    ).await?.get(0);
    if !fixture {
        return Err(Error::Invalid);
    }
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    // The same explicit Dev mode as the default disposable walkthrough.
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    if let Some(command) = command {
        let (capability, enabled) = match command {
            "revoke-reimburse" => (bilibi_people_claims::REIMBURSE, false),
            "restore-reimburse" => (bilibi_people_claims::REIMBURSE, true),
            "revoke-approve" => (bilibi_people_claims::APPROVE, false),
            "restore-approve" => (bilibi_people_claims::APPROVE, true),
            _ => return Err(Error::Invalid),
        };
        let admin = services
            .get::<Authentication>("base/tenancy.authentication")?
            .authenticate(&**client, TENANT, ADMIN, "claim-operations-proof".into())
            .await?;
        services
            .get::<Grants>("base/authz.grants")?
            .set_direct(&admin, MANAGER, capability, Some(enabled))
            .await?;
        println!("PC5 manager grant {capability}: {enabled}");
        Ok(())
    } else {
        drop(client);
        run(&services).await
    }
}

walkthrough_command!("claim-operations", |url, argument| async move {
    command(url, argument).await
});
