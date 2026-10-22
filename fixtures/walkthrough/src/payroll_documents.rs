//! Bilimbi Payroll document and retention proof through the real mounted services.
use super::{ADMIN, COMPANY, MANAGER, OTHER_COMPANY, OTHER_TENANT_COMPANY, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, MutationBoundary, Result};
use bilibi_module_registry::Services;
use bilibi_people_payroll::{
    NewClassification, NewContribution, NewItem, NewPeriod, Payroll, PayrollError as Refusal,
    MANAGE, VIEW,
};
use bilibi_people_workforce::Workforce;
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::download::{DownloadRequest, DownloadService};
use jiff::civil::date;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{collections::HashMap, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

fn check(label: &str, yes: bool) -> Result<()> {
    if !yes {
        eprintln!("{label}: FAILED");
        return Err(Error::Internal);
    }
    println!("{label}: proved");
    Ok(())
}
fn must<T>(value: bilibi_people_payroll::Result<T>) -> Result<T> {
    value.map_err(|error| {
        eprintln!("Payroll: {error:?}");
        Error::Internal
    })
}
fn denied<T>(
    label: &str,
    value: bilibi_people_payroll::Result<T>,
    expected: fn(&Refusal) -> bool,
) -> Result<()> {
    match value {
        Err(e) if expected(&e) => {
            println!("{label}: refused ({e})");
            Ok(())
        }
        Err(e) => {
            eprintln!("{label}: unexpected {e:?}");
            Err(Error::Internal)
        }
        Ok(_) => {
            eprintln!("{label}: unexpectedly accepted");
            Err(Error::Internal)
        }
    }
}
async fn setting(services: &Services, admin: &Scope, key: &str, value: Value) -> Result<()> {
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    let mut c = db.get().await?;
    let boundary = services.get::<MutationBoundary>("base/database.mutations")?;
    let mut tx = boundary
        .begin(
            &mut c,
            admin.mutation_context("walkthrough.payroll.documents.settings"),
        )
        .await?;
    services
        .get::<Settings>("base/settings.store")?
        .put(&mut tx, admin, Target::Global, key, value)
        .await
        .map_err(|error| {
            eprintln!("Setting {key}: {error}");
            error
        })?;
    tx.commit().await
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|_| Error::Internal)
}
pub async fn run(url: &str) -> Result<()> {
    let out = std::env::var("BILIBI_PAYROLL_PROOF_OUT").map_err(|_| Error::Invalid)?;
    let out = Path::new(&out);
    std::fs::create_dir_all(out).map_err(|_| Error::Internal)?;
    let root = out.join("private-artifacts");
    std::fs::create_dir_all(&root).map_err(|_| Error::Internal)?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::Internal)?;
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&bilibi_module_registry::discover()?)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let c = db.get().await?;
    let admin = auth
        .authenticate(&**c, TENANT, ADMIN, "PY3 proof".into())
        .await?;
    let manager = auth
        .authenticate(&**c, TENANT, MANAGER, "PY3 proof".into())
        .await?;
    let viewer = auth
        .authenticate(&**c, TENANT, VIEWER, "PY3 proof".into())
        .await?;
    let payroll = services.get::<Payroll>(bilibi_people_payroll::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let employees = workforce
        .employees(&manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let ids: Vec<i64> = employees
        .iter()
        .take(2)
        .map(|e| e.reference.stable_id.parse().map_err(|_| Error::Internal))
        .collect::<Result<_>>()?;
    check("two employment IDs available", ids.len() == 2)?;
    let from = date(2090, 1, 1);
    must(
        payroll
            .put_settings(&manager, COMPANY, "proof-jurisdiction", &["XAA".into()])
            .await,
    )?;
    let class = must(
        payroll
            .create_classification(
                &manager,
                COMPANY,
                &NewClassification {
                    code: "py3-class".into(),
                    name: "Document proof".into(),
                    effective_from: from,
                    effective_to: None,
                },
            )
            .await,
    )?;
    let item = must(
        payroll
            .create_item(
                &manager,
                COMPANY,
                &NewItem {
                    code: "py3-item".into(),
                    name: "Document proof".into(),
                    classification_id: class.id,
                    currency: "XAA".into(),
                    amount: Decimal::from_str_exact("12.345678").map_err(|_| Error::Invalid)?,
                    effective_from: from,
                    effective_to: None,
                },
            )
            .await,
    )?;
    let period = must(
        payroll
            .create_period(
                &manager,
                COMPANY,
                &NewPeriod {
                    code: "py3-period".into(),
                    starts_on: from,
                    ends_on: from,
                    pay_on: from,
                },
            )
            .await,
    )?;
    let run = must(
        payroll
            .freeze_setup(&manager, COMPANY, period.id, "XAA")
            .await,
    )?;
    denied(
        "unapproved document",
        payroll
            .generate_document(&manager, COMPANY, run.id, "report", None)
            .await,
        |e| matches!(e, Refusal::NotFound),
    )?;
    for i in 0..70 {
        must(
            payroll
                .intake(
                    &manager,
                    COMPANY,
                    run.id,
                    &NewContribution {
                        employee_id: ids[usize::from(i == 69)],
                        item_id: item.id,
                        source_key: format!("py3-{i:03}"),
                        evidence: "attestation:document-proof".into(),
                        on_date: from,
                        units: Decimal::from_str_exact("1.234567").map_err(|_| Error::Invalid)?,
                        direction: if i % 3 == 0 {
                            "deduction"
                        } else if i % 3 == 1 {
                            "earning"
                        } else {
                            "employer"
                        }
                        .into(),
                    },
                )
                .await,
        )?;
    }
    must(payroll.lock_run(&manager, COMPANY, run.id).await)?;
    must(payroll.calculate(&manager, COMPANY, run.id).await)?;
    denied(
        "calculated but undecided document",
        payroll
            .generate_document(&manager, COMPANY, run.id, "report", None)
            .await,
        |e| matches!(e, Refusal::NotFound),
    )?;
    must(
        payroll
            .approve_run(&admin, COMPANY, run.id, "Frozen result checked")
            .await,
    )?;
    let next = from.tomorrow().map_err(|_| Error::Invalid)?;
    let rejected_period = must(
        payroll
            .create_period(
                &manager,
                COMPANY,
                &NewPeriod {
                    code: "py3-rejected".into(),
                    starts_on: next,
                    ends_on: next,
                    pay_on: next,
                },
            )
            .await,
    )?;
    let rejected = must(
        payroll
            .freeze_setup(&manager, COMPANY, rejected_period.id, "XAA")
            .await,
    )?;
    must(
        payroll
            .intake(
                &manager,
                COMPANY,
                rejected.id,
                &NewContribution {
                    employee_id: ids[0],
                    item_id: item.id,
                    source_key: "py3-rejected".into(),
                    evidence: "attestation:rejected-proof".into(),
                    on_date: next,
                    units: Decimal::ONE,
                    direction: "earning".into(),
                },
            )
            .await,
    )?;
    must(payroll.lock_run(&manager, COMPANY, rejected.id).await)?;
    must(payroll.calculate(&manager, COMPANY, rejected.id).await)?;
    must(
        payroll
            .reject_run(&admin, COMPANY, rejected.id, "Not approved")
            .await,
    )?;
    denied(
        "rejected document",
        payroll
            .generate_document(&manager, COMPANY, rejected.id, "report", None)
            .await,
        |e| matches!(e, Refusal::NotFound),
    )?;
    for (kind, employee) in [
        ("other", None),
        ("report", Some(ids[0])),
        ("payslip", None),
        ("payslip", Some(i64::MAX)),
    ] {
        denied(
            "invalid kind or employee",
            payroll
                .generate_document(&manager, COMPANY, run.id, kind, employee)
                .await,
            |e| matches!(e, Refusal::NotFound | Refusal::InvalidDocument),
        )?;
    }
    denied(
        "viewer cannot generate",
        payroll
            .generate_document(&viewer, COMPANY, run.id, "report", None)
            .await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    setting(&services, &admin, "artifacts.storage_root", json!("")).await?;
    denied(
        "missing storage policy",
        payroll
            .generate_document(&manager, COMPANY, run.id, "report", None)
            .await,
        |e| {
            matches!(e, Refusal::ArtifactPolicy)
                || matches!(e, Refusal::ArtifactFailed(error) if error.code() == "unsafe_root")
        },
    )?;
    setting(&services, &admin, "artifacts.storage_root", json!(root)).await?;
    setting(&services, &admin, "artifacts.retention_days", Value::Null).await?;
    denied(
        "missing retention policy",
        payroll
            .generate_document(&manager, COMPANY, run.id, "report", None)
            .await,
        |e| matches!(e, Refusal::ArtifactPolicy),
    )?;
    setting(&services, &admin, "artifacts.retention_days", json!(30)).await?;
    check("refusals publish no artifact", c.query_one("SELECT count(*) FROM base_artifacts WHERE tenant_id=$1 AND company_id=$2 AND owner_id='people/payroll'", &[&TENANT, &COMPANY]).await?.get::<_, i64>(0) == 0)?;
    let report = must(
        payroll
            .generate_document(&manager, COMPANY, run.id, "report", None)
            .await,
    )?;
    // Live company settings must never alter an approved document.
    must(
        payroll
            .put_settings(
                &manager,
                COMPANY,
                "changed-after-approval",
                &["XAA".into(), "XBB".into()],
            )
            .await,
    )?;
    let repeat = must(
        payroll
            .generate_document(&manager, COMPANY, run.id, "report", None)
            .await,
    )?;
    let slip = must(
        payroll
            .generate_document(&manager, COMPANY, run.id, "payslip", Some(ids[0]))
            .await,
    )?;
    let bytes = must(
        payroll
            .read_document(&manager, COMPANY, &report.artifact_id)
            .await,
    )?
    .bytes;
    let repeated = must(
        payroll
            .read_document(&manager, COMPANY, &repeat.artifact_id)
            .await,
    )?
    .bytes;
    check(
        "repeated generation yields identical bytes",
        bytes == repeated && report.artifact_id != repeat.artifact_id,
    )?;
    let slip_bytes = must(
        payroll
            .read_document(&manager, COMPANY, &slip.artifact_id)
            .await,
    )?
    .bytes;
    write(&out.join("report.pdf"), &bytes)?;
    write(&out.join("payslip.pdf"), &slip_bytes)?;
    let output = must(payroll.run_output(&manager, COMPANY, run.id).await)?;
    write(&out.join("frozen.json"), &serde_json::to_vec(&json!({"run_id": run.id,"employee_id": ids[0],"currency":run.currency,"calculation":output.calculation})).map_err(|_| Error::Internal)?)?;
    check(
        "provenance contains only approved documents",
        output.documents.len() == 3,
    )?;
    for company in [OTHER_COMPANY, OTHER_TENANT_COMPANY] {
        denied(
            "foreign company cannot download",
            payroll
                .read_document(&manager, company, &report.artifact_id)
                .await,
            |e| {
                matches!(
                    e,
                    Refusal::NotFound | Refusal::Unauthorized | Refusal::CompanyArchived
                )
            },
        )?;
    }
    denied(
        "unknown artifact",
        payroll.read_document(&manager, COMPANY, "invalid").await,
        |e| matches!(e, Refusal::NotFound),
    )?;
    denied(
        "viewer cannot download",
        payroll
            .read_document(&viewer, COMPANY, &report.artifact_id)
            .await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    let impersonated = auth
        .authenticate_session(&**c, TENANT, MANAGER, Some(ADMIN), "PY3 proof".into())
        .await?;
    denied(
        "impersonated download",
        payroll
            .read_document(&impersonated, COMPANY, &report.artifact_id)
            .await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    let download = services.get::<DownloadService<Scope, Error>>("people/payroll.downloads")?;
    let path = HashMap::from([("id".into(), report.artifact_id.clone())]);
    let query = HashMap::from([("company_id".into(), COMPANY.to_string())]);
    let response = download
        .0
        .serve(DownloadRequest {
            operation: "document",
            scope: &manager,
            path: &path,
            query: &query,
        })
        .await?;
    check(
        "download re-reads identical bytes with private headers",
        response.bytes == bytes
            && response
                .headers()
                .iter()
                .any(|(k, v)| *k == "cache-control" && v == "private, no-store")
            && response
                .headers()
                .iter()
                .any(|(k, v)| *k == "x-content-type-options" && v == "nosniff"),
    )?;
    check(
        "download needs explicit company",
        download
            .0
            .serve(DownloadRequest {
                operation: "document",
                scope: &manager,
                path: &path,
                query: &HashMap::new(),
            })
            .await
            .is_err(),
    )?;
    grants
        .set_direct(&admin, MANAGER, VIEW, Some(false))
        .await?;
    denied(
        "revoked view refuses same sealed scope",
        payroll
            .read_document(&manager, COMPANY, &report.artifact_id)
            .await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    check(
        "revoked view refuses same download link",
        download
            .0
            .serve(DownloadRequest {
                operation: "document",
                scope: &manager,
                path: &path,
                query: &query,
            })
            .await
            .is_err(),
    )?;
    grants.set_direct(&admin, MANAGER, VIEW, Some(true)).await?;
    grants
        .set_direct(&admin, MANAGER, MANAGE, Some(false))
        .await?;
    denied(
        "revoked manage refuses generation",
        payroll
            .generate_document(&manager, COMPANY, run.id, "report", None)
            .await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    denied(
        "revoked manage refuses purge",
        payroll.purge(&manager, COMPANY).await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    grants
        .set_direct(&admin, MANAGER, MANAGE, Some(true))
        .await?;
    // Fixture-only clock advancement, with every production guard still enabled.
    c.execute("UPDATE base_artifacts SET expires_at=created_at+interval '1 millisecond' WHERE tenant_id=$1 AND company_id=$2 AND public_id=$3", &[&TENANT, &COMPANY, &repeat.artifact_id]).await?;
    denied(
        "expired document cannot download",
        payroll
            .read_document(&manager, COMPANY, &repeat.artifact_id)
            .await,
        |e| matches!(e, Refusal::NotFound),
    )?;
    setting(&services, &admin, "artifacts.purge_max_attempts", json!(1)).await?;
    let moved = out.join("temporarily-unavailable");
    std::fs::rename(&root, &moved).map_err(|_| Error::Internal)?;
    let failed = payroll.purge(&manager, COMPANY).await;
    std::fs::rename(&moved, &root).map_err(|_| Error::Internal)?;
    let failed = must(failed)?;
    check(
        "failed cleanup remains tracked",
        failed
            .failed
            .iter()
            .any(|(id, _)| id == &repeat.artifact_id),
    )?;
    let holds = must(payroll.purge_holds(&manager, COMPANY).await)?;
    check(
        "failed cleanup reaches operator hold",
        holds
            .iter()
            .any(|h| h.metadata.id == repeat.artifact_id && h.attempts == 1),
    )?;
    must(
        payroll
            .retry_purge(&manager, COMPANY, &repeat.artifact_id)
            .await,
    )?;
    check(
        "retry clears hold",
        must(payroll.purge_holds(&manager, COMPANY).await)?.is_empty(),
    )?;
    check(
        "purge keeps immutable provenance",
        must(payroll.run_output(&manager, COMPANY, run.id).await)?
            .documents
            .len()
            == 3,
    )?;
    check(
        "unexpired bytes survive purge",
        must(
            payroll
                .read_document(&manager, COMPANY, &report.artifact_id)
                .await,
        )?
        .bytes
            == bytes,
    )?;
    println!(
        "Browser download: /people/payroll/documents/{}?company_id={COMPANY}",
        report.artifact_id
    );
    Ok(())
}
