//! Mirrors Bilimbi Skills.Web.ProfileLive: a requirement profile version.
use super::*;
use crate::{
    Direction, ItemInput, ProfileChanges, ProfileDetail, Scale, Skill, Target, CATALOG_MANAGE,
    PROFILES_PUBLISH,
};
use bilibi_ui::page::{LiveBody, LiveFuture, LiveInput, LiveRequest};
use jiff::civil::Date;
use rust_decimal::Decimal;

enum Command {
    Publish(Date),
    Retire(Date),
    Discard,
    NewVersion,
    MoveScale(i64),
    RemoveItem(i64),
    RemoveSelector(i64),
}
impl Command {
    fn cap(&self) -> &'static str {
        match self {
            Self::Publish(_) | Self::Retire(_) => PROFILES_PUBLISH,
            _ => CATALOG_MANAGE,
        }
    }
    fn verb(&self) -> &'static str {
        match self {
            Self::Publish(_) => "Publish",
            Self::Retire(_) => "Retire",
            Self::Discard => "Discard draft",
            Self::NewVersion => "Draft a new version",
            Self::MoveScale(_) => "Change scale",
            Self::RemoveItem(_) | Self::RemoveSelector(_) => "Remove",
        }
    }
}
struct Pending {
    company: i64,
    profile: i64,
    command: Command,
    detail: String,
}
pub(super) struct ProfilePage {
    core: Arc<Core>,
    company: Company,
    id: Option<i64>,
    profile: Option<ProfileDetail>,
    skills: Vec<Skill>,
    scales: Vec<Scale>,
    positions: Vec<crate::PositionOption>,
    can_manage: bool,
    can_publish: bool,
    items: Table,
    selectors: Table,
    drafts: Drafts,
    flash: Flash,
    pending: Option<Pending>,
    redirect: Option<String>,
}
impl ProfilePage {
    pub(super) fn new(
        core: Arc<Core>,
        query: &HashMap<String, String>,
        id: Option<i64>,
        _framed: bool,
    ) -> Result<Self> {
        Ok(Self {
            core,
            company: Company::new(query),
            id,
            profile: None,
            skills: vec![],
            scales: vec![],
            positions: vec![],
            can_manage: false,
            can_publish: false,
            items: Table::new(
                "skill-profile-items",
                "items_grid",
                &[
                    ("sequence", "#"),
                    ("skill", "Skill"),
                    ("level", "Required level"),
                    ("criticality", "Criticality"),
                    ("weight", "Weight %"),
                    ("mandatory", "Mandatory"),
                    ("actions", "Actions"),
                ],
            )?,
            selectors: Table::new(
                "skill-profile-selectors",
                "selectors_grid",
                &[("target", "Target"), ("actions", "Actions")],
            )?,
            drafts: Drafts::default(),
            flash: Flash::default(),
            pending: None,
            redirect: None,
        })
    }
    async fn load(&mut self, scope: &Scope, changed: bool) -> Result<()> {
        self.company.load(&self.core, scope, false).await?;
        self.profile = None;
        self.skills.clear();
        self.scales.clear();
        self.positions.clear();
        self.can_manage = false;
        self.can_publish = false;
        if let (Some(c), Some(id)) = (self.company.id, self.id) {
            let s = &self.core.skills;
            if let (Ok(profile), Ok(skills), Ok(scales)) = (
                s.get_profile(scope, c, id).await,
                s.list_skills(scope, c).await,
                s.list_scales(scope, c).await,
            ) {
                self.profile = Some(profile);
                self.skills = skills;
                self.scales = scales;
                self.positions = s.position_options(scope, c).await.unwrap_or_default();
                self.can_manage = self
                    .company
                    .allowed(&self.core, scope, CATALOG_MANAGE)
                    .await;
                self.can_publish = self
                    .company
                    .allowed(&self.core, scope, PROFILES_PUBLISH)
                    .await;
            }
        }
        if self.profile.is_none() {
            self.pending = None;
        }
        if changed {
            if let Some(p) = &self.profile {
                self.items.load(&p.items, |r| r.id.to_string());
                self.selectors.load(&p.selectors, |r| r.id.to_string());
            }
        }
        Ok(())
    }
    async fn authorize(&mut self, scope: &Scope, cap: &str) -> bool {
        if self.company.allowed(&self.core, scope, cap).await {
            true
        } else {
            self.pending = None;
            self.flash.error(if cap == PROFILES_PUBLISH {
                PUBLISH_FORBIDDEN
            } else {
                FORBIDDEN
            });
            false
        }
    }
    fn decision(&mut self, command: Command, detail: String) {
        if let (Some(company), Some(profile)) = (self.company.id, self.id) {
            self.pending = Some(Pending {
                company,
                profile,
                command,
                detail,
            });
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
            "items_grid" => {
                self.items.event(p);
                return Ok(false);
            }
            "selectors_grid" => {
                self.selectors.event(p);
                return Ok(false);
            }
            "put_item" | "remove_item" | "move_item_up" | "move_item_down" | "add_selector"
            | "remove_selector" | "new_version" | "discard" | "move_scale" | "publish"
            | "retire" | "confirm" => {}
            _ => return Ok(false),
        }
        let cap = if event == "confirm" {
            self.pending
                .as_ref()
                .map(|p| p.command.cap())
                .unwrap_or(CATALOG_MANAGE)
        } else if ["publish", "retire"].contains(&event) {
            PROFILES_PUBLISH
        } else {
            CATALOG_MANAGE
        };
        if !self.authorize(scope, cap).await {
            return Ok(true);
        }
        let (Some(c), Some(id), Some(detail)) = (self.company.id, self.id, self.profile.as_ref())
        else {
            return Ok(false);
        };
        let label = format!(
            "{} ({}) v{}",
            detail.profile.name, detail.profile.code, detail.profile.version
        );
        let s = &self.core.skills;
        match event {
            "put_item" => {
                let f = "skill-profile-item-form";
                self.drafts.remember(f, p);
                let result = match text(p, "item[weight_percent]").trim().parse::<Decimal>() {
                    Ok(weight) => {
                        s.put_item(
                            scope,
                            c,
                            id,
                            &ItemInput {
                                skill_id: super::id(p, "item[skill_id]").unwrap_or_default(),
                                required_level: integer(p, "item[required_level]"),
                                criticality: text(p, "item[criticality]").into(),
                                weight_percent: weight,
                                mandatory: text(p, "item[mandatory]") == "true",
                                evidence_standard: optional(text(p, "item[evidence_standard]")),
                            },
                        )
                        .await
                    }
                    Err(_) => Err(SkillsError::Invalid(vec![crate::FieldError::new(
                        "weight_percent",
                        "must be a number from 0 to 100 with at most two decimal places",
                    )])),
                };
                self.drafts.result(
                    f,
                    &mut self.flash,
                    result,
                    "Requirement saved.",
                    "The requirement profile could not be changed.",
                );
            }
            "add_selector" => {
                let f = "skill-profile-selector-form";
                self.drafts.remember(f, p);
                let target = if text(p, "selector[target]") == "company" {
                    Target::Company
                } else {
                    Target::Position(super::id(p, "selector[target]").unwrap_or_default())
                };
                self.drafts.result(
                    f,
                    &mut self.flash,
                    s.add_selector(scope, c, id, target).await,
                    "Target added.",
                    "The requirement profile could not be changed.",
                );
            }
            "move_item_up" | "move_item_down" => {
                if let Some(item) = detail
                    .items
                    .iter()
                    .find(|r| Some(r.id) == super::id(p, "id"))
                {
                    let direction = if event == "move_item_up" {
                        Direction::Up
                    } else {
                        Direction::Down
                    };
                    let result = s.move_item(scope, c, id, item.id, direction).await;
                    self.respond(result, "Requirement moved.");
                }
            }
            "remove_item" => {
                if let Some(item) = detail
                    .items
                    .iter()
                    .find(|r| Some(r.id) == super::id(p, "id"))
                {
                    let name = self
                        .skills
                        .iter()
                        .find(|s| s.id == item.skill_id)
                        .map(|s| s.name.as_str())
                        .unwrap_or_default();
                    self.decision(Command::RemoveItem(item.id), format!("{label} · {name}"));
                }
                return Ok(false);
            }
            "remove_selector" => {
                if let Some(target) = detail
                    .selectors
                    .iter()
                    .find(|r| Some(r.id) == super::id(p, "id"))
                {
                    self.decision(
                        Command::RemoveSelector(target.id),
                        format!("{label} · {}", self.target_label(target)),
                    );
                }
                return Ok(false);
            }
            "new_version" => {
                self.decision(Command::NewVersion, label);
                return Ok(false);
            }
            "discard" => {
                self.decision(Command::Discard, label);
                return Ok(false);
            }
            "move_scale" => {
                self.drafts.remember("skill-profile-scale-form", p);
                if let Some(scale) = self
                    .scales
                    .iter()
                    .find(|r| Some(r.id) == super::id(p, "scale_id") && r.status == "published")
                {
                    self.decision(
                        Command::MoveScale(scale.id),
                        format!(
                            "{label} · {} ({}) v{}",
                            scale.name, scale.code, scale.version
                        ),
                    );
                }
                return Ok(false);
            }
            "publish" | "retire" => {
                let (key, form) = if event == "publish" {
                    ("effective_from", "skill-profile-publish-form")
                } else {
                    ("effective_to", "skill-profile-retire-form")
                };
                self.drafts.remember(form, p);
                match text(p, key).parse::<Date>() {
                    Ok(date) => self.decision(
                        if event == "publish" {
                            Command::Publish(date)
                        } else {
                            Command::Retire(date)
                        },
                        format!(
                            "{label} · {} {date}",
                            if event == "publish" {
                                "Effective from"
                            } else {
                                "Last effective day"
                            }
                        ),
                    ),
                    Err(_) => self
                        .flash
                        .error("Choose a valid date on or after the version's start."),
                }
                return Ok(false);
            }
            "confirm" => {
                if let Some(pending) = self
                    .pending
                    .take()
                    .filter(|p| p.company == c && p.profile == id)
                {
                    match pending.command {
                        Command::Publish(date) => {
                            let r = s.publish_profile(scope, c, id, date).await;
                            self.respond(r, "Profile published.");
                        }
                        Command::Retire(date) => {
                            let r = s.retire_profile(scope, c, id, date).await;
                            self.respond(r, "Profile retired.");
                        }
                        Command::Discard => {
                            let r = s.discard_profile(scope, c, id).await;
                            if self.respond(r, "Draft discarded.") {
                                self.redirect = Some(format!("/people/skills?company_id={c}"));
                            }
                        }
                        Command::NewVersion => match s.new_profile_version(scope, c, id).await {
                            Ok(new) => {
                                self.id = Some(new.profile.profile.id);
                                self.drafts = Drafts::default();
                                self.flash.success("New draft version added.");
                                if !new.missing_levels.is_empty() {
                                    self.flash.error(&format!("The current scale version lacks required levels {}; the draft keeps its previous scale until those requirements change.",new.missing_levels.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")));
                                }
                            }
                            Err(e) => {
                                self.respond::<()>(Err(e), "");
                            }
                        },
                        Command::MoveScale(scale_id) => {
                            let r = s
                                .update_profile(
                                    scope,
                                    c,
                                    id,
                                    &ProfileChanges {
                                        name: None,
                                        scale_id: Some(scale_id),
                                    },
                                )
                                .await;
                            self.respond(r, "Scale changed.");
                        }
                        Command::RemoveItem(item) => {
                            let r = s.remove_item(scope, c, id, item).await;
                            self.respond(r, "Requirement removed.");
                        }
                        Command::RemoveSelector(selector) => {
                            let r = s.remove_selector(scope, c, id, selector).await;
                            self.respond(r, "Target removed.");
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    }
    fn respond<T>(&mut self, result: crate::Result<T>, success: &str) -> bool {
        match result {
            Ok(_) => {
                self.flash.success(success);
                true
            }
            Err(SkillsError::Unauthorized) => {
                self.flash.error(PUBLISH_FORBIDDEN);
                false
            }
            Err(SkillsError::DraftExists) => {
                self.flash.error("This profile already has an open draft.");
                false
            }
            Err(e) => {
                self.flash.error(&match e {
                    SkillsError::Failed(_) | SkillsError::Invalid(_) => {
                        "The requirement profile could not be changed.".into()
                    }
                    _ => message(&e),
                });
                false
            }
        }
    }
    fn target_label(&self, target: &crate::Selector) -> String {
        if target.selector_type == "company" {
            "Whole company".into()
        } else {
            self.positions
                .iter()
                .find(|p| Some(p.id) == target.position_id)
                .map(|p| {
                    format!(
                        "{} ({})",
                        if p.title.is_empty() {
                            &p.code
                        } else {
                            &p.title
                        },
                        p.code
                    )
                })
                .unwrap_or_else(|| format!("Position #{}", target.position_id.unwrap_or_default()))
        }
    }
    fn render(&self) -> Result<String> {
        use askama::Template;
        let mut body = Html::EMPTY;
        let mut title = "Requirement profile";
        let mut subtitle = String::new();
        if let Some(p) = &self.profile {
            title = &p.profile.name;
            subtitle = format!(
                "{} · {} · version {} · {}",
                self.company
                    .selected
                    .as_ref()
                    .map(|c| c.name.as_str())
                    .unwrap_or_default(),
                p.profile.code,
                p.profile.version,
                p.profile.status
            );
            if let Some(from) = p.profile.effective_from {
                subtitle.push_str(&format!(
                    " · effective {from} – {}",
                    p.profile
                        .effective_to
                        .map(|d| d.to_string())
                        .unwrap_or("open".into())
                ));
            }
            body.push(&render(&bilibi_ui::BackLink {
                href: &format!(
                    "/people/skills?company_id={}",
                    self.company.id.unwrap_or_default()
                ),
                id: "skill-profile-back",
                title: "Back to skills",
                class: "",
            })?);
            if let Some(scale) = self.scales.iter().find(|s| s.id == p.profile.scale_id) {
                body.push(&render(&bilibi_ui::Facts {
                    id: "skill-profile-scale",
                    items: vec![bilibi_ui::Fact {
                        title: "Scale",
                        value: Html::text(&format!(
                            "{} v{} ({})",
                            scale.name, scale.version, scale.status
                        )),
                        ..bilibi_ui::Fact::default()
                    }],
                })?);
            }
            if self.can_manage
                && p.profile.status == "draft"
                && self.scales.iter().any(|s| s.status == "published")
            {
                body.push(&self.drafts.form(
                    "skill-profile-scale-form",
                    "move_scale",
                    p.profile.id,
                    vec![select("scale_id", "Scale", published_scales(&self.scales))
                            .value(p.profile.scale_id.to_string())],
                    "Change scale",
                )?);
            }
            body.push(&self.items_section(p)?);
            body.push(&self.targets_section(p)?);
            if p.profile.status != "retired" {
                body.push(&self.version_section(p)?);
            }
        } else {
            body = render(
                &EmptyState::missing("This requirement profile is not available.", "")
                    .id("skill-profile-missing"),
            )?;
        }
        View {
            flash: self.flash.map(),
            page_id: "skill-profile-page",
            title,
            subtitle,
            company: None,
            body,
            confirm: self
                .pending
                .as_ref()
                .map(|p| confirmation(&self.company, p.command.verb(), &p.detail))
                .transpose()?
                .unwrap_or(Html::EMPTY),
            profile: self.profile.as_ref().map(|p| p.profile.id.to_string()),
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    fn items_section(&self, p: &ProfileDetail) -> Result<Html> {
        let scale = self.scales.iter().find(|s| s.id == p.profile.scale_id);
        let editable = self.can_manage && p.profile.status == "draft";
        let rows = p
            .items
            .iter()
            .map(|item| {
                let mut r = row("skill-profile-item", item.id);
                r.cells.insert(
                    "sequence".into(),
                    bilibi_ui::flex_table::Cell::text(item.sequence.to_string()),
                );
                r.cells.insert(
                    "skill".into(),
                    bilibi_ui::flex_table::Cell::text(
                        self.skills
                            .iter()
                            .find(|s| s.id == item.skill_id)
                            .map(|s| s.name.as_str())
                            .unwrap_or_default(),
                    ),
                );
                r.cells.insert(
                    "level".into(),
                    bilibi_ui::flex_table::Cell::text(
                        scale
                            .and_then(|s| s.levels.iter().find(|l| l.level == item.required_level))
                            .map(|l| format!("{} · {}", l.level, l.name))
                            .unwrap_or_else(|| item.required_level.to_string()),
                    ),
                );
                r.cells.insert(
                    "criticality".into(),
                    bilibi_ui::flex_table::Cell::text(&item.criticality),
                );
                r.cells.insert(
                    "weight".into(),
                    bilibi_ui::flex_table::Cell::text(item.weight_percent.to_string()),
                );
                r.cells.insert(
                    "mandatory".into(),
                    bilibi_ui::flex_table::Cell::text(yes(item.mandatory)),
                );
                if editable {
                    let mut controls = button("move_item_up", "Up", item.id)?;
                    controls.push(&button("move_item_down", "Down", item.id)?);
                    controls.push(&button("remove_item", "Remove", item.id)?);
                    r.slots.insert("actions".into(), controls);
                }
                Ok(r)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut h = self.items.render(
            "Skill requirements",
            rows,
            "skill-profile-items-empty",
            "No skill requirements have been added.",
        )?;
        let total: Decimal = p.items.iter().map(|i| i.weight_percent).sum();
        h.push(&render(&bilibi_ui::Facts {
            id: "skill-profile-total",
            items: vec![bilibi_ui::Fact {
                title: "Total weight",
                value: Html::text(&format!("{total}% of 100%.")),
                ..bilibi_ui::Fact::default()
            }],
        })?);
        if let Some(scale) = scale.filter(|_| editable) {
            h.push(&self.drafts.form(
                "skill-profile-item-form",
                "put_item",
                p.profile.id,
                vec![
                        select(
                            "item[skill_id]",
                            "Skill",
                            self.skills
                                .iter()
                                .filter(|s| s.active)
                                .map(|s| (s.name.clone(), s.id.to_string()))
                                .collect(),
                        ),
                        select(
                            "item[required_level]",
                            "Required level",
                            scale
                                .levels
                                .iter()
                                .map(|l| (format!("{} · {}", l.level, l.name), l.level.to_string()))
                                .collect(),
                        ),
                        select(
                            "item[criticality]",
                            "Criticality",
                            crate::CRITICALITIES
                                .iter()
                                .map(|v| (v.to_string(), v.to_string()))
                                .collect(),
                        ),
                        field("item[weight_percent]", "Weight percent")
                            .attr("inputmode", "decimal")
                            .required(),
                        field("item[evidence_standard]", "Evidence standard")
                            .attr("maxlength", "2000"),
                        field("item[mandatory]", "Mandatory")
                            .kind(InputKind::Checkbox)
                            .value("true"),
                    ],
                "Save requirement",
            )?);
        }
        section("skill-profile-items-section", "Skill requirements", h)
    }
    fn targets_section(&self, p: &ProfileDetail) -> Result<Html> {
        let editable = self.can_manage && p.profile.status == "draft";
        let rows = p
            .selectors
            .iter()
            .map(|target| {
                let mut r = row("skill-profile-selector", target.id);
                r.cells.insert(
                    "target".into(),
                    bilibi_ui::flex_table::Cell::text(self.target_label(target)),
                );
                if editable {
                    r.slots.insert(
                        "actions".into(),
                        button("remove_selector", "Remove", target.id)?,
                    );
                }
                Ok(r)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut h = self.selectors.render(
            "Applies to",
            rows,
            "skill-profile-selectors-empty",
            "No target has been chosen.",
        )?;
        if editable {
            let mut options = vec![("Whole company".into(), "company".into())];
            options.extend(self.positions.iter().map(|p| {
                (
                    format!(
                        "{} ({})",
                        if p.title.is_empty() {
                            &p.code
                        } else {
                            &p.title
                        },
                        p.code
                    ),
                    p.id.to_string(),
                )
            }));
            h.push(&self.drafts.form(
                "skill-profile-selector-form",
                "add_selector",
                p.profile.id,
                vec![select("selector[target]", "Target", options)],
                "Add target",
            )?);
        }
        section("skill-profile-selectors-section", "Applies to", h)
    }
    fn version_section(&self, p: &ProfileDetail) -> Result<Html> {
        let mut h = Html::EMPTY;
        if self.can_publish {
            if p.profile.status == "draft" {
                h.push(&self.drafts.form(
                    "skill-profile-publish-form",
                    "publish",
                    p.profile.id,
                    vec![field("effective_from","Effective from").kind(InputKind::Date).required()],
                    "Publish",
                )?);
            }
            if p.profile.status == "published" {
                h.push(&self.drafts.form(
                    "skill-profile-retire-form",
                    "retire",
                    p.profile.id,
                    vec![field("effective_to", "Last effective day")
                            .kind(InputKind::Date)
                            .required()],
                    "Retire",
                )?);
            }
        }
        if self.can_manage {
            let (event, label) = if p.profile.status == "published" {
                ("new_version", "Draft a new version")
            } else {
                ("discard", "Discard draft")
            };
            h.push(&render(&Button {
                id: if event == "new_version" {
                    "skill-profile-new-version"
                } else {
                    "skill-profile-discard"
                },
                event,
                body: Html::text(label),
                ..Button::default()
            })?);
        }
        if !self.can_publish && p.profile.status == "draft" {
            h.push(&render(
                &EmptyState::missing(
                    "Publishing needs the requirement publisher capability for this company.",
                    "",
                )
                .id("skill-profile-publisher-required"),
            )?);
        }
        section("skill-profile-version", "Version", h)
    }
}
impl LiveState<Scope, Error> for ProfilePage {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let previous = self.id;
            let changed = match request.input {
                LiveInput::Refresh => true,
                LiveInput::Params(q) => {
                    let company = params::positive_id(q.get("company_id").map(String::as_str));
                    let id = params::positive_id(q.get("id").map(String::as_str)).or(self.id);
                    if company != self.company.id || id != self.id {
                        self.drafts = Drafts::default();
                        self.pending = None;
                    }
                    self.company.id = company;
                    self.id = id;
                    true
                }
                LiveInput::Event(p) => self.event(request.scope, p).await?,
            };
            self.load(request.scope, changed).await?;
            let redirect = self.redirect.take();
            let flash = if redirect.is_some() {
                self.flash.0.clone()
            } else {
                None
            };
            Ok(LiveBody {
                html: self.render()?,
                query: self.company.query(),
                path: if self.id != previous {
                    self.id.map(|id| format!("/people/skills/profiles/{id}"))
                } else {
                    None
                },
                redirect,
                flash,
                rows: self
                    .profile
                    .as_ref()
                    .map(|p| p.items.len())
                    .unwrap_or_default(),
                ..LiveBody::default()
            })
        })
    }
}
