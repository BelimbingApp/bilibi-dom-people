//! Run after the default disposable walkthrough. No production data or test harness.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let action = std::env::args().nth(1).unwrap_or_else(|| "run".into());
    bilibi_people_walkthrough::training_passports(&url, &action).await
}
