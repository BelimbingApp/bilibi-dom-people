use super::*;

// ---- Skills catalog (PK1) --------------------------------------------------
//
// The Skills catalog proof: the admin (grant-all) gives the manager the two
// catalog capabilities directly and keeps the publish and policy keys, so the
// block shows which key each write needs. The manager builds a category, two
// skills, a proficiency scale and a requirement profile and meets every
// refusal the facade makes on the way; the admin publishes, retires and
// supersedes profile versions and sets the policy; `requirements` resolves
// the version in force on a day; and PostgreSQL refuses what the facade never
// attempts. Position targets use Workforce
// and are unavailable here because Organisation is not mounted.

use bilibi_people_skills::{
    CategoryRevision, Direction, ItemInput, Level, NewCategory, NewProfile, NewScale, NewSkill,
    PolicyChanges, ProfileChanges, SkillRevision, Skills, SkillsError, Target, CATALOG_MANAGE,
    CATALOG_VIEW, SERVICE as SKILLS_SERVICE,
};
use rust_decimal::Decimal;

fn show_skills<T: std::fmt::Debug>(
    step: &str,
    result: std::result::Result<T, SkillsError>,
) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Some(value)
        }
        Err(SkillsError::Invalid(errors)) => {
            let fields = errors
                .iter()
                .map(|error| format!("{} {}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("; ");
            println!("{step}: refused (invalid input: {fields})");
            None
        }
        Err(error) => {
            println!("{step}: refused ({error})");
            None
        }
    }
}

fn weight(value: &str) -> Decimal {
    value.parse().expect("a decimal literal")
}

fn item(
    skill_id: i64,
    level: i32,
    criticality: &str,
    weight_percent: &str,
    mandatory: bool,
) -> ItemInput {
    ItemInput {
        skill_id,
        required_level: level,
        criticality: criticality.into(),
        weight_percent: weight(weight_percent),
        mandatory,
        evidence_standard: None,
    }
}

fn level(level: i32, name: &str, anchor: &str, authority: &str) -> Level {
    Level {
        level,
        name: name.into(),
        anchor: anchor.into(),
        authority: authority.into(),
    }
}

async fn skills_catalog(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    manager_employee_id: i64,
) -> Result<()> {
    let skills = services.get::<Skills>(SKILLS_SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    println!("--- Skills catalog ---");
    for key in [CATALOG_VIEW, CATALOG_MANAGE] {
        grants.set_direct(admin, MANAGER, key, Some(true)).await?;
    }
    println!("admin grants manager@example.test {CATALOG_VIEW} and {CATALOG_MANAGE} directly; the publish and policy keys stay with the admin");

    // Policy: seven company settings, read by anyone who sees the company,
    // written with `people.skills.policy.manage`.
    show_skills(
        "manager reads company 1's skills policy",
        skills.policy(manager, COMPANY).await,
    );
    show_skills(
        "manager sets the backup minimum",
        skills
            .put_policy(
                manager,
                COMPANY,
                &PolicyChanges {
                    backup_minimum: Some(3),
                    ..Default::default()
                },
            )
            .await,
    );
    show_skills(
        "admin sets the backup minimum to 3 and the critical multiplier to 4",
        skills
            .put_policy(
                admin,
                COMPANY,
                &PolicyChanges {
                    backup_minimum: Some(3),
                    multiplier_critical: Some(4),
                    ..Default::default()
                },
            )
            .await,
    );
    show_skills(
        "admin sets the backup minimum to 0",
        skills
            .put_policy(
                admin,
                COMPANY,
                &PolicyChanges {
                    backup_minimum: Some(0),
                    ..Default::default()
                },
            )
            .await,
    );
    show_skills(
        "admin sets nothing",
        skills
            .put_policy(admin, COMPANY, &PolicyChanges::default())
            .await,
    );

    // Categories.
    show_skills(
        "viewer adds the category 'technical'",
        skills
            .create_category(
                viewer,
                COMPANY,
                &NewCategory {
                    code: "technical".into(),
                    name: "Technical skills".into(),
                    description: None,
                },
            )
            .await,
    );
    let technical = saved(show_skills(
        "manager adds the category 'technical'",
        skills
            .create_category(
                manager,
                COMPANY,
                &NewCategory {
                    code: " technical ".into(),
                    name: "Technical skills".into(),
                    description: Some("Hands-on skills on the shop floor".into()),
                },
            )
            .await,
    ))?;
    show_skills(
        "manager adds the category 'technical' again",
        skills
            .create_category(
                manager,
                COMPANY,
                &NewCategory {
                    code: "technical".into(),
                    name: "Duplicate".into(),
                    description: None,
                },
            )
            .await,
    );
    show_skills(
        "manager adds a category with the code 'Bad Code!' and no name",
        skills
            .create_category(
                manager,
                COMPANY,
                &NewCategory {
                    code: "Bad Code!".into(),
                    name: "  ".into(),
                    description: None,
                },
            )
            .await,
    );
    let soft = saved(show_skills(
        "manager adds the category 'soft'",
        skills
            .create_category(
                manager,
                COMPANY,
                &NewCategory {
                    code: "soft".into(),
                    name: "Soft skills".into(),
                    description: None,
                },
            )
            .await,
    ))?;
    show_skills(
        "manager renames 'soft' to 'Behavioural skills'",
        skills
            .update_category(
                manager,
                COMPANY,
                soft.id,
                &CategoryRevision {
                    name: "Behavioural skills".into(),
                    description: None,
                },
            )
            .await,
    );

    // Skills.
    let welding = saved(show_skills(
        "manager adds the skill 'welding' under 'technical'",
        skills
            .create_skill(
                manager,
                COMPANY,
                &NewSkill {
                    category_id: technical.id,
                    code: "welding".into(),
                    name: "MIG welding".into(),
                    definition: "Joins mild steel to drawing with MIG equipment.".into(),
                    evidence_guide: Some("A supervised weld inspected to spec.".into()),
                    critical: true,
                    reassessment_months: Some(12),
                },
            )
            .await,
    ))?;
    let forklift = saved(show_skills(
        "manager adds the skill 'forklift' under 'technical'",
        skills
            .create_skill(
                manager,
                COMPANY,
                &NewSkill {
                    category_id: technical.id,
                    code: "forklift".into(),
                    name: "Forklift operation".into(),
                    definition: "Moves pallets safely in the warehouse.".into(),
                    evidence_guide: None,
                    critical: false,
                    reassessment_months: None,
                },
            )
            .await,
    ))?;
    show_skills(
        "manager adds a skill under a category that does not exist",
        skills
            .create_skill(
                manager,
                COMPANY,
                &NewSkill {
                    category_id: i64::MAX,
                    code: "ghost".into(),
                    name: "Ghost".into(),
                    definition: "Nothing.".into(),
                    evidence_guide: None,
                    critical: false,
                    reassessment_months: None,
                },
            )
            .await,
    );
    show_skills(
        "manager adds a skill reassessed every 0 months with the code 'welding'",
        skills
            .create_skill(
                manager,
                COMPANY,
                &NewSkill {
                    category_id: technical.id,
                    code: "welding".into(),
                    name: "Again".into(),
                    definition: "Again.".into(),
                    evidence_guide: None,
                    critical: false,
                    reassessment_months: Some(0),
                },
            )
            .await,
    );
    show_skills(
        "manager revises 'welding' (name, interval 18 months)",
        skills
            .update_skill(
                manager,
                COMPANY,
                welding.id,
                &SkillRevision {
                    category_id: technical.id,
                    name: "MIG and TIG welding".into(),
                    definition: welding.definition.clone(),
                    evidence_guide: welding.evidence_guide.clone(),
                    critical: true,
                    reassessment_months: Some(18),
                },
            )
            .await,
    );
    show_skills(
        "manager deactivates 'technical' while it has active skills",
        skills
            .set_category_active(manager, COMPANY, technical.id, false)
            .await,
    );
    show_skills(
        "manager deactivates 'soft'",
        skills
            .set_category_active(manager, COMPANY, soft.id, false)
            .await,
    );
    show_skills(
        "manager adds a skill under the inactive 'soft'",
        skills
            .create_skill(
                manager,
                COMPANY,
                &NewSkill {
                    category_id: soft.id,
                    code: "listening".into(),
                    name: "Active listening".into(),
                    definition: "Listens.".into(),
                    evidence_guide: None,
                    critical: false,
                    reassessment_months: None,
                },
            )
            .await,
    );
    if let Some(list) = show_skills(
        "manager lists the categories",
        skills.list_categories(manager, COMPANY).await,
    ) {
        println!("  {} category(ies)", list.len());
    }
    if let Some(list) = show_skills(
        "viewer lists the skills",
        skills.list_skills(viewer, COMPANY).await,
    ) {
        println!("  {} skill(s)", list.len());
    }

    // Proficiency scales: numbered versions, published one at a time.
    let scale = saved(show_skills(
        "manager starts the scale 'proficiency'",
        skills
            .create_scale(
                manager,
                COMPANY,
                &NewScale {
                    code: "proficiency".into(),
                    name: "Proficiency".into(),
                },
            )
            .await,
    ))?;
    show_skills(
        "manager starts the scale 'proficiency' again",
        skills
            .create_scale(
                manager,
                COMPANY,
                &NewScale {
                    code: "proficiency".into(),
                    name: "Again".into(),
                },
            )
            .await,
    );
    show_skills(
        "manager publishes the scale without levels",
        skills.publish_scale(manager, COMPANY, scale.id).await,
    );
    show_skills(
        "manager puts level 0 'Novice'",
        skills
            .put_scale_level(
                manager,
                COMPANY,
                scale.id,
                &level(0, "Novice", "Works under close supervision.", "None."),
            )
            .await,
    );
    show_skills(
        "manager puts level 2 'Expert' (skipping 1)",
        skills
            .put_scale_level(
                manager,
                COMPANY,
                scale.id,
                &level(2, "Expert", "Sets the standard.", "Signs off others' work."),
            )
            .await,
    );
    show_skills(
        "manager publishes the scale with a gap at level 1",
        skills.publish_scale(manager, COMPANY, scale.id).await,
    );
    show_skills(
        "manager puts level 1 named 'novice' (a duplicate name)",
        skills
            .put_scale_level(
                manager,
                COMPANY,
                scale.id,
                &level(1, "novice", "Works alone.", "Signs own work."),
            )
            .await,
    );
    show_skills(
        "manager publishes the scale with two levels named Novice",
        skills.publish_scale(manager, COMPANY, scale.id).await,
    );
    show_skills(
        "manager replaces level 1 as 'Competent'",
        skills
            .put_scale_level(
                manager,
                COMPANY,
                scale.id,
                &level(
                    1,
                    "Competent",
                    "Works alone to standard.",
                    "Signs own work.",
                ),
            )
            .await,
    );
    show_skills(
        "manager puts level 21",
        skills
            .put_scale_level(
                manager,
                COMPANY,
                scale.id,
                &level(21, "Beyond", "No.", "No."),
            )
            .await,
    );
    show_skills(
        "manager removes level 7, which the scale lacks",
        skills
            .delete_scale_level(manager, COMPANY, scale.id, 7)
            .await,
    );
    let scale_v1 = saved(show_skills(
        "manager publishes the scale (levels 0, 1, 2)",
        skills.publish_scale(manager, COMPANY, scale.id).await,
    ))?;
    show_skills(
        "manager puts a level on the published scale",
        skills
            .put_scale_level(
                manager,
                COMPANY,
                scale_v1.id,
                &level(3, "Master", "No.", "No."),
            )
            .await,
    );
    show_skills(
        "manager renames the published scale",
        skills
            .rename_scale(manager, COMPANY, scale_v1.id, "Renamed")
            .await,
    );
    let scale_v2 = saved(show_skills(
        "manager drafts version 2 of 'proficiency' (levels copied)",
        skills
            .new_scale_version(manager, COMPANY, scale_v1.id)
            .await,
    ))?;
    show_skills(
        "manager drafts another version while the draft is open",
        skills
            .new_scale_version(manager, COMPANY, scale_v1.id)
            .await,
    );
    show_skills(
        "manager renames the draft 'Proficiency (2026)'",
        skills
            .rename_scale(manager, COMPANY, scale_v2.id, "Proficiency (2026)")
            .await,
    );
    show_skills(
        "manager adds level 3 'Master' to the draft",
        skills
            .put_scale_level(
                manager,
                COMPANY,
                scale_v2.id,
                &level(3, "Master", "Teaches the skill.", "Certifies others."),
            )
            .await,
    );

    // Requirement profiles against the published scale.
    show_skills(
        "manager starts the profile 'operator' on the draft scale",
        skills
            .create_profile(
                manager,
                COMPANY,
                &NewProfile {
                    code: "operator".into(),
                    name: "Machine operator".into(),
                    scale_id: scale_v2.id,
                },
            )
            .await,
    );
    let operator = saved(show_skills(
        "manager starts the profile 'operator' on the published scale",
        skills
            .create_profile(
                manager,
                COMPANY,
                &NewProfile {
                    code: "operator".into(),
                    name: "Machine operator".into(),
                    scale_id: scale_v1.id,
                },
            )
            .await,
    ))?;
    show_skills(
        "manager requires welding at level 5",
        skills
            .put_item(
                manager,
                COMPANY,
                operator.id,
                &item(welding.id, 5, "critical", "60", true),
            )
            .await,
    );
    show_skills(
        "manager requires welding with a weight of 33.333",
        skills
            .put_item(
                manager,
                COMPANY,
                operator.id,
                &item(welding.id, 2, "critical", "33.333", true),
            )
            .await,
    );
    show_skills(
        "manager requires welding at level 2, critical, 60%, mandatory",
        skills
            .put_item(
                manager,
                COMPANY,
                operator.id,
                &item(welding.id, 2, "critical", "60", true),
            )
            .await,
    );
    show_skills(
        "manager requires forklift at level 1, essential, 40%",
        skills
            .put_item(
                manager,
                COMPANY,
                operator.id,
                &item(forklift.id, 1, "essential", "40.00", false),
            )
            .await,
    );
    show_skills(
        "manager requires a skill that does not exist",
        skills
            .put_item(
                manager,
                COMPANY,
                operator.id,
                &item(i64::MAX, 1, "essential", "10", false),
            )
            .await,
    );
    show_skills(
        "admin publishes 'operator' without a target",
        skills
            .publish_profile(admin, COMPANY, operator.id, date(2026, 1, 1))
            .await,
    );
    show_skills(
        "manager targets position 1",
        skills
            .add_selector(manager, COMPANY, operator.id, Target::Position(1))
            .await,
    );
    show_skills(
        "manager lists the positions a profile may target",
        skills.position_options(manager, COMPANY).await,
    );
    show_skills(
        "manager targets the whole company",
        skills
            .add_selector(manager, COMPANY, operator.id, Target::Company)
            .await,
    );
    show_skills(
        "manager targets the whole company again",
        skills
            .add_selector(manager, COMPANY, operator.id, Target::Company)
            .await,
    );
    let draft = saved(show_skills(
        "manager reads the 'operator' draft",
        skills.get_profile(manager, COMPANY, operator.id).await,
    ))?;
    let forklift_item = saved(
        draft
            .items
            .iter()
            .find(|item| item.skill_id == forklift.id)
            .map(|item| item.id),
    )?;
    show_skills(
        "manager moves the forklift requirement up",
        skills
            .move_item(manager, COMPANY, operator.id, forklift_item, Direction::Up)
            .await,
    );
    show_skills(
        "manager moves the forklift requirement up again (already first)",
        skills
            .move_item(manager, COMPANY, operator.id, forklift_item, Direction::Up)
            .await,
    );
    show_skills(
        "manager removes a requirement that does not exist",
        skills
            .remove_item(manager, COMPANY, operator.id, i64::MAX)
            .await,
    );
    show_skills(
        "manager moves the draft to the draft scale",
        skills
            .update_profile(
                manager,
                COMPANY,
                operator.id,
                &ProfileChanges {
                    name: None,
                    scale_id: Some(scale_v2.id),
                },
            )
            .await,
    );
    show_skills(
        "manager publishes 'operator'",
        skills
            .publish_profile(manager, COMPANY, operator.id, date(2026, 1, 1))
            .await,
    );
    let operator_v1 = saved(show_skills(
        "admin publishes 'operator' from 2026-01-01",
        skills
            .publish_profile(admin, COMPANY, operator.id, date(2026, 1, 1))
            .await,
    ))?;
    show_skills(
        "manager requires a third skill on the published version",
        skills
            .put_item(
                manager,
                COMPANY,
                operator_v1.id,
                &item(forklift.id, 1, "development", "0", false),
            )
            .await,
    );
    show_skills(
        &format!("manager reads the profile #{}", operator_v1.id),
        skills.get_profile(manager, COMPANY, operator_v1.id).await,
    );
    show_skills(
        "manager resolves the requirements in force on 2026-06-01",
        skills
            .requirements(manager, COMPANY, None, date(2026, 6, 1))
            .await
            .map(|detail| detail.map(|detail| (detail.profile.code, detail.profile.version))),
    );
    show_skills(
        "manager resolves the requirements in force on 2025-12-31",
        skills
            .requirements(manager, COMPANY, None, date(2025, 12, 31))
            .await
            .map(|detail| detail.map(|detail| detail.profile.version)),
    );
    show_skills(
        &format!("manager asks which position employee #{manager_employee_id} holds"),
        skills
            .position_of(manager, COMPANY, manager_employee_id, date(2026, 6, 1))
            .await,
    );

    // Another code targeting the same people overlaps.
    let supervisor = saved(show_skills(
        "manager starts the profile 'supervisor'",
        skills
            .create_profile(
                manager,
                COMPANY,
                &NewProfile {
                    code: "supervisor".into(),
                    name: "Shift supervisor".into(),
                    scale_id: scale_v1.id,
                },
            )
            .await,
    ))?;
    show_skills(
        "admin publishes 'supervisor' with no requirement",
        skills
            .publish_profile(admin, COMPANY, supervisor.id, date(2026, 3, 1))
            .await,
    );
    show_skills(
        "manager requires welding at level 2, critical, 50%",
        skills
            .put_item(
                manager,
                COMPANY,
                supervisor.id,
                &item(welding.id, 2, "critical", "50", true),
            )
            .await,
    );
    show_skills(
        "manager targets the whole company",
        skills
            .add_selector(manager, COMPANY, supervisor.id, Target::Company)
            .await,
    );
    show_skills(
        "admin publishes 'supervisor' with weights totalling 50",
        skills
            .publish_profile(admin, COMPANY, supervisor.id, date(2026, 3, 1))
            .await,
    );
    show_skills(
        "manager requires welding at level 2, critical, 100%",
        skills
            .put_item(
                manager,
                COMPANY,
                supervisor.id,
                &item(welding.id, 2, "critical", "100", true),
            )
            .await,
    );
    show_skills(
        "admin publishes 'supervisor' from 2026-03-01 over 'operator'",
        skills
            .publish_profile(admin, COMPANY, supervisor.id, date(2026, 3, 1))
            .await,
    );
    show_skills(
        "manager discards the 'supervisor' draft",
        skills
            .discard_profile(manager, COMPANY, supervisor.id)
            .await,
    );

    // The next version of 'operator' adopts the scale's published version
    // when every level exists there.
    show_skills(
        "manager publishes scale version 2 (retires version 1)",
        skills
            .publish_scale(manager, COMPANY, scale_v2.id)
            .await
            .map(|scale| (scale.version, scale.status, scale.levels.len())),
    );
    show_skills(
        "manager discards the retired scale",
        skills.discard_scale(manager, COMPANY, scale_v1.id).await,
    );
    let operator_v2 = saved(show_skills(
        "manager drafts 'operator' version 2 (adopts scale version 2)",
        skills
            .new_profile_version(manager, COMPANY, operator_v1.id)
            .await
            .map(|draft| {
                (
                    draft.profile.profile.clone(),
                    draft.profile.items.len(),
                    draft.profile.selectors.len(),
                    draft.missing_levels.clone(),
                )
            }),
    ))?
    .0;
    show_skills(
        "manager drafts 'operator' version 3 while version 2 is a draft",
        skills
            .new_profile_version(manager, COMPANY, operator_v1.id)
            .await,
    );
    show_skills(
        "manager raises welding to level 3 on the draft",
        skills
            .put_item(
                manager,
                COMPANY,
                operator_v2.id,
                &item(welding.id, 3, "critical", "60", true),
            )
            .await,
    );
    show_skills(
        "admin publishes version 2 from 2025-12-01, before version 1 started",
        skills
            .publish_profile(admin, COMPANY, operator_v2.id, date(2025, 12, 1))
            .await,
    );
    show_skills(
        "admin publishes version 2 from 2026-07-01",
        skills
            .publish_profile(admin, COMPANY, operator_v2.id, date(2026, 7, 1))
            .await,
    );
    if let Some(versions) = show_skills(
        "manager lists the 'operator' versions",
        skills.list_profiles(manager, COMPANY).await,
    ) {
        for profile in versions {
            println!(
                "  {} v{} {} from {:?} to {:?}",
                profile.code,
                profile.version,
                profile.status,
                profile.effective_from,
                profile.effective_to
            );
        }
    }
    show_skills(
        "manager resolves the requirements in force on 2026-06-30",
        skills
            .requirements(manager, COMPANY, None, date(2026, 6, 30))
            .await
            .map(|detail| detail.map(|detail| detail.profile.version)),
    );
    show_skills(
        "manager resolves the requirements in force on 2026-07-01",
        skills
            .requirements(manager, COMPANY, None, date(2026, 7, 1))
            .await
            .map(|detail| detail.map(|detail| detail.profile.version)),
    );
    show_skills(
        "admin retires version 2 on 2026-05-01, before it starts",
        skills
            .retire_profile(admin, COMPANY, operator_v2.id, date(2026, 5, 1))
            .await,
    );
    show_skills(
        "manager retires version 2 on 2026-12-31",
        skills
            .retire_profile(manager, COMPANY, operator_v2.id, date(2026, 12, 31))
            .await,
    );
    show_skills(
        "admin retires version 2 on 2026-12-31",
        skills
            .retire_profile(admin, COMPANY, operator_v2.id, date(2026, 12, 31))
            .await,
    );
    show_skills(
        "admin retires version 2 again",
        skills
            .retire_profile(admin, COMPANY, operator_v2.id, date(2026, 12, 31))
            .await,
    );
    show_skills(
        "manager resolves the requirements in force on 2027-01-01",
        skills
            .requirements(manager, COMPANY, None, date(2027, 1, 1))
            .await
            .map(|detail| detail.map(|detail| detail.profile.version)),
    );
    show_skills(
        "manager discards the published version 1",
        skills
            .discard_profile(manager, COMPANY, operator_v1.id)
            .await,
    );
    show_skills(
        "manager reads a profile of a company that does not exist",
        skills.get_profile(manager, i64::MAX, operator_v1.id).await,
    );
    show_skills(
        &format!("manager reads tenant 2's company {OTHER_TENANT_COMPANY}'s scales"),
        skills.list_scales(manager, OTHER_TENANT_COMPANY).await,
    );
    if let Some(list) = show_skills(
        "manager lists the scales",
        skills.list_scales(manager, COMPANY).await.map(|scales| {
            scales
                .iter()
                .map(|scale| {
                    format!(
                        "{} v{} {} ({} levels)",
                        scale.code,
                        scale.version,
                        scale.status,
                        scale.levels.len()
                    )
                })
                .collect::<Vec<_>>()
        }),
    ) {
        println!("  {}", list.join("; "));
    }

    // PostgreSQL itself refuses what the facade never attempts.
    for (step, sql, id) in [
        (
            "SQL changes a skill's code",
            "UPDATE people_skills SET code = 'other' WHERE id = $1",
            welding.id,
        ),
        (
            "SQL renames the published scale",
            "UPDATE people_skill_scales SET name = 'Other' WHERE id = $1",
            scale_v2.id,
        ),
        (
            "SQL adds a level to the published scale",
            "INSERT INTO people_skill_scale_levels (tenant_id, company_id, scale_id, level, name, anchor, authority) SELECT tenant_id, company_id, id, 9, 'x', 'x', 'x' FROM people_skill_scales WHERE id = $1",
            scale_v2.id,
        ),
        (
            "SQL deletes the retired scale",
            "DELETE FROM people_skill_scales WHERE id = $1",
            scale_v1.id,
        ),
        (
            "SQL changes a requirement of the retired profile version",
            "UPDATE people_skill_profile_items SET required_level = 0 WHERE profile_id = $1",
            operator_v1.id,
        ),
        (
            "SQL reopens the retired profile version as a draft",
            "UPDATE people_skill_profiles SET status = 'draft' WHERE id = $1",
            operator_v1.id,
        ),
        (
            "SQL deletes the retired profile version",
            "DELETE FROM people_skill_profiles WHERE id = $1",
            operator_v1.id,
        ),
        (
            "SQL moves a skill to Company 002",
            "UPDATE people_skills SET company_id = 2 WHERE id = $1",
            forklift.id,
        ),
    ] {
        refused(client, step, sql, id).await;
    }
    Ok(())
}

walkthrough_step!(
    "skills_catalog",
    after = ["entry_operation"],
    |ctx| async move {
        let client = ctx.db.get().await?;
        // Skills catalog (PK1): its own proof block below.
        skills_catalog(
            &ctx.services,
            &client,
            &ctx.admin,
            &ctx.manager,
            &ctx.viewer,
            ctx.manager_employment.id,
        )
        .await?;

        Ok(())
    }
);
