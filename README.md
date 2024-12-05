# soar-dl
A lightning-fast, feature-rich release asset downloader for GitHub and GitLab repositories with support for direct downloads.

# Installation
```sh
cargo install soar-dl -F binary
```

# Usage
## Examples

> [!note]
> Any filter or output path you specify applies to all the assets.

```sh
# Download from github, using specific tag
soar-dl --github "pkgforge/soar@nightly"

# Download from gitlab
soar-dl --gitlab "inkscape/inkscape"

# Download using gitlab project id
soar-dl --github "18817634"

# Direct download
soar-dl "https://github.com/pkgforge/soar/releases/download/nightly/soar-nightly-x86_64-linux"

# Filter assets
soar-dl --github "pkgforge/soar" --regex ".*x86_64" --exclude "tar,b3sum"
soar-dl --github "pkgforge/soar" --match "x86_64,tar" --exclude "b3sum"

# Specify output path. Trailing / means it's a directory
soar-dl --github "pkgforge/soar" --gitlab "18817634" --output "final/"

# Don't do this. The last download will replace the existing file
# Only use file in output path if you're downloading single file.
soar-dl --github "pkgforge/soar" --gitlab "18817634" --output "final"
```

## Command Line Options
```
Usage: soar-dl [OPTIONS] [LINKS]...

Arguments:
  [LINKS]...  Links to files

Options:
      --github <GITHUB>             Github project
      --gitlab <GITLAB>             Gitlab project
  -r, --regex <REGEX_PATTERNS>      Regex to select the asset. Only works for github downloads
  -m, --match <MATCH_KEYWORDS>      Check if the asset contains given string
  -e, --exclude <EXCLUDE_KEYWORDS>  Check if the asset contains given string
  -y, --yes                         Skip all prompts and use first
  -o, --output <OUTPUT>             Output file path
  -h, --help                        Print help
  -V, --version                     Print version
```
