//! PM2 executable proof: real page mounts and live events over disposable data.
use bilibi_authz::grants::Grants;
use bilibi_database::{Db, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_performance::{self as pm, Performance};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveBody, LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use serde_json::{json, Value};
use std::collections::HashMap;

type Mounted = Box<dyn LiveState<Scope, Error>>;
fn require(ok: bool, message: &str) -> Result<()> {
    if !ok {
        eprintln!("PM2 FAILED: {message}");
        return Err(Error::Internal);
    }
    println!("PM2 proved: {message}");
    Ok(())
}
fn saved<T>(value: pm::Result<T>) -> Result<T> {
    value.map_err(|error| {
        eprintln!("PM2 unexpected refusal: {error}");
        Error::Internal
    })
}
async fn mount(
    page: &PageService<Scope, Error>,
    scope: &Scope,
    operation: &'static str,
    query: &HashMap<String, String>,
) -> Result<Mounted> {
    let empty = HashMap::new();
    page.0
        .mount(PageRequest {
            operation,
            scope: Some(scope),
            path: &empty,
            query,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)
}
async fn event(state: &mut Mounted, scope: &Scope, payload: Value) -> Result<LiveBody> {
    state
        .update(LiveRequest {
            scope,
            credential: "walkthrough",
            input: LiveInput::Event(&payload),
        })
        .await
}
async fn refresh(state: &mut Mounted, scope: &Scope) -> Result<LiveBody> {
    state
        .update(LiveRequest {
            scope,
            credential: "walkthrough",
            input: LiveInput::Refresh,
        })
        .await
}
async fn confirm(state: &mut Mounted, scope: &Scope) -> Result<LiveBody> {
    // Forged record/company values must never replace the server's selection.
    event(state,scope,json!({"type":"confirm_publication","value":"99999999","company_id":"100","record[target]":"tampered"})).await
}
fn form(event_name: &str, pairs: &[(&str, String)]) -> Value {
    let mut value = json!({"type":event_name});
    for (key, text) in pairs {
        value[format!("record[{key}]")] = json!(text);
    }
    value
}

pub async fn run(url: &str) -> Result<()> {
    super::performance::run_with_pages(url, true).await
}
pub(super) async fn verify(services: &Services) -> Result<()> {
    let db = services.get::<Db>("base/database.pool")?;
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let manager = auth
        .authenticate(&**client, 1, 1, "walkthrough".into())
        .await?;
    let admin = auth
        .authenticate(&**client, 1, 4, "walkthrough".into())
        .await?;
    let staff = auth
        .authenticate(&**client, 1, 3, "walkthrough".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let performance = services.get::<Performance>(pm::SERVICE)?;
    let page = services.get::<PageService<Scope, Error>>("people/performance.pages")?;
    let query = HashMap::new();
    for (tab, tables) in [
        (
            "reviews",
            vec!["performance-reviews", "performance-release-queue"],
        ),
        ("descriptions", vec!["performance-descriptions"]),
        (
            "kpis",
            vec!["performance-definitions", "performance-targets"],
        ),
        ("evidence", vec!["performance-observations"]),
    ] {
        let mut state = mount(
            &page,
            &manager,
            "reviews",
            &HashMap::from([("tab".into(), tab.into())]),
        )
        .await?;
        let body = refresh(&mut state, &manager).await?;
        require(
            tables.iter().all(|id| body.html.contains(id))
                && !body.html.contains("Performance is unavailable"),
            &format!("{tab} task renders its FlexTables"),
        )?;
    }
    let mut state = mount(&page, &manager, "reviews", &query).await?;
    let body = event(
        &mut state,
        &manager,
        json!({"type":"filter_0","filters[perPage]":"10"}),
    )
    .await?;
    require(
        body.query.get("page_size").is_some_and(|s| s == "10"),
        "URL page-size state",
    )?;
    let body = event(
        &mut state,
        &manager,
        json!({"type":"page_0","page":"9999999999999999"}),
    )
    .await?;
    require(
        body.query.get("page").is_some_and(|s| s == "1"),
        "oversized review page is clamped",
    )?;

    let definition = form(
        "create_definition",
        &[
            ("code", "pm2_ui_measure".into()),
            ("version", "1".into()),
            ("name", "PM2 page measure".into()),
            ("purpose", "Page proof".into()),
            ("unit", "Fixture unit".into()),
            ("measure", "Fixture measure".into()),
            ("source_reference", "fixture:pm2".into()),
            ("calculation_version", "1".into()),
            ("direction", "higher".into()),
            ("precision", "2".into()),
            ("interpretation", "Fixture interpretation".into()),
        ],
    );
    let body = event(&mut state, &manager, definition.clone()).await?;
    require(
        body.html.contains("performance-publication")
            && !saved(performance.planning_records(&manager, 1).await)?
                .definitions
                .iter()
                .any(|r| r.code == "pm2_ui_measure"),
        "draft waits for confirmation",
    )?;
    event(&mut state, &manager, json!({"type":"cancel_publication"})).await?;
    confirm(&mut state, &manager).await?;
    require(
        !saved(performance.planning_records(&manager, 1).await)?
            .definitions
            .iter()
            .any(|r| r.code == "pm2_ui_measure"),
        "cancelled confirmation cannot write",
    )?;
    event(&mut state, &manager, definition.clone()).await?;
    grants
        .set_direct(&admin, 1, pm::KPIS_SUBMIT, Some(false))
        .await?;
    let body = confirm(&mut state, &manager).await?;
    require(
        body.html.contains("You cannot do that for this company."),
        "grant revoked after confirmation refuses the next event",
    )?;
    grants
        .set_direct(&admin, 1, pm::KPIS_SUBMIT, Some(true))
        .await?;
    event(&mut state, &manager, definition).await?;
    let body = confirm(&mut state, &manager).await?;
    require(
        body.html.contains("Record saved."),
        "confirmed measurement writes successfully",
    )?;
    confirm(&mut state, &manager).await?;
    let planning = saved(performance.planning_records(&manager, 1).await)?;
    require(
        planning
            .definitions
            .iter()
            .filter(|r| r.code == "pm2_ui_measure")
            .count()
            == 1,
        "duplicate confirmation cannot replay a write",
    )?;
    let definition = planning
        .definitions
        .iter()
        .find(|r| r.code == "pm2_ui_measure")
        .ok_or(Error::Internal)?;
    let choices = saved(performance.planning_choices(&manager, 1).await)?;
    let employee = choices.employees.first().ok_or(Error::Internal)?.id;
    let description = choices.descriptions.first().ok_or(Error::Internal)?.id;
    event(
        &mut state,
        &manager,
        form(
            "create_target",
            &[
                ("definition_id", definition.id.to_string()),
                ("employee_id", employee.to_string()),
                ("target", "PM2 page target".into()),
                ("period_start", "2020-01-01".into()),
                ("period_end", "2020-01-31".into()),
                ("effective_from", "2020-01-01".into()),
            ],
        ),
    )
    .await?;
    let body = confirm(&mut state, &manager).await?;
    require(
        body.html.contains("Record saved."),
        "target proposal through page",
    )?;
    let targets = saved(performance.planning_records(&manager, 1).await)?.targets;
    let target = targets
        .iter()
        .find(|r| r.target == "PM2 page target")
        .ok_or(Error::Internal)?
        .id;
    let mut approver = mount(&page, &admin, "reviews", &query).await?;
    let mut review = form(
        "review_target",
        &[("note", "PM2 independent rationale".into())],
    );
    review["record_id"] = json!(target.to_string());
    event(&mut approver, &admin, review).await?;
    require(
        confirm(&mut approver, &admin)
            .await?
            .html
            .contains("Record saved."),
        "independent target review through page",
    )?;
    event(
        &mut approver,
        &admin,
        json!({"type":"request_publish_target","value":target.to_string()}),
    )
    .await?;
    require(
        confirm(&mut approver, &admin)
            .await?
            .html
            .contains("Record saved."),
        "communicated target keeps the server-held selection",
    )?;

    let long_evidence = format!("PM2 evidence {}", "Attributable observation. ".repeat(100));
    event(
        &mut state,
        &manager,
        form(
            "create_observation",
            &[
                ("employee_id", employee.to_string()),
                ("window_start", "2020-01-01".into()),
                ("window_end", "2020-01-31".into()),
                ("evidence", long_evidence.clone()),
                ("source_reference", "fixture:pm2:evidence".into()),
                ("source_version", "1".into()),
            ],
        ),
    )
    .await?;
    require(
        confirm(&mut state, &manager)
            .await?
            .html
            .contains("Record saved."),
        "evidence longer than 1 KiB is retained",
    )?;
    let observation = saved(performance.planning_records(&manager, 1).await)?
        .observations
        .into_iter()
        .find(|r| r.evidence == long_evidence)
        .ok_or(Error::Internal)?;
    let mut review = form(
        "create_review",
        &[
            ("employee_id", employee.to_string()),
            ("description_id", description.to_string()),
            ("period_start", "2020-01-01".into()),
            ("period_end", "2020-01-31".into()),
            ("cutoff_at", jiff::Timestamp::now().to_string()),
            ("outcome", "PM2 page outcome".into()),
            ("rationale", "PM2 pinned evidence".into()),
        ],
    );
    review["record[observation_ids][]"] = json!([observation.id.to_string()]);
    review["record[target_ids][]"] = json!([target.to_string()]);
    event(&mut state, &manager, review).await?;
    require(
        confirm(&mut state, &manager)
            .await?
            .html
            .contains("Record saved."),
        "draft review decodes multiple evidence and target selections",
    )?;
    let reviews = saved(
        performance
            .reviews(&manager, 1, pm::PageOptions::default())
            .await,
    )?;
    let review = reviews
        .rows
        .iter()
        .find(|r| r.outcome == "PM2 page outcome")
        .ok_or(Error::Internal)?
        .id;
    event(
        &mut state,
        &manager,
        json!({"type":"request_release","value":review.to_string()}),
    )
    .await?;
    require(
        confirm(&mut state, &manager)
            .await?
            .html
            .contains("The author and the employee cannot approve their own record."),
        "author release refused without a fatal live error",
    )?;
    event(
        &mut approver,
        &admin,
        json!({"type":"request_release","value":review.to_string()}),
    )
    .await?;
    require(
        confirm(&mut approver, &admin)
            .await?
            .html
            .contains("Record saved."),
        "independent review release through page",
    )?;
    let mut own = mount(&page, &staff, "my", &query).await?;
    let body = refresh(&mut own, &staff).await?;
    require(
        body.html.contains("PM2 page target")
            && body.html.contains("PM2 page outcome")
            && body.html.contains("Your response or dispute"),
        "employee sees communicated target, released review and response form",
    )?;
    let mut response = form(
        "save_response",
        &[("response", "PM2 response from page".into())],
    );
    response["record_id"] = json!(review.to_string());
    require(
        event(&mut own, &staff, response)
            .await?
            .html
            .contains("performance-publication")
            && confirm(&mut own, &staff)
                .await?
                .html
                .contains("Response recorded."),
        "employee response through page",
    )?;
    let detail = saved(performance.review(&manager, 1, review).await)?;
    require(
        detail.responses.len() == 1 && detail.responses[0].response == "PM2 response from page",
        "response retained on the released version",
    )?;
    let mut kpis = mount(
        &page,
        &manager,
        "reviews",
        &HashMap::from([("tab".into(), "kpis".into())]),
    )
    .await?;
    event(&mut kpis, &manager, form("create_definition", &[])).await?;
    require(
        confirm(&mut kpis, &manager)
            .await?
            .html
            .contains("performance-form-errors"),
        "refused form opens its shared error summary",
    )?;

    let mut descriptions = mount(
        &page,
        &manager,
        "reviews",
        &HashMap::from([("tab".into(), "descriptions".into())]),
    )
    .await?;
    let position = choices.positions.first().ok_or(Error::Internal)?;
    let profile = choices.profiles.first().ok_or(Error::Internal)?;
    event(
        &mut descriptions,
        &manager,
        form(
            "create_description",
            &[
                ("code", "pm2_ui_description".into()),
                ("version", "1".into()),
                ("position_id", position.id.to_string()),
                (
                    "position_version",
                    position.version.ok_or(Error::Internal)?.to_string(),
                ),
                ("profile_id", profile.id.to_string()),
                ("effective_from", "2020-01-01".into()),
                ("purpose", "PM2 purpose".into()),
                ("responsibilities", "PM2 responsibilities".into()),
                ("duties", "PM2 duties".into()),
                ("authority", "PM2 authority".into()),
                ("qualifications", "PM2 qualifications".into()),
            ],
        ),
    )
    .await?;
    require(
        confirm(&mut descriptions, &manager)
            .await?
            .html
            .contains("Record saved."),
        "description draft pins the selected profile version",
    )?;
    let draft_description = saved(performance.planning_records(&manager, 1).await)?
        .descriptions
        .into_iter()
        .find(|r| r.code == "pm2_ui_description")
        .ok_or(Error::Internal)?;
    event(
        &mut descriptions,
        &manager,
        json!({"type":"request_publish_description", "value":draft_description.id.to_string()}),
    )
    .await?;
    require(
        confirm(&mut descriptions, &manager)
            .await?
            .html
            .contains("A published description already covers these dates."),
        "overlapping publication refuses with its live-page sentence",
    )?;

    event(
        &mut kpis,
        &manager,
        form(
            "create_target",
            &[
                ("supersedes_id", target.to_string()),
                ("target", "PM2 amended target".into()),
                ("effective_from", "2020-01-02".into()),
                ("change_reason", "PM2 target correction".into()),
            ],
        ),
    )
    .await?;
    require(
        confirm(&mut kpis, &manager)
            .await?
            .html
            .contains("Record saved."),
        "target amendment starts a new proposal cycle through the page",
    )?;
    let mut evidence = mount(
        &page,
        &manager,
        "reviews",
        &HashMap::from([("tab".into(), "evidence".into())]),
    )
    .await?;
    event(
        &mut evidence,
        &manager,
        form(
            "create_observation",
            &[
                ("supersedes_id", observation.id.to_string()),
                ("evidence", "PM2 corrected page evidence".into()),
                ("source_version", "2".into()),
                ("change_reason", "PM2 evidence correction".into()),
            ],
        ),
    )
    .await?;
    require(
        confirm(&mut evidence, &manager)
            .await?
            .html
            .contains("Record saved."),
        "evidence correction preserves its original subject and window",
    )?;
    let corrected_observation = saved(performance.planning_records(&manager, 1).await)?
        .observations
        .into_iter()
        .find(|r| r.supersedes_id == Some(observation.id))
        .ok_or(Error::Internal)?;
    let mut correction = form(
        "create_review",
        &[
            ("supersedes_id", review.to_string()),
            ("description_id", description.to_string()),
            ("cutoff_at", jiff::Timestamp::now().to_string()),
            ("outcome", "PM2 corrected page outcome".into()),
            ("rationale", "PM2 corrected page rationale".into()),
            ("change_reason", "PM2 review correction".into()),
        ],
    );
    correction["record[observation_ids][]"] = json!([corrected_observation.id.to_string()]);
    correction["record[target_ids][]"] = json!([target.to_string()]);
    event(&mut state, &manager, correction).await?;
    require(
        confirm(&mut state, &manager)
            .await?
            .html
            .contains("Record saved."),
        "review correction retains the old response and pins new evidence",
    )?;
    let corrected_review = saved(
        performance
            .reviews(&manager, 1, pm::PageOptions::default())
            .await,
    )?
    .rows
    .into_iter()
    .find(|r| r.supersedes_id == Some(review))
    .ok_or(Error::Internal)?;
    event(
        &mut approver,
        &admin,
        json!({"type":"request_release", "value":corrected_review.id.to_string()}),
    )
    .await?;
    require(
        confirm(&mut approver, &admin)
            .await?
            .html
            .contains("Record saved."),
        "corrected review requires independent release again",
    )?;
    let original = saved(performance.review(&manager, 1, review).await)?;
    require(
        original.review.rationale == "PM2 pinned evidence"
            && original.responses.len() == 1
            && original.observations[0].evidence == long_evidence,
        "original review evidence and response remain immutable",
    )?;

    let users = services.get::<bilibi_user::UserService>("core/user.accounts")?;
    let affiliation = staff.actor().affiliation().ok_or(Error::Internal)?;
    refresh(&mut own, &staff).await?;
    users.unlink(&admin, 3).await.map_err(|_| Error::Internal)?;
    let mut unlinked_response = form(
        "save_response",
        &[("response", "Former link must refuse".into())],
    );
    unlinked_response["record_id"] = json!(corrected_review.id.to_string());
    event(&mut own, &staff, unlinked_response).await?;
    users.unlink(&admin, 3).await.map_err(|_| Error::Internal)?;
    let body = confirm(&mut own, &staff).await?;
    require(
        body.html
            .contains("Your performance records are unavailable")
            && saved(performance.review(&manager, 1, corrected_review.id).await)?
                .responses
                .is_empty(),
        "an unlinked account's mounted response form refuses the old employee",
    )?;
    users
        .link_affiliation(&admin, 3, affiliation)
        .await
        .map_err(|_| Error::Internal)?;

    // Keep a view-only account available for the browser proof as well.
    grants.set_direct(&admin, 2, pm::VIEW, Some(true)).await?;
    let viewer = auth
        .authenticate(&**client, 1, 2, "walkthrough".into())
        .await?;
    let mut readonly = mount(&page, &viewer, "reviews", &query).await?;
    let body = refresh(&mut readonly, &viewer).await?;
    require(
        body.html.contains("Review drafting is read-only") && !body.html.contains("create_review"),
        "view-only reader sees the reference's withheld controls",
    )?;
    let body = event(&mut readonly, &viewer, form("create_definition", &[])).await?;
    require(
        body.html.contains("You cannot do that for this company."),
        "forged write from the view-only page is refused",
    )?;
    println!("PM2 page proof complete");
    Ok(())
}
