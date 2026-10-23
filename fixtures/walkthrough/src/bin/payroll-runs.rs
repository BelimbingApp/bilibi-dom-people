//! PY5 proof; run after the default People walkthrough on a disposable database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::payroll_runs(&url).await
}
