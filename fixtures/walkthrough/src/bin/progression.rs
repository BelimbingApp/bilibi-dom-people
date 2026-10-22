//! PG1 running-system proof, against its own fresh disposable database.
#[tokio::main]
async fn main() -> bilibi_database::Result<()> {
    use bilibi_database::Error;
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    if std::env::args().nth(1).as_deref() == Some("--performance-fixture") {
        return bilibi_people_walkthrough::performance(&url).await;
    }
    // Identity service bindings install once per process. Create the shared
    // Performance records in a child, then install Progression's services here.
    let executable = std::env::current_exe().map_err(|_| Error::Internal)?;
    let status = std::process::Command::new(executable)
        .arg("--performance-fixture")
        .status()
        .map_err(|_| Error::Internal)?;
    if !status.success() {
        return Err(Error::Invalid);
    }
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    bilibi_people_walkthrough::progression(&url).await
}
