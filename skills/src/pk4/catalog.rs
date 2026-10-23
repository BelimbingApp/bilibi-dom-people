//! Mirrors Bilimbi Skills.Web.CatalogLive: four catalog sections and scale levels.
use super::*;
use crate::{
    Category, Level, NewCategory, NewProfile, NewScale, NewSkill, Profile, Scale, Skill,
    CATALOG_MANAGE,
};
use bilibi_ui::{
    lists::RecordLink,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest},
};

enum Command {
    Publish(i64),
    Retire(i64),
    Discard(i64),
    DeleteLevel(i64, i32),
}
struct Pending {
    company: i64,
    command: Command,
    detail: String,
}
impl Command {
    fn consequence(&self) -> &'static str {
        match self {
            Self::Publish(_) => "Publish this proficiency scale?",
            Self::Retire(_) => "Retire this proficiency scale?",
            Self::Discard(_) => "Discard this scale draft and its levels?",
            Self::DeleteLevel(..) => "Remove this level from the draft?",
        }
    }
    fn verb(&self) -> &'static str {
        match self {
            Self::Publish(_) => "Publish",
            Self::Retire(_) => "Retire",
            Self::Discard(_) => "Discard",
            Self::DeleteLevel(..) => "Remove",
        }
    }
}
pub(super) struct Catalog {
    core: Arc<Core>,
    company: Company,
    framed: bool,
    available: bool,
    can_manage: bool,
    categories: Vec<Category>,
    skills: Vec<Skill>,
    scales: Vec<Scale>,
    profiles: Vec<Profile>,
    categories_table: Table,
    skills_table: Table,
    scales_table: Table,
    profiles_table: Table,
    levels: HashMap<i64, Table>,
    drafts: Drafts,
    flash: Flash,
    pending: Option<Pending>,
}
impl Catalog {
    pub(super) fn new(
        core: Arc<Core>,
        query: &HashMap<String, String>,
        framed: bool,
    ) -> Result<Self> {
        Ok(Self {
            core,
            company: Company::new(query),
            framed,
            available: false,
            can_manage: false,
            categories: vec![],
            skills: vec![],
            scales: vec![],
            profiles: vec![],
            categories_table: Table::new(
                "skill-categories",
                "categories_grid",
                &[
                    ("name", "Name"),
                    ("code", "Code"),
                    ("status", "Status"),
                    ("actions", "Actions"),
                ],
            )?,
            skills_table: Table::new(
                "skills",
                "skills_grid",
                &[
                    ("name", "Skill"),
                    ("code", "Code"),
                    ("category", "Category"),
                    ("critical", "Critical"),
                    ("status", "Status"),
                    ("actions", "Actions"),
                ],
            )?,
            scales_table: Table::new(
                "skill-scales",
                "scales_grid",
                &[
                    ("name", "Scale"),
                    ("code", "Code"),
                    ("version", "Version"),
                    ("status", "Status"),
                    ("actions", "Actions"),
                ],
            )?,
            profiles_table: Table::new(
                "skill-profiles",
                "profiles_grid",
                &[
                    ("name", "Profile"),
                    ("code", "Code"),
                    ("version", "Version"),
                    ("status", "Status"),
                    ("scale", "Scale"),
                    ("effective", "Effective"),
                ],
            )?,
            levels: HashMap::new(),
            drafts: Drafts::default(),
            flash: Flash::default(),
            pending: None,
        })
    }
    async fn load(&mut self, scope: &Scope, changed: bool) -> Result<()> {
        let previous = self.company.id;
        self.company.load(&self.core, scope, true).await?;
        if previous != self.company.id {
            self.drafts = Drafts::default();
            self.pending = None;
        }
        self.available = false;
        self.can_manage = false;
        self.categories.clear();
        self.skills.clear();
        self.scales.clear();
        self.profiles.clear();
        if let Some(c) = self.company.id {
            let s = &self.core.skills;
            if let (Ok(categories), Ok(skills), Ok(scales), Ok(profiles)) = (
                s.list_categories(scope, c).await,
                s.list_skills(scope, c).await,
                s.list_scales(scope, c).await,
                s.list_profiles(scope, c).await,
            ) {
                self.categories = categories;
                self.skills = skills;
                self.scales = scales;
                self.profiles = profiles;
                self.available = true;
                self.can_manage = self
                    .company
                    .allowed(&self.core, scope, CATALOG_MANAGE)
                    .await;
            }
        }
        if !self.available {
            self.pending = None;
        }
        if changed {
            self.categories_table
                .load(&self.categories, |r| r.id.to_string());
            self.skills_table.load(&self.skills, |r| r.id.to_string());
            self.scales_table.load(&self.scales, |r| r.id.to_string());
            self.profiles_table
                .load(&self.profiles, |r| r.id.to_string());
            self.levels
                .retain(|id, _| self.scales.iter().any(|s| s.id == *id));
            for scale in &self.scales {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.levels.entry(scale.id)
                {
                    entry.insert(Table::new(
                        &format!("skill-scale-levels-{}", scale.id),
                        &format!("levels_grid_{}", scale.id),
                        &[
                            ("level", "Level"),
                            ("name", "Name"),
                            ("anchor", "Observable behaviour"),
                            ("authority", "Authority"),
                            ("actions", "Actions"),
                        ],
                    )?);
                }
                if let Some(table) = self.levels.get_mut(&scale.id) {
                    table.load(&scale.levels, |l| l.level.to_string());
                }
            }
        }
        Ok(())
    }
    async fn authorize(&mut self, scope: &Scope) -> bool {
        if self
            .company
            .allowed(&self.core, scope, CATALOG_MANAGE)
            .await
        {
            true
        } else {
            self.pending = None;
            self.flash.error(FORBIDDEN);
            false
        }
    }
    async fn event(&mut self, scope: &Scope, p: &Value) -> Result<bool> {
        let event = text(p, "type");
        match event {
            "dismiss_flash" => {
                self.flash.dismiss(p);
                return Ok(false);
            }
            "dismiss_confirm" => {
                self.pending = None;
                return Ok(false);
            }
            "categories_grid" => {
                self.categories_table.event(p);
                return Ok(false);
            }
            "skills_grid" => {
                self.skills_table.event(p);
                return Ok(false);
            }
            "scales_grid" => {
                self.scales_table.event(p);
                return Ok(false);
            }
            "profiles_grid" => {
                self.profiles_table.event(p);
                return Ok(false);
            }
            e if e.starts_with("levels_grid_") => {
                if let Some(table) = e
                    .strip_prefix("levels_grid_")
                    .and_then(|id| id.parse::<i64>().ok())
                    .and_then(|id| self.levels.get_mut(&id))
                {
                    table.event(p);
                }
                return Ok(false);
            }
            "select_company" => {
                self.company.id = id(p, "company_id");
                self.pending = None;
                self.drafts = Drafts::default();
                return Ok(true);
            }
            "create_category"
            | "set_category_active"
            | "create_skill"
            | "set_skill_active"
            | "create_scale"
            | "put_level"
            | "delete_level"
            | "scale_publish"
            | "scale_retire"
            | "scale_discard"
            | "scale_new_version"
            | "create_profile"
            | "confirm" => {}
            _ => return Ok(false),
        }
        if !self.authorize(scope).await {
            return Ok(true);
        }
        if event != "confirm" {
            self.pending = None;
            if !self.selection_exists(event, p) {
                self.flash.error(MISSING);
                return Ok(false);
            }
        }
        let c = self.company.id.unwrap_or_default();
        let s = &self.core.skills;
        match event {
            "create_category" => {
                let f = "skill-category-form";
                self.drafts.remember(f, p);
                self.drafts.result(
                    f,
                    &mut self.flash,
                    s.create_category(
                        scope,
                        c,
                        &NewCategory {
                            code: text(p, "category[code]").into(),
                            name: text(p, "category[name]").into(),
                            description: optional(text(p, "category[description]")),
                        },
                    )
                    .await,
                    "Category added.",
                    "Enter a unique lowercase code and a name.",
                );
            }
            "create_skill" => {
                let f = "skill-form";
                self.drafts.remember(f, p);
                let months = optional(text(p, "skill[reassessment_months]"));
                let new = NewSkill {
                    category_id: id(p, "skill[category_id]").unwrap_or_default(),
                    code: text(p, "skill[code]").into(),
                    name: text(p, "skill[name]").into(),
                    definition: text(p, "skill[definition]").into(),
                    evidence_guide: optional(text(p, "skill[evidence_guide]")),
                    critical: text(p, "skill[critical]") == "true",
                    reassessment_months: months.map(|v| v.parse().unwrap_or(-1)),
                };
                self.drafts.result(
                    f,
                    &mut self.flash,
                    s.create_skill(scope, c, &new).await,
                    "Skill added.",
                    "Choose an active category and enter a unique code, a name and a definition.",
                );
            }
            "set_category_active" => {
                if let Some(category) = self.categories.iter().find(|r| Some(r.id) == id(p, "id")) {
                    self.drafts.result(
                        "",
                        &mut self.flash,
                        s.set_category_active(scope, c, category.id, !category.active)
                            .await,
                        "Category updated.",
                        "Deactivate the category's active skills first.",
                    );
                }
            }
            "set_skill_active" => {
                if let Some(skill) = self.skills.iter().find(|r| Some(r.id) == id(p, "id")) {
                    let result = s.set_skill_active(scope, c, skill.id, !skill.active).await;
                    if matches!(result, Err(SkillsError::CategoryUnavailable)) {
                        self.flash.error("Reactivate the skill's category first.");
                    } else {
                        self.drafts.result(
                            "",
                            &mut self.flash,
                            result,
                            "Skill updated.",
                            "Reactivate the skill's category first.",
                        );
                    }
                }
            }
            "create_scale" => {
                let f = "skill-scale-form";
                self.drafts.remember(f, p);
                self.drafts.result(
                    f,
                    &mut self.flash,
                    s.create_scale(
                        scope,
                        c,
                        &NewScale {
                            code: text(p, "scale[code]").into(),
                            name: text(p, "scale[name]").into(),
                        },
                    )
                    .await,
                    "Scale draft added.",
                    "Enter a new lowercase code and a name.",
                );
            }
            "put_level" => {
                let scale = id(p, "level[scale_id]").unwrap_or_default();
                let f = format!("skill-level-form-{scale}");
                self.drafts.remember(&f, p);
                let level = Level {
                    level: integer(p, "level[level]"),
                    name: text(p, "level[name]").into(),
                    anchor: text(p, "level[anchor]").into(),
                    authority: text(p, "level[authority]").into(),
                };
                self.drafts.result(
                    &f,
                    &mut self.flash,
                    s.put_scale_level(scope, c, scale, &level).await,
                    "Level saved.",
                    "Enter a level from 0 to 20 with a name, observable anchor and authority.",
                );
            }
            "create_profile" => {
                let f = "skill-profile-form";
                self.drafts.remember(f, p);
                self.drafts.result(
                    f,
                    &mut self.flash,
                    s.create_profile(
                        scope,
                        c,
                        &NewProfile {
                            code: text(p, "profile[code]").into(),
                            name: text(p, "profile[name]").into(),
                            scale_id: id(p, "profile[scale_id]").unwrap_or_default(),
                        },
                    )
                    .await,
                    "Requirement profile draft added.",
                    "Enter a new lowercase code, a name and a published scale.",
                );
            }
            "scale_new_version" => {
                if let Some(scale) = self
                    .scales
                    .iter()
                    .find(|r| Some(r.id) == id(p, "id") && r.status == "published")
                {
                    let result = s.new_scale_version(scope, c, scale.id).await;
                    self.scale_result(result, "New scale draft added.");
                }
            }
            "scale_publish" | "scale_retire" | "scale_discard" => {
                if let Some(scale) = self.scales.iter().find(|r| Some(r.id) == id(p, "id")) {
                    let command = match event {
                        "scale_publish" => Command::Publish(scale.id),
                        "scale_retire" => Command::Retire(scale.id),
                        _ => Command::Discard(scale.id),
                    };
                    let detail = format!(
                        "{} ({}) v{} · {}",
                        scale.name, scale.code, scale.version, scale.status
                    );
                    self.pending = Some(Pending {
                        company: c,
                        command,
                        detail,
                    });
                }
                return Ok(false);
            }
            "delete_level" => {
                if let Some((scale, level)) = text(p, "id")
                    .split_once(':')
                    .and_then(|(s, l)| Some((s.parse::<i64>().ok()?, l.parse::<i32>().ok()?)))
                {
                    if let Some(s) = self
                        .scales
                        .iter()
                        .find(|s| s.id == scale && s.status == "draft")
                    {
                        if let Some(l) = s.levels.iter().find(|l| l.level == level) {
                            self.pending = Some(Pending {
                                company: c,
                                command: Command::DeleteLevel(scale, level),
                                detail: format!(
                                    "{} v{} · {} · {}",
                                    s.name, s.version, l.level, l.name
                                ),
                            });
                        }
                    }
                }
                return Ok(false);
            }
            "confirm" => {
                if let Some(pending) = self.pending.take().filter(|p| p.company == c) {
                    match pending.command {
                        Command::Publish(id) => {
                            let r = s.publish_scale(scope, c, id).await;
                            self.scale_result(r, "Scale published.");
                        }
                        Command::Retire(id) => {
                            let r = s.retire_scale(scope, c, id).await;
                            self.scale_result(r, "Scale retired.");
                        }
                        Command::Discard(id) => {
                            let r = s.discard_scale(scope, c, id).await;
                            self.scale_result(r, "Scale draft discarded.");
                        }
                        Command::DeleteLevel(id, level) => {
                            let r = s.delete_scale_level(scope, c, id, level).await;
                            self.scale_result(r, "Level removed.");
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    }
    fn selection_exists(&self, event: &str, p: &Value) -> bool {
        match event {
            "set_category_active" => self.categories.iter().any(|r| Some(r.id) == id(p, "id")),
            "set_skill_active" => self.skills.iter().any(|r| Some(r.id) == id(p, "id")),
            "put_level" => self
                .scales
                .iter()
                .any(|r| Some(r.id) == id(p, "level[scale_id]") && r.status == "draft"),
            "scale_publish" | "scale_discard" => self
                .scales
                .iter()
                .any(|r| Some(r.id) == id(p, "id") && r.status == "draft"),
            "scale_retire" | "scale_new_version" => self
                .scales
                .iter()
                .any(|r| Some(r.id) == id(p, "id") && r.status == "published"),
            "delete_level" => text(p, "id")
                .split_once(':')
                .and_then(|(s, l)| Some((s.parse::<i64>().ok()?, l.parse::<i32>().ok()?)))
                .is_some_and(|(scale, level)| {
                    self.scales.iter().any(|s| {
                        s.id == scale
                            && s.status == "draft"
                            && s.levels.iter().any(|l| l.level == level)
                    })
                }),
            _ => true,
        }
    }
    fn scale_result<T>(&mut self, result: crate::Result<T>, success: &str) {
        match result {
            Ok(_) => self.flash.success(success),
            Err(SkillsError::DraftExists) => {
                self.flash.error("That scale already has an open draft.")
            }
            Err(SkillsError::Unauthorized) => self
                .flash
                .error("You no longer have permission to change this company's skills."),
            Err(
                e @ (SkillsError::InvalidLevels
                | SkillsError::ScaleInUse
                | SkillsError::CompanyArchived),
            ) => self.flash.error(&message(&e)),
            Err(_) => self.flash.error("The scale could not be changed."),
        }
    }
    fn render(&self) -> Result<String> {
        use askama::Template;
        let body = if self.company.selected.is_none() {
            render(
                &EmptyState::missing("No active company is available for skills.", "")
                    .id("skills-no-company"),
            )?
        } else if !self.available {
            render(
                &EmptyState::missing(
                    "Skills are unavailable while this company's workforce is not current.",
                    "",
                )
                .id("skills-unavailable"),
            )?
        } else {
            render(&CatalogLayout {
                categories: self.categories_section()?,
                scales: self.scales_section()?,
                skills: self.skills_section()?,
                profiles: self.profiles_section()?,
            })?
        };
        View {
            flash: self.flash.map(),
            page_id: "skills-page",
            title: "Skills",
            subtitle: self
                .company
                .selected
                .as_ref()
                .map(|c| {
                    format!(
                        "{} · Catalog, proficiency scales and requirement profiles",
                        c.name
                    )
                })
                .unwrap_or_default(),
            company: self.company.selected.as_ref().map(|_| self.company.field()),
            body,
            confirm: self
                .pending
                .as_ref()
                .map(|p| {
                    confirmation(
                        &self.company,
                        p.command.consequence(),
                        p.command.verb(),
                        &p.detail,
                    )
                })
                .transpose()?
                .unwrap_or(Html::EMPTY),
            profile: None,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    fn categories_section(&self) -> Result<Html> {
        let rows = self
            .categories
            .iter()
            .map(|c| {
                let mut r = row("skill-category", c.id);
                r.cells
                    .insert("name".into(), bilibi_ui::flex_table::Cell::text(&c.name));
                r.cells
                    .insert("code".into(), bilibi_ui::flex_table::Cell::text(&c.code));
                r.cells.insert(
                    "status".into(),
                    bilibi_ui::flex_table::Cell::text(if c.active { "Active" } else { "Inactive" }),
                );
                if self.can_manage {
                    r.slots.insert(
                        "actions".into(),
                        button(
                            "set_category_active",
                            if c.active { "Deactivate" } else { "Reactivate" },
                            c.id,
                        )?,
                    );
                }
                Ok(r)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut h = self.categories_table.render(
            "Categories",
            rows,
            "skill-categories-empty",
            "No skill categories have been added for this company.",
        )?;
        if self.can_manage {
            h.push(&self.drafts.form(
                "skill-category-form",
                "create_category",
                self.company.id.unwrap_or_default(),
                vec![
                        field("category[code]", "Category code")
                            .required()
                            .attr("maxlength", "80"),
                        field("category[name]", "Category name")
                            .required()
                            .attr("maxlength", "160"),
                        field("category[description]", "Category description")
                            .wrapper_class("mb-0 sm:col-span-2")
                            .attr("maxlength", "2000"),
                    ],
                "Add category",
            )?);
        }
        section("skill-categories-section", "Categories", h)
    }
    fn scales_section(&self) -> Result<Html> {
        let rows = self
            .scales
            .iter()
            .map(|s| {
                let mut r = row("skill-scale", s.id);
                r.cells
                    .insert("name".into(), bilibi_ui::flex_table::Cell::text(&s.name));
                r.cells
                    .insert("code".into(), bilibi_ui::flex_table::Cell::text(&s.code));
                r.cells.insert(
                    "version".into(),
                    bilibi_ui::flex_table::Cell::text(s.version.to_string()),
                );
                r.cells.insert(
                    "status".into(),
                    bilibi_ui::flex_table::Cell::text(&s.status),
                );
                let mut controls = Html::EMPTY;
                if self.can_manage {
                    for (event, label) in match s.status.as_str() {
                        "draft" => vec![("scale_publish", "Publish"), ("scale_discard", "Discard")],
                        "published" => vec![
                            ("scale_new_version", "New version"),
                            ("scale_retire", "Retire"),
                        ],
                        _ => vec![],
                    } {
                        controls.push(&button(event, label, s.id)?);
                    }
                }
                r.slots.insert("actions".into(), controls);
                Ok(r)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut h = self.scales_table.render(
            "Proficiency scales",
            rows,
            "skill-scales-empty",
            "No proficiency scales have been added. A requirement profile needs a published scale.",
        )?;
        for s in &self.scales {
            let rows = s
                .levels
                .iter()
                .map(|l| {
                    let mut r = row(&format!("skill-scale-{}-level", s.id), l.level);
                    r.cells.insert(
                        "level".into(),
                        bilibi_ui::flex_table::Cell::text(l.level.to_string()),
                    );
                    r.cells
                        .insert("name".into(), bilibi_ui::flex_table::Cell::text(&l.name));
                    r.cells.insert(
                        "anchor".into(),
                        bilibi_ui::flex_table::Cell::text(&l.anchor),
                    );
                    r.cells.insert(
                        "authority".into(),
                        bilibi_ui::flex_table::Cell::text(&l.authority),
                    );
                    if self.can_manage && s.status == "draft" {
                        r.slots.insert(
                            "actions".into(),
                            button("delete_level", "Remove", format!("{}:{}", s.id, l.level))?,
                        );
                    }
                    Ok(r)
                })
                .collect::<Result<Vec<_>>>()?;
            let mut levels = self.levels[&s.id].render(
                "Scale levels",
                rows,
                &format!("skill-scale-{}-empty", s.id),
                "No levels have been added.",
            )?;
            if self.can_manage && s.status == "draft" {
                levels.push(&self.drafts.form(
                    &format!("skill-level-form-{}", s.id),
                    "put_level",
                    self.company.id.unwrap_or_default(),
                    vec![
                            hidden("level[scale_id]", s.id),
                            field("level[level]", "Level number")
                                .kind(InputKind::Number)
                                .attr("min", "0")
                                .attr("max", "20")
                                .value(s.levels.len().to_string())
                                .required(),
                            field("level[name]", "Level name")
                                .required()
                                .attr("maxlength", "100"),
                            field("level[anchor]", "Observable behaviour")
                                .required()
                                .attr("maxlength", "2000"),
                            field("level[authority]", "Authority granted")
                                .required()
                                .attr("maxlength", "2000"),
                        ],
                    "Save level",
                )?);
            }
            h.push(&section(
                &format!("skill-scale-{}-levels", s.id),
                &format!("{} · v{} · {}", s.name, s.version, s.status),
                levels,
            )?);
        }
        if self.can_manage {
            h.push(&self.drafts.form(
                "skill-scale-form",
                "create_scale",
                self.company.id.unwrap_or_default(),
                vec![
                        field("scale[code]", "Scale code")
                            .required()
                            .attr("maxlength", "80"),
                        field("scale[name]", "Scale name")
                            .required()
                            .attr("maxlength", "160"),
                    ],
                "Add scale",
            )?);
        }
        section("skill-scales-section", "Proficiency scales", h)
    }
    fn skills_section(&self) -> Result<Html> {
        let rows = self
            .skills
            .iter()
            .map(|s| {
                let mut r = row("skill", s.id);
                r.cells
                    .insert("name".into(), bilibi_ui::flex_table::Cell::text(&s.name));
                r.cells
                    .insert("code".into(), bilibi_ui::flex_table::Cell::text(&s.code));
                r.cells.insert(
                    "category".into(),
                    bilibi_ui::flex_table::Cell::text(
                        self.categories
                            .iter()
                            .find(|c| c.id == s.category_id)
                            .map(|c| c.name.as_str())
                            .unwrap_or_default(),
                    ),
                );
                r.cells.insert(
                    "critical".into(),
                    bilibi_ui::flex_table::Cell::text(yes(s.critical)),
                );
                r.cells.insert(
                    "status".into(),
                    bilibi_ui::flex_table::Cell::text(if s.active { "Active" } else { "Inactive" }),
                );
                if self.can_manage {
                    r.slots.insert(
                        "actions".into(),
                        button(
                            "set_skill_active",
                            if s.active { "Deactivate" } else { "Reactivate" },
                            s.id,
                        )?,
                    );
                }
                Ok(r)
            })
            .collect::<Result<Vec<_>>>()?;
        let options: Vec<_> = self
            .categories
            .iter()
            .filter(|c| c.active)
            .map(|c| (c.name.clone(), c.id.to_string()))
            .collect();
        let empty = if options.is_empty() {
            "No skills have been added. Add an active category first."
        } else {
            "No skills have been added."
        };
        let mut h = self
            .skills_table
            .render("Skills", rows, "skills-empty", empty)?;
        if self.can_manage && !options.is_empty() {
            h.push(&self.drafts.form(
                "skill-form",
                "create_skill",
                self.company.id.unwrap_or_default(),
                vec![
                        select("skill[category_id]", "Skill category", options),
                        field("skill[code]", "Skill code")
                            .required()
                            .attr("maxlength", "80"),
                        field("skill[name]", "Skill name")
                            .required()
                            .attr("maxlength", "160"),
                        field("skill[definition]", "Skill definition")
                            .wrapper_class("mb-0 sm:col-span-2")
                            .required()
                            .attr("maxlength", "2000"),
                        field("skill[reassessment_months]", "Reassess every (months)")
                            .kind(InputKind::Number)
                            .attr("min", "1")
                            .attr("max", "120"),
                        field("skill[evidence_guide]", "Evidence guide")
                            .wrapper_class("mb-0 sm:col-span-2")
                            .attr("maxlength", "2000"),
                        field("skill[critical]", "Critical skill")
                            .kind(InputKind::Checkbox)
                            .value("true"),
                    ],
                "Add skill",
            )?);
        }
        section("skills-section", "Skills", h)
    }
    fn profiles_section(&self) -> Result<Html> {
        let c = self.company.id.unwrap_or_default();
        let rows = self
            .profiles
            .iter()
            .map(|p| {
                let mut r = row("skill-profile", p.id);
                r.slots.insert(
                    "name".into(),
                    render(&RecordLink {
                        workspace: self.framed,
                        kind: crate::MODULE,
                        record_id: &p.id.to_string(),
                        href: &format!("/people/skills/profiles/{}?company_id={c}", p.id),
                        class: "underline",
                        body: Html::text(&p.name),
                    })?,
                );
                r.cells
                    .insert("code".into(), bilibi_ui::flex_table::Cell::text(&p.code));
                r.cells.insert(
                    "version".into(),
                    bilibi_ui::flex_table::Cell::text(p.version.to_string()),
                );
                r.cells.insert(
                    "status".into(),
                    bilibi_ui::flex_table::Cell::text(&p.status),
                );
                r.cells.insert(
                    "scale".into(),
                    bilibi_ui::flex_table::Cell::text(scale_label(&self.scales, p.scale_id)),
                );
                r.cells.insert(
                    "effective".into(),
                    bilibi_ui::flex_table::Cell::text(
                        p.effective_from
                            .map(|d| {
                                format!(
                                    "{d} – {}",
                                    p.effective_to
                                        .map(|d| d.to_string())
                                        .unwrap_or("open".into())
                                )
                            })
                            .unwrap_or("—".into()),
                    ),
                );
                Ok(r)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut h = self.profiles_table.render(
            "Requirement profiles",
            rows,
            "skill-profiles-empty",
            "No requirement profiles have been added.",
        )?;
        let options = published_scales(&self.scales);
        if self.can_manage && !options.is_empty() {
            h.push(&self.drafts.form(
                "skill-profile-form",
                "create_profile",
                c,
                vec![
                        field("profile[code]", "Profile code")
                            .required()
                            .attr("maxlength", "80"),
                        field("profile[name]", "Profile name")
                            .required()
                            .attr("maxlength", "160"),
                        select("profile[scale_id]", "Profile scale", options),
                    ],
                "Add profile",
            )?);
        }
        section("skill-profiles-section", "Requirement profiles", h)
    }
}
impl LiveState<Scope, Error> for Catalog {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let changed = match request.input {
                LiveInput::Refresh => true,
                LiveInput::Params(q) => {
                    let company = params::positive_id(q.get("company_id").map(String::as_str));
                    if company != self.company.id {
                        self.drafts = Drafts::default();
                        self.pending = None;
                    }
                    self.company.id = company;
                    true
                }
                LiveInput::Event(p) => self.event(request.scope, p).await?,
            };
            self.load(request.scope, changed).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.company.query(),
                rows: self.skills.len() + self.profiles.len(),
                ..LiveBody::default()
            })
        })
    }
}

#[derive(askama::Template)]
#[template(path = "pk4_catalog_layout.html")]
struct CatalogLayout {
    categories: Html,
    scales: Html,
    skills: Html,
    profiles: Html,
}
