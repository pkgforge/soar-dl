use indicatif::{HumanBytes, ProgressBar, ProgressState, ProgressStyle};
use soar_dl::downloader::DownloadState;

pub fn create_progress_bar() -> ProgressBar {
    let progress_bar = ProgressBar::new(0);
    let style = ProgressStyle::with_template(
        "[{wide_bar:.green/white}] {bytes_per_sec:14} {computed_bytes:22}",
    )
    .unwrap()
    .with_key("computed_bytes", format_bytes)
    .progress_chars("━━");
    progress_bar.set_style(style);
    progress_bar
}

fn format_bytes(state: &ProgressState, w: &mut dyn std::fmt::Write) {
    write!(
        w,
        "{}/{}",
        HumanBytes(state.pos()),
        HumanBytes(state.len().unwrap_or(state.pos()))
    )
    .unwrap();
}

pub fn handle_progress(state: DownloadState, progress_bar: &ProgressBar) {
    match state {
        DownloadState::Preparing(total_size) => {
            progress_bar.set_length(total_size);
        }
        DownloadState::Progress(progress) => {
            progress_bar.set_position(progress);
        }
        DownloadState::Complete => progress_bar.finish(),
        _ => {}
    }
}
