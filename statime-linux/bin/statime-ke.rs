#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    statime_linux::ke_main().await
}
