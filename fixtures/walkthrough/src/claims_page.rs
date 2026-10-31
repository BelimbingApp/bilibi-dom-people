//! PC3's disposable browser records and two-session steering, all through owners.
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_claims::{
    Approval, Claims, ClaimsError, NewCategory, NewClaimType, NewPolicy, NewRequest, Reimbursement,
    Rejection, SUBMIT,
};
use bilibi_tenancy::Authentication;
use jiff::civil::date;
use rust_decimal::Decimal;

fn saved<T>(value: std::result::Result<T, ClaimsError>) -> Result<T> {
    value.map_err(|error| {
        eprintln!("PC3: {error}");
        Error::Invalid
    })
}
pub(crate) async fn run(url: &str, action: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(url)?;
    let mut services = Services::default();
    services.provide("bootstrap.database", std::sync::Arc::new(db.clone()))?;
    services.provide(
        "bootstrap.composition",
        std::sync::Arc::new(bilibi_web::composition()),
    )?;
    services.install(&bilibi_module_registry::discover()?)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, 1, 4, "pc3-proof".into())
        .await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "pc3-proof".into())
        .await?;
    let staff = auth
        .authenticate(&**client, 1, 3, "pc3-proof".into())
        .await?;
    let grants = services.get::<bilibi_authz::grants::Grants>("base/authz.grants")?;
    let users = services.get::<bilibi_user::UserService>("core/user.accounts")?;
    match action {
        "revoke" | "restore" => {
            grants
                .set_direct(&admin, 3, SUBMIT, Some(action == "restore"))
                .await?;
        }
        "unlink" => {
            users.unlink(&admin, 3).await.map_err(|_| Error::Invalid)?;
        }
        "relink" => {
            let workforce = services
                .get::<bilibi_people_workforce::Workforce>(bilibi_people_workforce::SERVICE)?;
            let employees = workforce
                .employees_at_any_status(&manager, 1, None)
                .await
                .map_err(|_| Error::Invalid)?;
            let employee = employees
                .require_current()
                .map_err(|_| Error::Invalid)?
                .into_iter()
                .find(|e| e.employee_number == "E-002")
                .ok_or(Error::NotFound)?;
            let id = employee
                .reference
                .stable_id
                .parse()
                .map_err(|_| Error::Invalid)?;
            let employment = workforce
                .employment(&manager, 1, id)
                .await
                .map_err(|_| Error::Invalid)?
                .require_current()
                .map_err(|_| Error::Invalid)?;
            crate::give_staff_desk(
                &users,
                &admin,
                3,
                bilibi_party_ids::PersonId::new(employment.person_id)
                    .map_err(|_| Error::Invalid)?,
                bilibi_party_ids::OrganisationId::new(employment.platform_company_id)
                    .map_err(|_| Error::Invalid)?,
            )
            .await
            .map_err(|_| Error::Invalid)?;
        }
        "prepare" => {
            let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
            let category = saved(
                claims
                    .create_category(
                        &manager,
                        1,
                        &NewCategory {
                            code: "pc3_expenses".into(),
                            name: "Expense category".into(),
                        },
                    )
                    .await,
            )?;
            let mut types = Vec::new();
            for (code, name, receipt, threshold) in [
                ("pc3_general", "General expense", "never", None),
                ("pc3_receipt", "Receipt expense", "always", None),
                (
                    "pc3_threshold",
                    "Threshold expense",
                    "above_threshold",
                    Some(Decimal::from(25)),
                ),
            ] {
                let kind = saved(
                    claims
                        .create_claim_type(
                            &manager,
                            1,
                            &NewClaimType {
                                category_id: category.id,
                                code: code.into(),
                                name: name.into(),
                                receipt_requirement: receipt.into(),
                                eligibility: None,
                            },
                        )
                        .await,
                )?;
                saved(
                    claims
                        .create_policy(
                            &manager,
                            1,
                            &NewPolicy {
                                claim_type_id: kind.id,
                                effective_from: date(2026, 1, 1),
                                effective_to: None,
                                currency: "USD".into(),
                                per_claim_limit: Some(Decimal::from(500)),
                                monthly_limit: Some(Decimal::from(5000)),
                                yearly_limit: Some(Decimal::from(50000)),
                                receipt_threshold: threshold,
                            },
                        )
                        .await,
                )?;
                types.push(kind.id);
            }
            for (amount, state) in [
                (40, "submitted"),
                (50, "approved"),
                (60, "rejected"),
                (70, "reimbursed"),
            ] {
                let request = saved(
                    claims
                        .submit_request(
                            &staff,
                            1,
                            &NewRequest {
                                claim_type_id: types[0],
                                incurred_on: date(2026, 10, 1),
                                amount: Decimal::from(amount),
                                currency: "USD".into(),
                                description: Some(format!("PC3 {state} browser proof")),
                                receipt_number: None,
                                confirm_duplicate: false,
                            },
                        )
                        .await,
                )?;
                match state {
                    "approved" | "reimbursed" => {
                        saved(
                            claims
                                .approve_request(
                                    &manager,
                                    1,
                                    request.id,
                                    &Approval {
                                        approved_amount: Some(Decimal::from(amount - 5)),
                                        decision_reason: Some(
                                            "Adjusted to eligible expense".into(),
                                        ),
                                    },
                                )
                                .await,
                        )?;
                        if state == "reimbursed" {
                            saved(
                                claims
                                    .reimburse_request(
                                        &manager,
                                        1,
                                        request.id,
                                        &Reimbursement::default(),
                                    )
                                    .await,
                            )?;
                        }
                    }
                    "rejected" => {
                        saved(
                            claims
                                .reject_request(
                                    &manager,
                                    1,
                                    request.id,
                                    &Rejection {
                                        decision_reason: "Outside the claim policy".into(),
                                    },
                                )
                                .await,
                        )?;
                    }
                    _ => {}
                }
            }
        }
        _ => return Err(Error::Invalid),
    }
    println!("PC3 browser proof: {action} complete");
    Ok(())
}

walkthrough_command!("claims-page", |url, argument| async move {
    run(url, argument.unwrap_or("prepare")).await
});
