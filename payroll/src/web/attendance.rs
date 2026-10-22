//! Bilimbi Payroll.Web.AttendanceMappingsLive: current/future rules and history.
use super::*;
use super::setup::{card, heading};
use askama::Template;
#[derive(Template)]
#[template(path="attendance_mappings.html")]
struct View<'a> { flash: FlashMap<'a>, has_companies: bool, has_company: bool, available: bool, company: Field, table: Html, form: Html, history: Html, no_items: bool }
impl State {
    pub(super) fn render_attendance(&self) -> Result<String> {
        let mut view = View { flash:self.flash.map(), has_companies:!self.companies.is_empty(),has_company:self.company.is_some(),available:self.attendance.is_some(),company:self.company_field("payroll-mapping-company"),table:Html::EMPTY,form:Html::EMPTY,history:Html::EMPTY,no_items:false };
        if let Some(data) = &self.attendance {
            let rows = data.sources.iter().map(|r| {
                let labels: Vec<_> = data.mappings.iter().filter(|m| m.attendance_rule_code == r.code && m.effective_from <= self.today.to_string() && m.effective_to.as_ref().is_none_or(|to| to >= &self.today.to_string())).map(|m| item_label(&data.items,m.item_id)).collect();
                row("payroll-attendance-rule",r.id).slot("rule",Html::text(&format!("{} · {}",r.name,r.code))).slot("value",Html::text(&format!("{} {} / {}",r.value,r.currency,r.unit))).slot("effective",Html::text(&effective(&r.effective_from,r.effective_until.as_deref()))).slot("items",Html::text(&if labels.is_empty() { "Not mapped".into() } else { labels.join(", ") }))
            }).collect();
            view.table = self.tables["rules"].render("Attendance allowance rules",rows,"No current or future allowance rules are configured.","payroll-mappings-no-rules")?;
            view.no_items = !data.sources.is_empty() && data.items.is_empty();
            if self.can_map && !data.sources.is_empty() && !data.items.is_empty() {
                let mut seen = std::collections::HashSet::new();
                let options = data.sources.iter().filter(|r| seen.insert(r.code.clone())).map(|r| (r.code.clone(),format!("{} · {}",r.name,r.code))).collect();
                let mut fields = vec![self.select("create","mapping","attendance_rule_code","Attendance rule",options), self.select("create","mapping","item_id","Pay item",data.items.iter().map(|i| (i.id.to_string(),format!("{} · {}",item_label(&data.items,i.id),effective(&i.effective_from,i.effective_to.as_deref())))).collect())];
                fields.extend(self.dates("create","mapping"));
                let mut body=heading("","Add mapping version","")?;
                body.push(&self.form("payroll-attendance-mapping-form","create",fields,"Add mapping")?);
                view.form=card("",body)?;
            }
            if !data.mappings.is_empty() {
                let mut body=heading("","Mapping history","")?;
                body.push(&self.tables["history"].render("Mapping history",data.mappings.iter().map(|m| row("payroll-attendance-mapping",m.id).slot("rule",Html::text(&m.attendance_rule_code)).slot("item",Html::text(&item_label(&data.items,m.item_id))).slot("effective",Html::text(&effective(&m.effective_from,m.effective_to.as_deref())))).collect(),"No mappings yet.","attendance-history-empty")?);
                view.history=card("",body)?;
            }
        }
        view.render().map_err(|_| Error::Internal)
    }
}
fn item_label(items: &[crate::Item],id:i64) -> String { items.iter().find(|i| i.id==id).map(|i| format!("{} · {} · {}",i.name,i.code,i.currency)).unwrap_or_else(|| format!("Item {id}")) }
