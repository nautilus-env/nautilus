#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    nautilus_engine::run_engine_from_cli().await
}
