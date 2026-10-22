//! PY2 proof over the default walkthrough and Payroll foundation fixture.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::payroll_calculation(&url).await
}
