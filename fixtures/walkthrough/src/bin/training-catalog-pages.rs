//! PT6 catalog/page proof, independent of the other slices' walkthroughs.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::command("training-catalog-pages", &url, None).await
}
