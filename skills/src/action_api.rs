//! Reassessment and development-action part of the one Skills facade.
//! Mirrors Bilimbi Skills' public delegates, with sealed scopes on every call.
use crate::{
    access, action_rows,
    actions::{self, Progress},
    Action, ActionEvent, ActionGroup, ActionInput, ActionPlan, ActionType, ActionView,
    AssessableEmployee, NewActionType, Result, Skills, SkillsError, Write, ACTIONS_APPROVE,
    ACTIONS_MANAGE, ACTIONS_UPDATE, ACTIONS_VIEW, CATALOG_MANAGE,
};
use bilibi_database::Error;
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use tokio_postgres::Client;

impl Skills {
    pub async fn list_action_types(&self, scope: &Scope, company: i64) -> Result<Vec<ActionType>> {
        self.current_company(scope, company).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        actions::types(&**client, scope.tenant(), company).await
    }

    pub async fn create_action_type(
        &self,
        scope: &Scope,
        company: i64,
        input: &NewActionType,
    ) -> Result<ActionType> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                CATALOG_MANAGE,
                "create_action_type",
            )
            .await?;
        let row = actions::create_type(&mut w, input).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    pub async fn set_action_type_active(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        active: bool,
    ) -> Result<ActionType> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                CATALOG_MANAGE,
                "set_action_type_active",
            )
            .await?;
        let old = actions::get_type(w.sql(), w.tenant, company, id).await?;
        if old.active == active {
            w.tx.rollback().await?;
            return Ok(old);
        }
        let row = actions::set_type_active(&mut w, old, active).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Managers may name any current company employee as owner, coordinator
    /// or provider. Names come through Workforce's Core directory summaries.
    pub async fn action_people(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<AssessableEmployee>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company, ACTIONS_MANAGE).await?;
        self.current_company(scope, company).await?;
        let mut rows = Vec::new();
        for employee in self
            .workforce
            .employees(scope, company)
            .await?
            .require_current()?
        {
            rows.push(AssessableEmployee {
                id: access::employee_id(&employee)?,
                name: format!("{} ({})", employee.display_name, employee.employee_number),
            });
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows)
    }

    /// Proposals snapshot a current finalized gap (or expired critical
    /// certificate), or the stated manual levels and reason. A company-wide
    /// key replays the same objective and type, exactly as Bilimbi does.
    pub async fn propose_action(
        &self,
        scope: &Scope,
        company: i64,
        input: &ActionInput,
    ) -> Result<Action> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                ACTIONS_MANAGE,
                "propose_action",
            )
            .await?;
        let (row, replay) = actions::propose(self, &mut w, scope, input).await?;
        if replay {
            w.tx.rollback().await?;
        } else {
            w.tx.commit().await?;
        }
        Ok(row)
    }

    pub async fn revise_action(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        plan: &ActionPlan,
    ) -> Result<Action> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, ACTIONS_MANAGE, "revise_action")
            .await?;
        let row = actions::revise(self, &mut w, scope, id, plan).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// The proposer and the employee cannot approve, including a formerly
    /// working employee or an account relinked since its scope was minted.
    pub async fn approve_action(&self, scope: &Scope, company: i64, id: i64) -> Result<Action> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                ACTIONS_APPROVE,
                "approve_action",
            )
            .await?;
        let subject = action_rows::get(w.sql(), w.tenant, company, id).await?;
        self.workforce
            .lock_employee_subject(&w.tx, scope, company, subject.employee_id)
            .await?;
        let old = action_rows::lock(&w, id).await?;
        if old.status != "proposed" {
            return Err(SkillsError::NotProposed);
        }
        access::independent(
            self,
            w.sql(),
            scope,
            company,
            old.employee_id,
            old.created_by_user_id,
        )
        .await?;
        let mut new = old.clone();
        new.status = if new
            .start_on
            .is_some_and(|d| d > crate::assessments::today())
        {
            "scheduled"
        } else {
            "not_started"
        }
        .into();
        new.approved_by_user_id = Some(scope.user());
        new.approved_at = Some(actions::now());
        let row = action_rows::save(&mut w, old, new, action_rows::Event::new("approved")).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    pub async fn start_action(&self, scope: &Scope, company: i64, id: i64) -> Result<Action> {
        self.progress_action(scope, company, id, Progress::Start)
            .await
    }
    pub async fn hold_action(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        reason: &str,
    ) -> Result<Action> {
        self.progress_action(scope, company, id, Progress::Hold(reason))
            .await
    }
    /// Records the finished intervention. A finalized assessment on or after
    /// its UTC completion day supplies the result; completion alone never does.
    pub async fn complete_action(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        evidence: &str,
        reassessment_due_on: Date,
    ) -> Result<Action> {
        self.progress_action(
            scope,
            company,
            id,
            Progress::Complete(evidence, reassessment_due_on),
        )
        .await
    }
    pub async fn cancel_action(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        reason: &str,
    ) -> Result<Action> {
        self.progress_action(scope, company, id, Progress::Cancel(reason))
            .await
    }
    async fn progress_action(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        command: Progress<'_>,
    ) -> Result<Action> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let action = match &command {
            Progress::Start => "start_action",
            Progress::Hold(_) => "hold_action",
            Progress::Complete(..) => "complete_action",
            Progress::Cancel(_) => "cancel_action",
        };
        let mut w = self
            .begin_actions(&mut client, scope, company, action)
            .await?;
        let own = actions::progress_owner(self, &w, scope).await?;
        let old = action_rows::lock(&w, id).await?;
        if own.is_some_and(|id| id != old.owner_employee_id) {
            return Err(SkillsError::NotFound);
        }
        let row = actions::progress(&mut w, old, command).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Explicitly links finalized work that existed before completion of an
    /// intervention; finalizations after completion close matching work
    /// automatically. The assessment must pass the same subject/date rules.
    pub async fn link_action_reassessment(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        assessment: i64,
    ) -> Result<Action> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin_actions(&mut client, scope, company, "link_action_reassessment")
            .await?;
        let own = actions::progress_owner(self, &w, scope).await?;
        let old = action_rows::lock(&w, id).await?;
        if own.is_some_and(|id| id != old.owner_employee_id) {
            return Err(SkillsError::NotFound);
        }
        let a = crate::assessments::get(w.sql(), w.tenant, company, assessment).await?;
        let row = actions::close(&mut w, old, &a).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Comments are append-only events, including on terminal actions.
    pub async fn comment_action(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        comment: &str,
        evidence: Option<&str>,
    ) -> Result<Action> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin_actions(&mut client, scope, company, "comment_action")
            .await?;
        let own = actions::progress_owner(self, &w, scope).await?;
        let row = action_rows::lock(&w, id).await?;
        if own.is_some_and(|id| id != row.owner_employee_id) {
            return Err(SkillsError::NotFound);
        }
        let comment = actions::required(comment, SkillsError::CommentRequired)?;
        let evidence = actions::text(evidence.unwrap_or_default(), 2000, false)?;
        action_rows::comment(&mut w, &row, comment, evidence).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Visible work: all for action managers; otherwise owned actions and
    /// actions of employees in the viewer's supervisor chain. At most 300,
    /// mandatory and highest priority first while open, newest first closed.
    pub async fn list_actions(
        &self,
        scope: &Scope,
        company: i64,
        group: ActionGroup,
    ) -> Result<Vec<ActionView>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company, ACTIONS_VIEW).await?;
        self.current_company(scope, company).await?;
        let reach = access::reach_with(self, &**client, scope, company, ACTIONS_MANAGE).await?;
        let own = self
            .workforce
            .linked_employee(&**client, scope, company)
            .await?;
        let rows = actions::list(
            &**client,
            scope.tenant(),
            company,
            group,
            &reach,
            own,
            false,
        )
        .await?;
        actions::present(self, &**client, scope, company, rows).await
    }

    pub async fn owned_actions(
        &self,
        scope: &Scope,
        company: i64,
        group: ActionGroup,
    ) -> Result<Vec<ActionView>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company, ACTIONS_UPDATE).await?;
        self.current_company(scope, company).await?;
        let me = self
            .workforce
            .authorize_self(scope, company, ACTIONS_UPDATE)
            .await?;
        let rows = actions::list(
            &**client,
            scope.tenant(),
            company,
            group,
            &access::Reach::Company,
            Some(me.employee_id),
            true,
        )
        .await?;
        actions::present(self, &**client, scope, company, rows).await
    }

    pub async fn action_events(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
    ) -> Result<Vec<ActionEvent>> {
        let client = self.db.get().await.map_err(Error::from)?;
        access::authorize_any(
            &**client,
            scope,
            company,
            &[ACTIONS_VIEW, ACTIONS_MANAGE, ACTIONS_UPDATE],
        )
        .await?;
        self.current_company(scope, company).await?;
        let row = action_rows::get(&**client, scope.tenant(), company, id).await?;
        let own = self
            .workforce
            .linked_employee(&**client, scope, company)
            .await?;
        let manager = authorization::allowed(&**client, scope, company, ACTIONS_MANAGE).await;
        let viewer = authorization::allowed(&**client, scope, company, ACTIONS_VIEW).await;
        let reach = access::reach_with(self, &**client, scope, company, ACTIONS_MANAGE).await?;
        if !manager
            && own != Some(row.owner_employee_id)
            && !(viewer && reach.contains(row.employee_id))
        {
            return Err(SkillsError::NotFound);
        }
        client.query(&format!("SELECT {} FROM people_skill_action_events WHERE tenant_id=$1 AND company_id=$2 AND action_id=$3 ORDER BY id",action_rows::event_row_columns()), &[&scope.tenant(),&company,&id]).await.map_err(Error::from)?.iter().map(action_rows::event_row).collect::<bilibi_database::Result<Vec<_>>>().map_err(Into::into)
    }

    async fn begin_actions<'a>(
        &self,
        client: &'a mut Client,
        scope: &Scope,
        company: i64,
        action: &str,
    ) -> Result<Write<'a>> {
        let cap = if authorization::allowed(&*client, scope, company, ACTIONS_MANAGE).await {
            ACTIONS_MANAGE
        } else {
            ACTIONS_UPDATE
        };
        self.begin(client, scope, company, cap, action).await
    }
}
