//! PC5: run after the People walkthrough installed its disposable database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::claim_operations(&url, std::env::args().nth(1).as_deref()).await
}
