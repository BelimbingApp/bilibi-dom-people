//! PG1 running-system proof, against its own empty disposable database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::performance(&url).await?;
    bilibi_people_walkthrough::progression(&url).await
}
