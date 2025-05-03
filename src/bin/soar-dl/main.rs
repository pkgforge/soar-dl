use std::{error::Error, sync::Arc};

use clap::Parser;
use cli::Args;
use download_manager::DownloadManager;
use progress::create_progress_bar;
use soar_dl::http_client::{configure_http_client, create_http_header_map};

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

    let proxy = args.proxy.clone();
    let user_agent = args.user_agent.clone();
    let header = args.header.clone();

    if let Err(err) = configure_http_client(|config| {
        config.proxy = proxy;

        if let Some(user_agent) = user_agent {
            config.user_agent = Some(user_agent);
        }

        if let Some(headers) = header {
            config.headers = Some(create_http_header_map(headers));
        }
    }) {
        error!("Error configuring HTTP client: {}", err);
        if let Some(source) = err.source() {
            error!("  Caused by: {}", source);
        }
        std::process::exit(1);
    };

    let manager = DownloadManager::new(args, progress_callback);
    manager.execute().await;
}
