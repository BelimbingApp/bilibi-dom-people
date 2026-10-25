//! `DATABASE_URL=… BILIBI_ENV=development cargo run -p bilibi-people-walkthrough --bin performance`
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::command("performance", &url, None).await
}
