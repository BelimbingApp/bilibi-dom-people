//! `DATABASE_URL=… cargo run -p bilibi-people-walkthrough`; see `../README.md`.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("--list") {
        return bilibi_people_walkthrough::list();
    }
    let url = std::env::var("DATABASE_URL").map_err(|_| bilibi_database::Error::Invalid)?;
    match command {
        Some(command) => {
            bilibi_people_walkthrough::command(&command, &url, args.next().as_deref()).await
        }
        None => bilibi_people_walkthrough::run(&url).await,
    }
}
