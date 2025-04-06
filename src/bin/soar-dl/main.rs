use std::sync::Arc;

use clap::Parser;
use cli::Args;
use download_manager::DownloadManager;
use progress::create_progress_bar;

mod cli;
mod download_manager;
mod log;
mod progress;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    log::init(args.quiet);

    let progress_bar = create_progress_bar();
    let progress_callback = Arc::new(move |state| progress::handle_progress(state, &progress_bar));

    let manager = DownloadManager::new(args, progress_callback);
    manager.execute().await;
}
