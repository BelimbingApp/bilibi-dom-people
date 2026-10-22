//! Bilimbi Training.Participation: append-only facts and import-key replay.
//! Public operations live on Training, the module's single facade.
use crate::{
    catalog, writable, FieldError, Result, Session, Training, TrainingError, RECORDS_MANAGE,
    RECORDS_VIEW,
};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use serde::Serialize;
use serde_json::json;
use tokio_postgres::Row;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttendanceStatus {
    Confirmed,
    Absent,
}
impl AttendanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Absent => "absent",
        }
    }
}
#[derive(Clone, Debug)]
pub struct NewAttendance {
    pub session_id: i64,
    /// The Workforce employment ID, never a Person or account ID.
    pub employee_id: i64,
    pub status: AttendanceStatus,
    pub reason: String,
    pub import_key: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AttendanceFact {
    pub id: i64,
    pub platform_company_id: i64,
    pub session_id: i64,
    pub employee_id: i64,
    pub revision: i32,
    pub status: AttendanceStatus,
    pub reason: String,
    pub import_key: String,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub inserted_at: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub id: i64,
    pub platform_company_id: i64,
    pub fact_id: i64,
    pub artifact_id: String,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub inserted_at: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmployeeOption {
    pub id: i64,
    pub name: String,
}

const FACT_COLUMNS: &str = "id, company_id, session_id, employee_id, revision, status, reason, import_key, actor_user_id, impersonator_id, to_char(inserted_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS inserted_at";
pub(crate) const EVIDENCE_COLUMNS: &str = "id, company_id, fact_id, artifact_id::text, actor_user_id, impersonator_id, to_char(inserted_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS inserted_at";
fn fact(row: &Row) -> AttendanceFact {
    AttendanceFact {
        id: row.get("id"),
        platform_company_id: row.get("company_id"),
        session_id: row.get("session_id"),
        employee_id: row.get("employee_id"),
        revision: row.get("revision"),
        status: if row.get::<_, String>("status") == "confirmed" {
            AttendanceStatus::Confirmed
        } else {
            AttendanceStatus::Absent
        },
        reason: row.get("reason"),
        import_key: row.get("import_key"),
        actor_user_id: row.get("actor_user_id"),
        impersonator_id: row.get("impersonator_id"),
        inserted_at: row.get("inserted_at"),
    }
}
pub(crate) fn evidence(row: &Row) -> Evidence {
    Evidence {
        id: row.get("id"),
        platform_company_id: row.get("company_id"),
        fact_id: row.get("fact_id"),
        artifact_id: row.get("artifact_id"),
        actor_user_id: row.get("actor_user_id"),
        impersonator_id: row.get("impersonator_id"),
        inserted_at: row.get("inserted_at"),
    }
}

impl Training {
    /// Bilimbi Participation.record. A key replays the original fact (even
    /// after later revisions); changed attendance with the same key refuses.
    pub async fn record(
        &self,
        scope: &Scope,
        company_id: i64,
        input: &NewAttendance,
    ) -> Result<AttendanceFact> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RECORDS_MANAGE)
            .await?;
        if input.session_id <= 0 || input.employee_id <= 0 {
            return Err(TrainingError::InvalidRecord);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.record"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let actor = self
            .authorize(tx.sql(), scope, company_id, RECORDS_MANAGE)
            .await?;
        // Lock before the replay, revision and occupancy queries. Company
        // serialization also makes the company-wide import key race safe.
        let session = tx.sql().query_opt("SELECT capacity FROM people_training_session_runs WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE", &[&scope.tenant(), &company_id, &input.session_id]).await.map_err(Error::from)?.ok_or(TrainingError::SessionUnavailable)?;
        let replay = tx.sql().query_opt(&format!("SELECT {FACT_COLUMNS} FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2 AND import_key=$3"), &[&scope.tenant(), &company_id, &input.import_key]).await.map_err(Error::from)?;
        if let Some(row) = replay {
            let previous = fact(&row);
            return if previous.session_id == input.session_id
                && previous.employee_id == input.employee_id
                && previous.status == input.status
                && previous.reason == input.reason.trim()
            {
                Ok(previous)
            } else {
                Err(TrainingError::ImportConflict)
            };
        }
        let employees = self
            .workforce
            .working_employee_ids_on(tx.sql(), scope, company_id, &[input.employee_id])
            .await
            .map_err(|_| TrainingError::EmployeeUnavailable)?
            .require_current()
            .map_err(|_| TrainingError::EmployeeUnavailable)?;
        if employees.is_empty() {
            return Err(TrainingError::EmployeeUnavailable);
        }
        let reason = input.reason.trim();
        let mut errors = Vec::new();
        for (field, value, maximum) in [
            ("reason", reason, 2000),
            ("import_key", input.import_key.as_str(), 160),
        ] {
            if value.trim().is_empty() {
                errors.push(FieldError::new(field, "can't be blank"));
            } else if value.chars().count() > maximum {
                errors.push(FieldError::new(
                    field,
                    format!("should be at most {maximum} character(s)"),
                ));
            }
        }
        if !errors.is_empty() {
            return Err(TrainingError::Invalid(errors));
        }
        let previous = tx.sql().query_opt("SELECT revision, status FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2 AND session_id=$3 AND employee_id=$4 ORDER BY revision DESC LIMIT 1", &[&scope.tenant(), &company_id, &input.session_id, &input.employee_id]).await.map_err(Error::from)?;
        let revision = previous
            .as_ref()
            .map_or(0, |row| row.get::<_, i32>("revision"))
            .checked_add(1)
            .ok_or(TrainingError::InvalidRecord)?;
        let already_confirmed = previous
            .as_ref()
            .is_some_and(|row| row.get::<_, String>("status") == "confirmed");
        if input.status == AttendanceStatus::Confirmed && !already_confirmed {
            let occupied: i64 = tx.sql().query_one("SELECT count(*) FROM (SELECT DISTINCT ON (employee_id) status FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2 AND session_id=$3 ORDER BY employee_id, revision DESC) latest WHERE status='confirmed'", &[&scope.tenant(), &company_id, &input.session_id]).await.map_err(Error::from)?.get(0);
            if occupied >= i64::from(session.get::<_, i32>("capacity")) {
                return Err(TrainingError::AttendanceCapacityExceeded);
            }
        }
        let saved = insert_fact(&mut tx, scope, company_id, actor, input, reason, revision).await?;
        tx.commit().await?;
        Ok(saved)
    }

    /// At most 500 sessions, newest first, for the training records workspace.
    pub async fn sessions(&self, scope: &Scope, company_id: i64) -> Result<Vec<Session>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RECORDS_VIEW)
            .await?;
        catalog::record_sessions(&**client, scope.tenant(), company_id).await
    }
    /// Revisions ordered by employee then newest revision. A missing session
    /// has an empty history, matching Bilimbi's scoped query.
    pub async fn history(
        &self,
        scope: &Scope,
        company_id: i64,
        session_id: i64,
    ) -> Result<Vec<AttendanceFact>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RECORDS_VIEW)
            .await?;
        if session_id <= 0 {
            return Err(TrainingError::NotFound);
        }
        Ok(client.query(&format!("SELECT {FACT_COLUMNS} FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2 AND session_id=$3 ORDER BY employee_id, revision DESC"), &[&scope.tenant(), &company_id, &session_id]).await.map_err(Error::from)?.iter().map(fact).collect())
    }
    /// Current employment choices; names come from the Workforce directory.
    pub async fn participation_employees(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<EmployeeOption>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RECORDS_MANAGE)
            .await?;
        // Workforce opens its own connection. Release this read's connection
        // first so concurrent pickers cannot exhaust the pool waiting for a second.
        drop(client);
        let read = self.workforce.employees(scope, company_id).await?;
        let employees = read
            .require_current()
            .map_err(|e| TrainingError::WorkforceNotCurrent(e.0))?;
        Ok(employees
            .into_iter()
            .map(|employee| EmployeeOption {
                id: employee
                    .reference
                    .stable_id
                    .parse()
                    .expect("Workforce employment reference is an integer"),
                name: employee.display_name,
            })
            .collect())
    }
}

async fn insert_fact(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    actor: catalog::Actor,
    input: &NewAttendance,
    reason: &str,
    revision: i32,
) -> Result<AttendanceFact> {
    let tenant = scope.tenant();
    let input = input.clone();
    let reason = reason.to_owned();
    Ok(tx.mutate(move |sql| Box::pin(async move {
        let row = sql.query_one(&format!("INSERT INTO people_training_attendance_facts (tenant_id, company_id, session_id, employee_id, revision, status, reason, import_key, actor_user_id, impersonator_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING {FACT_COLUMNS}"), &[&tenant, &company_id, &input.session_id, &input.employee_id, &revision, &input.status.as_str(), &reason, &input.import_key, &actor.user, &actor.impersonator]).await?;
        let saved = fact(&row);
        let change = Change { kind: "people_training_attendance_fact".into(), id: saved.id, old: json!({}), new: json!(saved) };
        Ok((saved, vec![change]))
    })).await?)
}
