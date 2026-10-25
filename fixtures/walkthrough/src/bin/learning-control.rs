//! Disposable Learning browser proof; prepare the database with the default binary first.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let action = std::env::args()
        .nth(1)
        .ok_or(bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::command("learning-control", &url, Some(&action)).await
}
