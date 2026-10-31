//! PC2: running-system proof on the disposable walkthrough database.
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_claims::{
    Approval, Claims, ClaimsError, NewCategory, NewClaimType, NewPolicy, NewRequest, Reimbursement,
    Rejection, APPROVE, REIMBURSE, SUBMIT,
};
use bilibi_people_workforce::{EmploymentChange, Workforce};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_user::UserService;
use jiff::civil::date;
use rust_decimal::Decimal;
use tokio_postgres::Client;
const COMPANY: i64 = 1;
const STAFF: i64 = 3;

fn saved<T>(step: &str, result: std::result::Result<T, ClaimsError>) -> Result<T> {
    match result {
        Ok(value) => {
            println!("PC2 {step}: saved");
            Ok(value)
        }
        Err(error) => {
            eprintln!("PC2 {step}: {error:?}");
            Err(Error::Internal)
        }
    }
}
fn refuses<T>(
    step: &str,
    result: std::result::Result<T, ClaimsError>,
    expected: ClaimsError,
) -> Result<()> {
    match result {
        Err(error) if std::mem::discriminant(&error) == std::mem::discriminant(&expected) => {
            println!("PC2 {step}: refused ({error})");
            Ok(())
        }
        Err(error) => {
            eprintln!("PC2 {step}: expected {expected:?}, got {error:?}");
            Err(Error::Internal)
        }
        Ok(_) => {
            eprintln!("PC2 {step}: unexpectedly accepted");
            Err(Error::Internal)
        }
    }
}
fn ensure(step: &str, condition: bool) -> Result<()> {
    if condition {
        println!("PC2 {step}: proven");
        Ok(())
    } else {
        eprintln!("PC2 {step}: failed");
        Err(Error::Internal)
    }
}
fn invalid() -> ClaimsError {
    ClaimsError::Invalid(Vec::new())
}
fn amount(value: i64) -> Decimal {
    Decimal::from(value)
}

pub(crate) async fn run(
    services: &Services,
    client: &Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    employees: (i64, i64),
) -> Result<()> {
    let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let staff_employment = workforce
        .employment(manager, COMPANY, employees.1)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        admin,
        STAFF,
        bilibi_party_ids::PersonId::new(staff_employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(staff_employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    for capability in [SUBMIT, APPROVE, REIMBURSE] {
        grants
            .set_direct(admin, STAFF, capability, Some(true))
            .await?;
    }
    let staff = authentication
        .authenticate(client, 1, STAFF, "pc2-proof".into())
        .await?;
    saved(
        "configure currencies",
        claims
            .put_currencies(manager, COMPANY, &["USD".into(), "EUR".into()])
            .await,
    )?;
    let category = saved(
        "create workflow category",
        claims
            .create_category(
                manager,
                COMPANY,
                &NewCategory {
                    code: "claim-workflow".into(),
                    name: "Workflow claims".into(),
                },
            )
            .await,
    )?;
    let mut types = Vec::new();
    for (code, receipt, eligibility) in [
        ("limited", "always", "all_employees"),
        ("threshold", "above_threshold", "all_employees"),
        ("assigned", "never", "assigned_only"),
        ("other-currency", "never", "all_employees"),
    ] {
        let kind = saved(
            "create workflow claim type",
            claims
                .create_claim_type(
                    manager,
                    COMPANY,
                    &NewClaimType {
                        category_id: category.id,
                        code: code.into(),
                        name: code.into(),
                        receipt_requirement: receipt.into(),
                        eligibility: Some(eligibility.into()),
                    },
                )
                .await,
        )?;
        saved(
            "create workflow policy",
            claims
                .create_policy(
                    manager,
                    COMPANY,
                    &NewPolicy {
                        claim_type_id: kind.id,
                        effective_from: date(2025, 1, 1),
                        effective_to: None,
                        currency: if code == "other-currency" {
                            "EUR".into()
                        } else {
                            "USD".into()
                        },
                        per_claim_limit: (code == "limited").then(|| amount(100)),
                        monthly_limit: (code == "limited").then(|| amount(150)),
                        yearly_limit: (code == "limited").then(|| amount(200)),
                        receipt_threshold: (receipt == "above_threshold").then(|| amount(20)),
                    },
                )
                .await,
        )?;
        types.push(kind.id);
    }
    let input = NewRequest {
        claim_type_id: types[0],
        incurred_on: date(2025, 1, 10),
        amount: amount(100),
        currency: " usd ".into(),
        description: Some("=SUM(1,2)\r\n\"separate expense\"".into()),
        receipt_number: Some("  inv   one  ".into()),
        confirm_duplicate: false,
    };
    refuses(
        "unlinked admin submits",
        claims.submit_request(admin, COMPANY, &input).await,
        ClaimsError::NotLinked,
    )?;
    refuses(
        "viewer submits without grant",
        claims.submit_request(viewer, COMPANY, &input).await,
        ClaimsError::Unauthorized,
    )?;
    refuses(
        "future expense",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    incurred_on: date(2099, 1, 1),
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::FutureIncurredOn,
    )?;
    refuses(
        "no policy on expense date",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    incurred_on: date(2024, 12, 31),
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::NoEffectivePolicy,
    )?;
    refuses(
        "currency differs from policy",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    currency: "EUR".into(),
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::CurrencyNotAllowed,
    )?;
    refuses(
        "required receipt missing",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    receipt_number: None,
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::ReceiptRequired,
    )?;
    refuses(
        "per claim limit",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: amount(101),
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::PerClaimLimitExceeded,
    )?;
    refuses(
        "fractional third decimal",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: Decimal::new(101, 3),
                    ..input.clone()
                },
            )
            .await,
        invalid(),
    )?;
    refuses(
        "assignment required on expense date",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    claim_type_id: types[2],
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::ClaimTypeNotAssigned,
    )?;
    saved(
        "temporarily remove policy currency",
        claims
            .put_currencies(manager, COMPANY, &["EUR".into()])
            .await,
    )?;
    refuses(
        "removed currency closes new claims",
        claims.submit_request(&staff, COMPANY, &input).await,
        ClaimsError::CurrencyNotAllowed,
    )?;
    saved(
        "restore policy currencies",
        claims
            .put_currencies(manager, COMPANY, &["USD".into(), "EUR".into()])
            .await,
    )?;
    let assignment = saved(
        "assignment covering expense date only",
        claims
            .create_assignment(
                manager,
                COMPANY,
                &bilibi_people_claims::NewAssignment {
                    code: "workflow-assigned".into(),
                    name: "Expense-date assignment".into(),
                    effective_from: date(2025, 1, 1),
                    effective_to: Some(date(2025, 1, 31)),
                },
            )
            .await,
    )?;
    saved(
        "assign claim type and employee",
        claims
            .set_assignment_members(manager, COMPANY, assignment.id, &[types[2]], &[employees.1])
            .await,
    )?;
    let assigned = saved(
        "assignment uses expense date rather than today",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    claim_type_id: types[2],
                    receipt_number: None,
                    ..input.clone()
                },
            )
            .await,
    )?;
    saved(
        "withdraw assignment proof",
        claims.withdraw_request(&staff, COMPANY, assigned.id).await,
    )?;
    let first = saved(
        "submit staff claim",
        claims.submit_request(&staff, COMPANY, &input).await,
    )?;
    ensure(
        "receipt canonical form",
        first.receipt_number.as_deref() == Some("INV ONE"),
    )?;
    refuses(
        "normalised duplicate receipt",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: amount(10),
                    receipt_number: Some("inv one".into()),
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::DuplicateReceipt,
    )?;
    refuses(
        "monthly limit",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: amount(51),
                    receipt_number: Some("two".into()),
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::MonthlyLimitExceeded,
    )?;
    refuses(
        "own approval",
        claims
            .approve_request(&staff, COMPANY, first.id, &Approval::default())
            .await,
        ClaimsError::OwnClaim,
    )?;
    refuses(
        "approval exceeds claim",
        claims
            .approve_request(
                manager,
                COMPANY,
                first.id,
                &Approval {
                    approved_amount: Some(amount(101)),
                    decision_reason: None,
                },
            )
            .await,
        invalid(),
    )?;
    refuses(
        "partial approval needs reason",
        claims
            .approve_request(
                manager,
                COMPANY,
                first.id,
                &Approval {
                    approved_amount: Some(amount(80)),
                    decision_reason: Some(" ".into()),
                },
            )
            .await,
        invalid(),
    )?;
    refuses(
        "own rejection",
        claims
            .reject_request(
                &staff,
                COMPANY,
                first.id,
                &Rejection {
                    decision_reason: "Own rejection".into(),
                },
            )
            .await,
        ClaimsError::OwnClaim,
    )?;
    saved(
        "approve staff claim for 80 with reason",
        claims
            .approve_request(
                manager,
                COMPANY,
                first.id,
                &Approval {
                    approved_amount: Some(amount(80)),
                    decision_reason: Some("Adjusted to evidenced expense".into()),
                },
            )
            .await,
    )?;
    refuses(
        "own individual reimbursement",
        claims
            .reimburse_request(&staff, COMPANY, first.id, &Reimbursement::default())
            .await,
        ClaimsError::OwnClaim,
    )?;
    refuses(
        "approved claim cannot withdraw",
        claims.withdraw_request(&staff, COMPANY, first.id).await,
        ClaimsError::NotWithdrawable,
    )?;
    let second = saved(
        "approved amount frees monthly capacity",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: amount(70),
                    receipt_number: Some("two".into()),
                    ..input.clone()
                },
            )
            .await,
    )?;
    let limited_policy = saved(
        "read policy before version change",
        claims.policies(manager, COMPANY).await,
    )?
    .into_iter()
    .find(|p| p.claim_type_id == types[0])
    .ok_or(Error::Internal)?;
    saved(
        "end expense-date policy",
        claims
            .end_policy(manager, COMPANY, limited_policy.id, date(2025, 1, 31))
            .await,
    )?;
    saved(
        "create next policy version",
        claims
            .create_policy(
                manager,
                COMPANY,
                &NewPolicy {
                    claim_type_id: types[0],
                    effective_from: date(2025, 2, 1),
                    effective_to: None,
                    currency: "USD".into(),
                    per_claim_limit: Some(amount(100)),
                    monthly_limit: Some(amount(150)),
                    yearly_limit: Some(amount(200)),
                    receipt_threshold: None,
                },
            )
            .await,
    )?;
    refuses(
        "yearly limit across months and policy versions",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: amount(51),
                    incurred_on: date(2025, 2, 1),
                    receipt_number: Some("three".into()),
                    ..input.clone()
                },
            )
            .await,
        ClaimsError::YearlyLimitExceeded,
    )?;
    refuses(
        "another employee cannot withdraw",
        claims.withdraw_request(manager, COMPANY, second.id).await,
        ClaimsError::NotFound,
    )?;
    refuses(
        "reject needs reason",
        claims
            .reject_request(
                manager,
                COMPANY,
                second.id,
                &Rejection {
                    decision_reason: "  ".into(),
                },
            )
            .await,
        invalid(),
    )?;
    saved(
        "reject staff claim",
        claims
            .reject_request(
                manager,
                COMPANY,
                second.id,
                &Rejection {
                    decision_reason: "Outside this claim's scope".into(),
                },
            )
            .await,
    )?;
    saved(
        "rejected claim releases receipt and limit",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: amount(70),
                    receipt_number: Some("two".into()),
                    ..input.clone()
                },
            )
            .await,
    )?;
    let low = NewRequest {
        claim_type_id: types[1],
        amount: amount(20),
        receipt_number: None,
        description: None,
        ..input.clone()
    };
    saved(
        "threshold equal needs no receipt",
        claims.submit_request(&staff, COMPANY, &low).await,
    )?;
    refuses(
        "threshold above needs receipt",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    amount: amount(21),
                    ..low.clone()
                },
            )
            .await,
        ClaimsError::ReceiptRequired,
    )?;
    refuses(
        "possible duplicate",
        claims.submit_request(&staff, COMPANY, &low).await,
        ClaimsError::PossibleDuplicate,
    )?;
    let confirmed = saved(
        "confirmed possible duplicate",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    confirm_duplicate: true,
                    ..low.clone()
                },
            )
            .await,
    )?;
    ensure("confirmation recorded", confirmed.duplicate_confirmed)?;
    saved(
        "withdraw confirmed duplicate",
        claims.withdraw_request(&staff, COMPANY, confirmed.id).await,
    )?;
    refuses(
        "withdraw twice",
        claims.withdraw_request(&staff, COMPANY, confirmed.id).await,
        ClaimsError::NotWithdrawable,
    )?;
    let euro = saved(
        "separate currency submission",
        claims
            .submit_request(
                &staff,
                COMPANY,
                &NewRequest {
                    claim_type_id: types[3],
                    currency: "EUR".into(),
                    amount: amount(10),
                    receipt_number: None,
                    ..input.clone()
                },
            )
            .await,
    )?;
    saved(
        "approve separate currency",
        claims
            .approve_request(manager, COMPANY, euro.id, &Approval::default())
            .await,
    )?;
    let manager_claim = saved(
        "manager submits own expense",
        claims
            .submit_request(
                manager,
                COMPANY,
                &NewRequest {
                    amount: amount(30),
                    receipt_number: Some("manager expense".into()),
                    ..input.clone()
                },
            )
            .await,
    )?;
    saved(
        "staff approves manager's expense",
        claims
            .approve_request(&staff, COMPANY, manager_claim.id, &Approval::default())
            .await,
    )?;
    let batch = saved(
        "create USD hand off",
        claims.create_handoff_batch(manager, COMPANY, " usd ").await,
    )?;
    ensure(
        "one currency batch totals",
        batch.currency == "USD" && batch.request_count == 2 && batch.total_amount == amount(110),
    )?;
    refuses(
        "no approved USD left",
        claims.create_handoff_batch(manager, COMPANY, "USD").await,
        ClaimsError::NothingToHandOff,
    )?;
    let euro_batch = saved(
        "create EUR hand off",
        claims.create_handoff_batch(manager, COMPANY, "EUR").await,
    )?;
    ensure(
        "currencies remain separate",
        euro_batch.request_count == 1 && euro_batch.total_amount == amount(10),
    )?;
    let export = saved(
        "export stored batch",
        claims.handoff_export(manager, COMPANY, batch.id).await,
    )?;
    ensure(
        "CSV formula neutralisation and RFC 4180 quoting",
        export
            .content
            .contains("\"'=SUM(1,2)\r\n\"\"separate expense\"\"\"")
            && export.content.ends_with("\r\n"),
    )?;
    refuses(
        "viewer exports CSV",
        claims.handoff_export(viewer, COMPANY, batch.id).await,
        ClaimsError::Unauthorized,
    )?;
    refuses(
        "batch is company scoped",
        claims.handoff_export(manager, 2, batch.id).await,
        ClaimsError::NotFound,
    )?;
    refuses(
        "manager cannot reimburse mixed own batch",
        claims
            .reimburse_batch(manager, COMPANY, batch.id, &Reimbursement::default())
            .await,
        ClaimsError::OwnClaim,
    )?;
    let status=client.query_one("SELECT count(*) FROM people_claim_submissions WHERE handoff_batch_id=$1 AND status='approved'", &[&batch.id]).await?.get::<_,i64>(0);
    ensure("own-claim batch refusal rolls back all", status == 2)?;
    saved(
        "independent admin pays USD batch",
        claims
            .reimburse_batch(
                admin,
                COMPANY,
                batch.id,
                &Reimbursement {
                    payment_reference: Some("=payment,\"reference\"".into()),
                },
            )
            .await,
    )?;
    refuses(
        "empty reimbursement batch",
        claims
            .reimburse_batch(admin, COMPANY, batch.id, &Reimbursement::default())
            .await,
        ClaimsError::NothingToReimburse,
    )?;
    // Force the outbox insert to fail after the status/history writes. The
    // enclosing MutationBoundary must roll all of them back together.
    client.batch_execute("CREATE FUNCTION fixture_pc2_refuse_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.name='people/claims.request_reimbursed' THEN RAISE EXCEPTION 'PC2 outbox rollback proof'; END IF; RETURN NEW; END $$; CREATE TRIGGER fixture_pc2_refuse_event BEFORE INSERT ON base_events FOR EACH ROW EXECUTE FUNCTION fixture_pc2_refuse_event()").await?;
    ensure(
        "outbox failure refuses payment",
        matches!(
            claims
                .reimburse_request(manager, COMPANY, euro.id, &Reimbursement::default())
                .await,
            Err(ClaimsError::Failed(_))
        ),
    )?;
    client.batch_execute("DROP TRIGGER fixture_pc2_refuse_event ON base_events; DROP FUNCTION fixture_pc2_refuse_event()").await?;
    let euro_after = saved(
        "read claim after outbox rollback",
        claims
            .employee_requests(manager, COMPANY, employees.1)
            .await,
    )?
    .into_iter()
    .find(|r| r.id == euro.id)
    .ok_or(Error::Internal)?;
    ensure(
        "outbox failure rolls back payment and history",
        euro_after.status == "approved"
            && saved(
                "history after outbox rollback",
                claims
                    .request_events(manager, COMPANY, employees.1, euro.id)
                    .await,
            )?
            .len()
                == 2,
    )?;
    saved(
        "individual EUR reimbursement",
        claims
            .reimburse_request(manager, COMPANY, euro.id, &Reimbursement::default())
            .await,
    )?;
    let history = saved(
        "read status history",
        claims
            .request_events(manager, COMPANY, employees.1, first.id)
            .await,
    )?;
    ensure(
        "submission approval reimbursement history",
        history
            .iter()
            .map(|e| e.to_status.as_str())
            .collect::<Vec<_>>()
            == ["submitted", "approved", "reimbursed"]
            && history[1].reason.as_deref() == Some("Adjusted to evidenced expense"),
    )?;
    let ids = saved(
        "payroll source type IDs",
        claims
            .requested_claim_type_ids(manager, COMPANY, date(2025, 1, 1), date(2025, 12, 31))
            .await,
    )?;
    ensure(
        "only live types in payroll source",
        ids.contains(&types[0])
            && ids.contains(&types[1])
            && ids.contains(&types[3])
            && !ids.contains(&types[2]),
    )?;
    let events=client.query("SELECT name,version,payload FROM base_events WHERE publisher='people/claims' AND subject=$1 ORDER BY id", &[&format!("claim:{}",first.id)]).await?;
    ensure(
        "versioned events committed in order",
        events.len() == 2
            && events[0].get::<_, i32>("version") == 1
            && events[0].get::<_, String>("name") == bilibi_people_claims::REQUEST_APPROVED
            && events[1].get::<_, String>("name") == bilibi_people_claims::REQUEST_REIMBURSED,
    )?;
    let payload = events[0].get::<_, serde_json::Value>("payload");
    ensure(
        "exact event amount and category",
        payload["approved_amount"] == "80.00"
            && payload["category_code"] == "claim-workflow"
            && payload["claim_id"] == first.id,
    )?;
    println!(
        "PC2 browser download: /people/claims/handoff/{COMPANY}/{}",
        batch.id
    );

    // Separate transactions contend on the same company/employee locks.
    let racing = NewRequest {
        amount: amount(7),
        ..low.clone()
    };
    let (one, two) = tokio::join!(
        claims.submit_request(&staff, COMPANY, &racing),
        claims.submit_request(&staff, COMPANY, &racing)
    );
    let race_claim = match (one, two) {
        (Ok(saved), Err(ClaimsError::PossibleDuplicate))
        | (Err(ClaimsError::PossibleDuplicate), Ok(saved)) => saved,
        outcomes => {
            eprintln!("PC2 concurrent submission outcome: {outcomes:?}");
            return Err(Error::Internal);
        }
    };
    println!("PC2 concurrent duplicate submits: one saved, one refused");
    saved(
        "approve concurrent winner",
        claims
            .approve_request(manager, COMPANY, race_claim.id, &Approval::default())
            .await,
    )?;
    let (one, two) = tokio::join!(
        claims.create_handoff_batch(manager, COMPANY, "USD"),
        claims.create_handoff_batch(manager, COMPANY, "USD")
    );
    ensure(
        "concurrent hand off cannot double-claim approved rows",
        matches!(
            (one, two),
            (Ok(_), Err(ClaimsError::NothingToHandOff))
                | (Err(ClaimsError::NothingToHandOff), Ok(_))
        ),
    )?;

    // An old scope must not keep acting after grants or the account link change.
    grants.set_direct(admin, STAFF, SUBMIT, Some(false)).await?;
    refuses(
        "grant revoked on old scope",
        claims.submit_request(&staff, COMPANY, &low).await,
        ClaimsError::Unauthorized,
    )?;
    grants.set_direct(admin, STAFF, SUBMIT, Some(true)).await?;
    users
        .unlink(admin, STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    refuses(
        "account unlinked on old scope",
        claims.submit_request(&staff, COMPANY, &low).await,
        ClaimsError::NotLinked,
    )?;
    refuses(
        "unlinked scope cannot read former claims",
        claims.self_requests(&staff, COMPANY).await,
        ClaimsError::NotLinked,
    )?;
    // Relink a different account to prove the employee check independent of submitter.
    users.unlink(admin, 1).await.map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        admin,
        1,
        bilibi_party_ids::PersonId::new(staff_employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(staff_employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    let pending = saved(
        "old employee pending claims",
        claims.employee_requests(admin, COMPANY, employees.1).await,
    )?
    .into_iter()
    .find(|r| r.status == "submitted")
    .ok_or(Error::Internal)?;
    let current = workforce
        .employment(admin, COMPANY, employees.1)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    workforce
        .update_employment(
            admin,
            COMPANY,
            current.id,
            &EmploymentChange {
                status: Some("inactive".into()),
                ..EmploymentChange::at(current.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    refuses(
        "relinked account cannot approve linked inactive employee claim",
        claims
            .approve_request(manager, COMPANY, pending.id, &Approval::default())
            .await,
        ClaimsError::OwnClaim,
    )?;
    users.unlink(admin, 1).await.map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        admin,
        1,
        bilibi_party_ids::PersonId::new(1).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(1).map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    let current = workforce
        .employment(admin, COMPANY, employees.1)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    workforce
        .update_employment(
            admin,
            COMPANY,
            current.id,
            &EmploymentChange {
                status: Some("active".into()),
                ..EmploymentChange::at(current.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        admin,
        STAFF,
        bilibi_party_ids::PersonId::new(staff_employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(staff_employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;

    for sql in [
        "UPDATE people_claim_request_events SET reason='changed' WHERE request_id=$1",
        "DELETE FROM people_claim_request_events WHERE request_id=$1",
        "UPDATE people_claim_handoff_batches SET total_amount=1 WHERE id=(SELECT handoff_batch_id FROM people_claim_submissions WHERE id=$1)",
        "UPDATE people_claim_submissions SET handoff_batch_id=NULL WHERE id=$1",
    ] {
        ensure(
            "database preserves history and hand off",
            client.execute(sql, &[&first.id]).await.is_err(),
        )?;
    }
    let companies = services.get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?;
    companies
        .archive_company(admin, 2, Some("PC2 disposable archived-company refusal"))
        .await
        .map_err(|_| Error::Internal)?;
    refuses(
        "archived company rejects writes",
        claims.create_handoff_batch(manager, 2, "USD").await,
        ClaimsError::CompanyArchived,
    )?;
    Ok(())
}

walkthrough_step!(
    "claim_workflow",
    after = ["employee_workspace"],
    |ctx| async move {
        let client = ctx.db.get().await?;
        // PC2: requests, decisions, hand-off, transactional events and refusals.
        crate::claim_workflow::run(
            &ctx.services,
            &client,
            &ctx.admin,
            &ctx.manager,
            &ctx.viewer,
            (ctx.manager_employment.id, ctx.staff_employment.id),
        )
        .await?;

        Ok(())
    }
);
