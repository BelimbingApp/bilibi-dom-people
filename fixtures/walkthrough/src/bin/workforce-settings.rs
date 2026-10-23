//! PW2 standalone proof and browser fixture on a disposable database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let action = std::env::args().nth(1).unwrap_or_else(|| "setup".into());
    bilibi_people_walkthrough::workforce_settings(&url, &action).await
}
