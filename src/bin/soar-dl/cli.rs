use clap::Parser;

#[derive(Parser)]
#[command(
    author,
    version,
    about,
    help_template = "{before-help}{name} {version}
{author-with-newline}{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}",
    arg_required_else_help = true
)]
pub struct Args {
    /// Github project
    #[arg(required = false, long)]
    pub github: Vec<String>,

    /// Gitlab project
    #[arg(required = false, long)]
    pub gitlab: Vec<String>,

    /// GHCR image or blob
    #[arg(required = false, long)]
    pub ghcr: Vec<String>,

    /// Links to files
    #[arg(required = false)]
    pub links: Vec<String>,

    /// Regex to select the asset.
    #[arg(required = false, short = 'r', long = "regex")]
    pub regexes: Option<Vec<String>>,

    /// Glob to select the asset.
    #[arg(required = false, short = 'g', long = "glob")]
    pub globs: Option<Vec<String>>,

    /// Check if the asset contains given string
    #[arg(required = false, short, long = "match")]
    pub match_keywords: Option<Vec<String>>,

    /// Check if the asset contains given string
    #[arg(required = false, short, long = "exclude")]
    pub exclude_keywords: Option<Vec<String>>,

    /// Skip all prompts and use first
    #[arg(required = false, short, long)]
    pub yes: bool,

    /// Output file path
    #[arg(required = false, short, long)]
    pub output: Option<String>,

    /// GHCR concurrency
    #[arg(required = false, short, long)]
    pub concurrency: Option<u64>,

    /// GHCR API to use
    #[arg(required = false, long)]
    pub ghcr_api: Option<String>,

    /// Whether to use exact case matching for keywords
    #[arg(required = false, long)]
    pub exact_case: bool,

    /// Extract supported archive automatically
    #[arg(required = false, long)]
    pub extract: bool,

    /// Directory where to extract the archive
    #[arg(required = false, long)]
    pub extract_dir: Option<String>,

    /// Quiet mode
    #[arg(required = false, long, short)]
    pub quiet: bool,

    /// Set proxy
    #[arg(required = false, long)]
    pub proxy: Option<String>,

    /// Set request headers
    #[arg(required = false, long, short = 'H')]
    pub header: Option<Vec<String>>,

    /// Set user agent
    #[arg(required = false, long, short = 'A')]
    pub user_agent: Option<String>,

    /// Skip existing download with same file
    #[arg(required = false, long)]
    pub skip_existing: bool,

    /// Overwrite existing download with same file
    #[arg(required = false, long)]
    pub force_overwrite: bool,
}
