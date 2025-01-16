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

    /// OCI reference
    #[arg(required = false, long)]
    pub ghcr: Vec<String>,

    /// Links to files
    #[arg(required = false)]
    pub links: Vec<String>,

    /// Regex to select the asset.
    #[arg(required = false, short = 'r', long = "regex")]
    pub regex_patterns: Option<Vec<String>>,

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
}
