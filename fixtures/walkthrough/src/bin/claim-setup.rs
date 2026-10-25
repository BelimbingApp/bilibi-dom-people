//! Run after the default People walkthrough on a disposable database.
//! proof drives the registered page; revoke/restore/archive steer browser checks.
//! archived verifies the facade refusal after archiving the disposable company.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let action = std::env::args().nth(1).unwrap_or_else(|| "proof".into());
    bilibi_people_walkthrough::command("claim-setup", &url, Some(&action)).await
}
