# soar-dl

A lightning-fast, feature-rich release download manager with support for GitHub, GitLab and OCI package downloads

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

# Download ghcr image or blob
# Note: when using ghcr image, the custom path is always treated as a directory
soar-dl --ghcr "ghcr.io/pkgforge/pkgcache/86box/appimage/official/stable/86box:v4.2.1-x86_64-linux"
soar-dl --ghcr "ghcr.io/pkgforge/pkgcache/86box/appimage/official/stable/86box@sha256:28e166a2253f058bfe380bd856cd056b3ca9d8544fc82193f017bb7fdc39b749"

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
      --ghcr <GHCR>                 GHCR image or blob
  -r, --regex <REGEX_PATTERNS>      Regex to select the asset
  -m, --match <MATCH_KEYWORDS>      Check if the asset contains given string
  -e, --exclude <EXCLUDE_KEYWORDS>  Check if the asset contains given string
  -y, --yes                         Skip all prompts and use first
  -o, --output <OUTPUT>             Output file path
  -c, --concurrency <CONCURRENCY>   GHCR concurrency
      --ghcr-api <GHCR_API>         GHCR API to use
      --exact-case                  Whether to use exact case matching for keywords
  -h, --help                        Print help
  -V, --version                     Print version
```
