//! PT9 proof on an empty disposable database; setup also installs the walkthrough.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let action = std::env::args().nth(1).unwrap_or_else(|| "setup".into());
    bilibi_people_walkthrough::training_effectiveness_pages(&url, &action).await
}
