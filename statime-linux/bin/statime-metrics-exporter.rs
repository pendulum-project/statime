#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    statime_linux::metrics_exporter_main().await
}
