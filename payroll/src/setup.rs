//! Bilimbi Payroll setup, immutable versions and owner catalog reads.
use crate::{
    input,
    store::{self, Kind},
    *,
};
use jiff::civil::Date;
use serde_json::json;
impl Payroll {
    /// Full setup in stable id order; requires VIEW on a live company.
    pub async fn setup(&self, scope: &Scope, company: i64) -> Result<Setup> {
        let c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, VIEW).await?;
        let (country, currencies) = self.configuration(&**c, scope, company).await?;
        Ok(Setup {
            country,
            currencies,
            classifications: store::list(&**c, scope, company, Kind::Classification).await?,
            items: store::list(&**c, scope, company, Kind::Item).await?,
            periods: store::list(&**c, scope, company, Kind::Period).await?,
            mappings: store::list(&**c, scope, company, Kind::Mapping).await?,
            runs: store::list(&**c, scope, company, Kind::Run).await?,
        })
    }
    /// Both settings change atomically. Currency codes are explicit, uppercase and deduplicated.
    pub async fn put_settings(
        &self,
        scope: &Scope,
        company: i64,
        country: &str,
        currencies: &[String],
    ) -> Result<()> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut c,
                scope,
                company,
                MANAGE,
                "people.payroll.put_settings",
            )
            .await?;
        let country = country.trim();
        if !(1..=100).contains(&country.chars().count())
            || currencies.len() > 20
            || !currencies.iter().all(|v| contributions::currency_shaped(v))
        {
            return Err(PayrollError::InvalidSettings);
        }
        let mut unique = Vec::new();
        for code in currencies {
            if !unique.contains(code) {
                unique.push(code.clone());
            }
        }
        let target =
            Target::Company(OrganisationId::new(company).map_err(|_| PayrollError::NotFound)?);
        self.settings
            .put(&mut tx, scope, target, COUNTRY_KEY, json!(country))
            .await?;
        self.settings
            .put(&mut tx, scope, target, CURRENCIES_KEY, json!(unique))
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn create_classification(
        &self,
        scope: &Scope,
        company: i64,
        new: &NewClassification,
    ) -> Result<Classification> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut c,
                scope,
                company,
                MANAGE,
                "people.payroll.create_classification",
            )
            .await?;
        let code = input::text(&new.code, "code", 60)?;
        let name = input::text(&new.name, "name", 120)?;
        input::dates(new.effective_from, new.effective_to)?;
        let from = new.effective_from.to_string();
        let to = new.effective_to.map(|d| d.to_string());
        let existing: Vec<Classification> =
            store::list(tx.sql(), scope, company, Kind::Classification).await?;
        if existing.iter().any(|v| {
            v.code == code
                && input::overlaps(
                    &from,
                    to.as_deref(),
                    &v.effective_from,
                    v.effective_to.as_deref(),
                )
        }) {
            return Err(PayrollError::OverlappingVersion);
        }
        let row = store::insert(
            &mut tx,
            scope,
            company,
            Kind::Classification,
            json!({"code":code,"name":name,"effective_from":from,"effective_to":to}),
        )
        .await?;
        tx.commit().await?;
        Ok(row)
    }
    pub async fn create_item(&self, scope: &Scope, company: i64, new: &NewItem) -> Result<Item> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(&mut c, scope, company, MANAGE, "people.payroll.create_item")
            .await?;
        let code = input::text(&new.code, "code", 60)?;
        let name = input::text(&new.name, "name", 120)?;
        input::dates(new.effective_from, new.effective_to)?;
        let from = new.effective_from.to_string();
        let to = new.effective_to.map(|d| d.to_string());
        let (_, currencies) = self.configuration(tx.sql(), scope, company).await?;
        let parent: Classification = store::get(
            tx.sql(),
            scope,
            company,
            Kind::Classification,
            new.classification_id,
        )
        .await?
        .ok_or(PayrollError::InvalidItem)?;
        if !currencies.contains(&new.currency)
            || new.amount < rust_decimal::Decimal::ZERO
            || new.amount >= rust_decimal::Decimal::from(100_000_000_000_000i64)
            || new.amount.round_dp(6) != new.amount
            || !input::covers(
                &parent.effective_from,
                parent.effective_to.as_deref(),
                &from,
                to.as_deref(),
            )
        {
            return Err(PayrollError::InvalidItem);
        }
        let existing: Vec<Item> = store::list(tx.sql(), scope, company, Kind::Item).await?;
        if existing.iter().any(|v| {
            v.code == code
                && input::overlaps(
                    &from,
                    to.as_deref(),
                    &v.effective_from,
                    v.effective_to.as_deref(),
                )
        }) {
            return Err(PayrollError::OverlappingVersion);
        }
        let row=store::insert(&mut tx,scope,company,Kind::Item,json!({"code":code,"name":name,"classification_id":new.classification_id,"currency":new.currency,"amount":new.amount.to_string(),"effective_from":from,"effective_to":to})).await?;
        tx.commit().await?;
        Ok(row)
    }
    pub async fn create_period(
        &self,
        scope: &Scope,
        company: i64,
        new: &NewPeriod,
    ) -> Result<Period> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut c,
                scope,
                company,
                MANAGE,
                "people.payroll.create_period",
            )
            .await?;
        let code = input::text(&new.code, "code", 60)?;
        if new.ends_on < new.starts_on {
            return Err(PayrollError::Invalid(vec![FieldError::new(
                "ends_on",
                "must follow start",
            )]));
        }
        if new.pay_on < new.ends_on {
            return Err(PayrollError::Invalid(vec![FieldError::new(
                "pay_on",
                "must be on or after the period end",
            )]));
        }
        let from = new.starts_on.to_string();
        let to = new.ends_on.to_string();
        let existing: Vec<Period> = store::list(tx.sql(), scope, company, Kind::Period).await?;
        if existing.iter().any(|v| v.code == code) {
            return Err(PayrollError::Invalid(vec![FieldError::new(
                "code",
                "has already been taken",
            )]));
        }
        if existing
            .iter()
            .any(|v| input::overlaps(&from, Some(&to), &v.starts_on, Some(&v.ends_on)))
        {
            return Err(PayrollError::Invalid(vec![FieldError::new(
                "starts_on",
                "must not overlap another period",
            )]));
        }
        let row = store::insert(
            &mut tx,
            scope,
            company,
            Kind::Period,
            json!({"code":code,"starts_on":from,"ends_on":to,"pay_on":new.pay_on.to_string()}),
        )
        .await?;
        tx.commit().await?;
        Ok(row)
    }
    /// Company catalogs, through their owners. A non-current Workforce read refuses this read.
    pub async fn sources(&self, scope: &Scope, company: i64) -> Result<Sources> {
        let c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, VIEW).await?;
        self.workforce
            .company_on(&**c, scope, company)
            .await?
            .require_current()
            .map_err(|e| PayrollError::NotCurrent(e.0))?;
        drop(c);
        let mut sources = Sources::new();
        sources.insert(
            "leave".into(),
            self.leave
                .list_types(scope, company)
                .await?
                .into_iter()
                .map(|v| Source {
                    key: v.id.to_string(),
                    name: v.name,
                    active: v.status == "active",
                })
                .collect(),
        );
        sources.insert(
            "claims".into(),
            self.claims
                .claim_types(scope, company)
                .await?
                .into_iter()
                .map(|v| Source {
                    key: v.id.to_string(),
                    name: v.name,
                    active: v.active,
                })
                .collect(),
        );
        sources.insert("attendance".into(), vec![]);
        Ok(sources)
    }
    pub async fn create_mapping(
        &self,
        scope: &Scope,
        company: i64,
        new: &NewMapping,
    ) -> Result<Mapping> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut c,
                scope,
                company,
                MANAGE,
                "people.payroll.create_mapping",
            )
            .await?;
        input::dates(new.effective_from, new.effective_to)?;
        let key = input::text(&new.source_key, "source_key", 100)?;
        let from = new.effective_from.to_string();
        let to = new.effective_to.map(|d| d.to_string());
        if !["leave", "claims"].contains(&new.source_kind.as_str())
            || !self
                .sources(scope, company)
                .await?
                .get(&new.source_kind)
                .is_some_and(|v| v.iter().any(|v| v.key == key))
        {
            return Err(PayrollError::InvalidMapping);
        }
        let item: Item = store::get(tx.sql(), scope, company, Kind::Item, new.item_id)
            .await?
            .ok_or(PayrollError::InvalidMapping)?;
        if !input::covers(
            &item.effective_from,
            item.effective_to.as_deref(),
            &from,
            to.as_deref(),
        ) {
            return Err(PayrollError::InvalidMapping);
        }
        let existing: Vec<Mapping> = store::list(tx.sql(), scope, company, Kind::Mapping).await?;
        let items: Vec<Item> = store::list(tx.sql(), scope, company, Kind::Item).await?;
        if existing.iter().any(|v| {
            v.source_kind == new.source_kind
                && v.source_key == key
                && same_currency(&items, v.item_id, &item.currency)
                && input::overlaps(
                    &from,
                    to.as_deref(),
                    &v.effective_from,
                    v.effective_to.as_deref(),
                )
        }) {
            return Err(PayrollError::OverlappingVersion);
        }
        let row=store::insert(&mut tx,scope,company,Kind::Mapping,json!({"source_kind":new.source_kind,"source_key":key,"item_id":new.item_id,"effective_from":from,"effective_to":to})).await?;
        tx.commit().await?;
        Ok(row)
    }
    /// Every active rule not ended by as_of, including future-effective versions.
    pub async fn attendance_allowances(
        &self,
        scope: &Scope,
        company: i64,
        as_of: Date,
    ) -> Result<AttendanceAllowances> {
        let c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, ATTENDANCE_MAPPINGS_MANAGE)
            .await?;
        let items = store::list(&**c, scope, company, Kind::Item).await?;
        let mappings = store::list(&**c, scope, company, Kind::AttendanceMapping).await?;
        drop(c);
        let mut sources = self.attendance.list_allowance_rules(scope, company).await?;
        let from = as_of.to_string();
        sources.retain(|r| rule_overlaps(r, &from, None));
        sources.sort_by(|a, b| {
            (&a.code, &a.effective_from, a.id).cmp(&(&b.code, &b.effective_from, b.id))
        });
        Ok(AttendanceAllowances {
            sources,
            items,
            mappings,
        })
    }
    pub async fn create_attendance_allowance_mapping(
        &self,
        scope: &Scope,
        company: i64,
        new: &NewAttendanceAllowanceMapping,
    ) -> Result<AttendanceAllowanceMapping> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut c,
                scope,
                company,
                ATTENDANCE_MAPPINGS_MANAGE,
                "people.payroll.create_attendance_allowance_mapping",
            )
            .await?;
        input::dates(new.effective_from, new.effective_to)?;
        let code = input::text(new.attendance_rule_code.trim(), "attendance_rule_code", 40)?;
        let from = new.effective_from.to_string();
        let to = new.effective_to.map(|d| d.to_string());
        let item: Item = store::get(tx.sql(), scope, company, Kind::Item, new.item_id)
            .await?
            .ok_or(PayrollError::InvalidAttendanceMapping)?;
        let rules = self.attendance.list_allowance_rules(scope, company).await?;
        let versions: Vec<_> = rules
            .iter()
            .filter(|r| r.code == code && rule_overlaps(r, &from, to.as_deref()))
            .collect();
        if !input::covers(
            &item.effective_from,
            item.effective_to.as_deref(),
            &from,
            to.as_deref(),
        ) || versions.is_empty()
            || versions.iter().any(|v| v.currency != item.currency)
        {
            return Err(PayrollError::InvalidAttendanceMapping);
        }
        let existing: Vec<AttendanceAllowanceMapping> =
            store::list(tx.sql(), scope, company, Kind::AttendanceMapping).await?;
        let items: Vec<Item> = store::list(tx.sql(), scope, company, Kind::Item).await?;
        if existing.iter().any(|v| {
            v.attendance_rule_code == code
                && same_currency(&items, v.item_id, &item.currency)
                && input::overlaps(
                    &from,
                    to.as_deref(),
                    &v.effective_from,
                    v.effective_to.as_deref(),
                )
        }) {
            return Err(PayrollError::OverlappingVersion);
        }
        let row=store::insert(&mut tx,scope,company,Kind::AttendanceMapping,json!({"attendance_rule_code":code,"item_id":new.item_id,"effective_from":from,"effective_to":to})).await?;
        tx.commit().await?;
        Ok(row)
    }
    pub async fn lock_run(&self, scope: &Scope, company: i64, run: i64) -> Result<Run> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(&mut c, scope, company, MANAGE, "people.payroll.lock_run")
            .await?;
        let result = store::lock_run(&mut tx, scope, company, run).await?;
        tx.commit().await?;
        Ok(result)
    }
}
fn same_currency(items: &[Item], id: i64, currency: &str) -> bool {
    items.iter().any(|i| i.id == id && i.currency == currency)
}
pub(crate) fn rule_overlaps(
    rule: &bilibi_people_attendance::AllowanceRule,
    from: &str,
    to: Option<&str>,
) -> bool {
    rule.status == "active"
        && input::overlaps(
            from,
            to,
            &rule.effective_from,
            rule.effective_until.as_deref(),
        )
}
