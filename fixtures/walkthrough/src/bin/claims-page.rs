//! Run after the default walkthrough on a disposable database.
//! `claims-page prepare` adds browser records; revoke/restore and unlink/relink
//! change staff's access while a browser page remains open.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    let action = std::env::args().nth(1).unwrap_or_else(|| "prepare".into());
    bilibi_people_walkthrough::command("claims-page", &url, Some(&action)).await
}
