//! `DATABASE_URL=… cargo run -p bilibi-people-walkthrough --bin attendance`,
//! after the default binary prepared the database: the attendance core
//! proof in `src/attendance.rs`. A process installs the composed services once, so
//! each later block is its own binary against the same database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    bilibi_people_walkthrough::command("attendance", &url, None).await
}
