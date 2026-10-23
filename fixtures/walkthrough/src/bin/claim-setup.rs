//! Run after the default People walkthrough on a disposable database.
//! proof drives the registered live page; revoke/restore/archive steer browser checks.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let action = std::env::args().nth(1).unwrap_or_else(|| "proof".into());
    bilibi_people_walkthrough::claim_setup(&url, &action).await
}
