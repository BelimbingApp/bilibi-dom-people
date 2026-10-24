//! PV3: real People approvals for the Finance FY1 composition walkthrough.
//! Mirrors Bilimbi's Payroll and Claims facade workflows; no Finance dependency.
use bilibi_affiliation::{AffiliationInput, AffiliationService, EffectiveDates, OrganisationId};
use bilibi_authz::grants::Grants;
use bilibi_database::pool;
use bilibi_module_registry::Services;
use bilibi_people_claims::{
    Approval, Claims, NewCategory, NewClaimType, NewPolicy, NewRequest, Reimbursement,
};
use bilibi_people_organisation::{NewAssignment, NewPosition, NewVersion, Organisation};
use bilibi_people_payroll::{NewClassification, NewContribution, NewItem, NewPeriod, Payroll};
use bilibi_people_reference_data::{NewEntry, ReferenceData};
use bilibi_people_workforce::{NewEmployment, Workforce};
use bilibi_person::{PersonInput, PersonKind, PersonService};
use bilibi_tenancy::{Authentication, Scope};
use jiff::civil::date;
use std::sync::Arc;

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> Outcome<()> {
    // The caller chooses the book currency. It is company data, not a default.
    let currency = std::env::args()
        .nth(1)
        .ok_or("pass the disposable Finance book's currency")?;
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_database::migrate(&db, &modules).await?;
    bilibi_walkthrough_fixture::install(&db).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let client = db.get().await?;
    let manager = auth.authenticate(&**client, 1, 1, "PV3".into()).await?;
    let admin = auth.authenticate(&**client, 1, 4, "PV3".into()).await?;
    drop(client);
    for key in grants.vocabulary().capabilities.keys() {
        if key.starts_with("people.") {
            grants.set_direct(&admin, 1, key, Some(true)).await?;
        }
    }
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    if std::env::args().nth(2).as_deref() == Some("directory") {
        let employees = workforce.employees(&admin, 1).await?.require_current()?;
        let supervisor = employees
            .iter()
            .find(|e| e.employee_number == "PV3-001")
            .ok_or("create the composition source records first")?
            .reference
            .stable_id
            .parse()?;
        companion(&services, &workforce, &admin, supervisor).await?;
        return Ok(());
    }
    services
        .get::<ReferenceData>(bilibi_people_reference_data::SERVICE)?
        .create_entry(
            &admin,
            1,
            &NewEntry::new("employment_type", "pv3_type", "Composition proof type"),
        )
        .await?;
    companion(&services, &workforce, &admin, employee.id).await?;
    let employee = workforce
        .create_employment(
            &admin,
            1,
            &NewEmployment::new(1, 1, "PV3-001", "pv3_type", "active"),
        )
        .await?;
    let organisation = services.get::<Organisation>(bilibi_people_organisation::SERVICE)?;
    let position = organisation
        .create_position(&manager, 1, &NewPosition::new("PV3-position"))
        .await?;
    organisation
        .record_version(
            &manager,
            1,
            position.id,
            &NewVersion {
                version: 1,
                title: "Composition position".into(),
                effective_from: date(2026, 1, 1),
                effective_to: None,
            },
        )
        .await?;
    organisation
        .assign(
            &manager,
            1,
            position.id,
            &NewAssignment {
                employee_id: employee.id,
                kind: "substantive".into(),
                effective_from: date(2026, 1, 1),
                effective_to: None,
            },
        )
        .await?;
    let payroll = services.get::<Payroll>(bilibi_people_payroll::SERVICE)?;
    payroll
        .put_settings(
            &manager,
            1,
            "composition proof",
            std::slice::from_ref(&currency),
        )
        .await?;
    let classification = payroll
        .create_classification(
            &manager,
            1,
            &NewClassification {
                code: "pv3_class".into(),
                name: "Composition classification".into(),
                effective_from: date(2026, 1, 1),
                effective_to: None,
            },
        )
        .await?;
    let item = payroll
        .create_item(
            &manager,
            1,
            &NewItem {
                code: "pv3_item".into(),
                name: "Composition earning".into(),
                classification_id: classification.id,
                currency: currency.clone(),
                amount: "12.345678".parse()?,
                effective_from: date(2026, 1, 1),
                effective_to: None,
            },
        )
        .await?;
    let period = payroll
        .create_period(
            &manager,
            1,
            &NewPeriod {
                code: "FY1-run".into(),
                starts_on: date(2026, 1, 1),
                ends_on: date(2026, 1, 31),
                pay_on: date(2026, 1, 31),
            },
        )
        .await?;
    let run = payroll
        .freeze_setup(&manager, 1, period.id, &currency)
        .await?;
    payroll
        .intake(
            &manager,
            1,
            run.id,
            &NewContribution {
                employee_id: employee.id,
                item_id: item.id,
                source_key: "pv3:earning".into(),
                evidence: "walkthrough/pv3".into(),
                on_date: date(2026, 1, 15),
                units: "1.234567".parse()?,
                direction: "earning".into(),
            },
        )
        .await?;
    payroll.lock_run(&manager, 1, run.id).await?;
    payroll.calculate(&manager, 1, run.id).await?;
    payroll
        .approve_run(&admin, 1, run.id, "Independently checked for composition")
        .await?;

    let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
    claims
        .put_currencies(&manager, 1, std::slice::from_ref(&currency))
        .await?;
    let category = claims
        .create_category(
            &manager,
            1,
            &NewCategory {
                code: "pv3_category".into(),
                name: "Composition category".into(),
            },
        )
        .await?;
    let kind = claims
        .create_claim_type(
            &manager,
            1,
            &NewClaimType {
                category_id: category.id,
                code: "pv3_claim".into(),
                name: "Composition claim".into(),
                receipt_requirement: "never".into(),
                eligibility: None,
            },
        )
        .await?;
    claims
        .create_policy(
            &manager,
            1,
            &NewPolicy {
                claim_type_id: kind.id,
                effective_from: date(2026, 1, 1),
                effective_to: None,
                currency: currency.clone(),
                per_claim_limit: None,
                monthly_limit: None,
                yearly_limit: None,
                receipt_threshold: None,
            },
        )
        .await?;
    for day in 1..=3 {
        let request = claims
            .submit_request(
                &manager,
                1,
                &NewRequest {
                    claim_type_id: kind.id,
                    incurred_on: date(2026, 1, day),
                    amount: "80.00".parse()?,
                    currency: currency.clone(),
                    description: Some(format!("Composition expense {day}")),
                    receipt_number: None,
                    confirm_duplicate: false,
                },
            )
            .await?;
        claims
            .approve_request(&admin, 1, request.id, &Approval::default())
            .await?;
        if day == 3 {
            claims
                .reimburse_request(
                    &admin,
                    1,
                    request.id,
                    &Reimbursement {
                        payment_reference: Some("PV3-payment".into()),
                    },
                )
                .await?;
        }
    }
    println!("PV3: employment, exact payroll approval, three claim approvals and reimbursement committed through People facades");
    Ok(())
}

async fn companion(
    services: &Services,
    workforce: &Workforce,
    admin: &Scope,
    supervisor: i64,
) -> Outcome<()> {
    let employees = workforce.employees(admin, 1).await?.require_current()?;
    if employees.iter().any(|e| e.employee_number == "PV3-002") {
        return Ok(());
    }
    let person = services
        .get::<PersonService>(bilibi_person::SERVICE)?
        .create(
            admin,
            &PersonInput {
                name: "Composition employee".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await?;
    let affiliation = services
        .get::<AffiliationService>(bilibi_affiliation::SERVICE)?
        .create(
            admin,
            &AffiliationInput {
                person_id: person.id,
                organisation_id: OrganisationId::new(1)?,
                title: None,
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None)?,
            },
        )
        .await?;
    workforce
        .create_employment(
            admin,
            1,
            &NewEmployment {
                supervisor_employment_id: Some(supervisor),
                ..NewEmployment::new(
                    person.id.get(),
                    affiliation.id.get(),
                    "PV3-002",
                    "pv3_type",
                    "active",
                )
            },
        )
        .await?;
    println!("PV3: second working employee and supervisor link created through Core and Workforce");
    Ok(())
}
