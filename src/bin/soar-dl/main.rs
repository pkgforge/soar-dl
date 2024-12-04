#[cfg(feature = "binary")]
use std::sync::Arc;

#[cfg(feature = "binary")]
use clap::Parser;
#[cfg(feature = "binary")]
use cli::Args;
#[cfg(feature = "binary")]
use download_manager::DownloadManager;
#[cfg(feature = "binary")]
use progress::create_progress_bar;

#[cfg(feature = "binary")]
mod cli;
#[cfg(feature = "binary")]
mod download_manager;
#[cfg(feature = "binary")]
mod progress;

#[cfg(feature = "binary")]
#[tokio::main]
async fn main() {
    let args = Args::parse();
    let progress_bar = create_progress_bar();
    let progress_callback = Arc::new(move |state| progress::handle_progress(state, &progress_bar));

    let manager = DownloadManager::new(args, progress_callback);
    manager.execute().await;
}
