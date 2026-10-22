//! PA4 browser fixture; run the main People walkthrough once before `seed`.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let mode = std::env::args().nth(1).unwrap_or_else(|| "seed".into());
    bilibi_people_walkthrough::my_attendance(&url, &mode).await
}
