//! PA5: run on a fresh disposable database, then start bilibi-web against it.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::command("attendance-pages", &url, None).await
}
