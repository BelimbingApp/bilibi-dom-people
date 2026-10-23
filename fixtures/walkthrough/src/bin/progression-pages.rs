//! PG2: running-app fixture and permission changes for a disposable database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    use bilibi_database::Error;
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    let command = std::env::args().nth(1).unwrap_or_else(|| "prepare".into());
    if command == "--performance-fixture" {
        return bilibi_people_walkthrough::performance(&url).await;
    }
    if command == "prepare" {
        let status =
            std::process::Command::new(std::env::current_exe().map_err(|_| Error::Internal)?)
                .arg("--performance-fixture")
                .status()
                .map_err(|_| Error::Internal)?;
        if !status.success() {
            return Err(Error::Internal);
        }
    }
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    bilibi_people_walkthrough::progression_pages(&url, &command).await
}
