//! Prepare a fresh disposable database for the PL3 browser interaction proof.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::command("leave-pages", &url, None).await
}
