//! Run after the default People walkthrough, on its disposable database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::command("skill-reminders", &url, None).await
}
