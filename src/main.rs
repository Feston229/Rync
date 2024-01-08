mod config;
mod controllers;
mod crypto;
mod share;
mod test;
mod utils;

use controllers::run;

#[tokio::main]
async fn main() {
    run().await.unwrap_or_else(|e| tracing::error!("{}", e));
}
