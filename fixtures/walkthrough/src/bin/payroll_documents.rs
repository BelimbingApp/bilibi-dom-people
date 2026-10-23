//! PY3 executable proof on a disposable database, after the default walkthrough.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::payroll_documents(&url).await
}
