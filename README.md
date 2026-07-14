<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://gita.jokelbaf.dev/api/widget/public/crunchydl-readme-hero?w=880&amp;theme=dark" />
  <img alt="crunchydl - Download, decrypt, and mux your Crunchyroll library without external tools" src="https://gita.jokelbaf.dev/api/widget/public/crunchydl-readme-hero?w=880&amp;theme=light" width="880" />
</picture>

**Crunchydl** is an experimental, pure-Rust Crunchyroll downloader with a headless CLI and a polished terminal interface. It handles catalog discovery, resumable DASH transfers, PlayReady or Widevine licensing, CENC decryption, subtitles, chapters, fonts, and native muxing without invoking FFmpeg, MKVToolNix, Bento4, Shaka Packager, or any other executable.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://gita.jokelbaf.dev/api/widget/public/crunchydl-feature-cards?w=880&amp;theme=dark&amp;items=Pure%20Rust%7CDASH%20transfer%2C%20CENC%20decryption%2C%20and%20native%20muxing%20without%20external%20tools%3BResume-safe%7CAtomic%20staging%2C%20retries%2C%20cancellation%2C%20and%20crash-resumable%20downloads%3BFull%20fidelity%7CMulti-audio%20MKV%2C%20ASS%20subtitles%2C%20chapters%2C%20fonts%2C%20plus%20strict%20MP4%3BTerminal%20first%7CHeadless%20commands%20and%20a%20polished%20Ratatui%20interface" />
  <img alt="crunchydl features: pure Rust, resume-safe, full fidelity, and terminal first" src="https://gita.jokelbaf.dev/api/widget/public/crunchydl-feature-cards?w=880&amp;theme=light&amp;items=Pure%20Rust%7CDASH%20transfer%2C%20CENC%20decryption%2C%20and%20native%20muxing%20without%20external%20tools%3BResume-safe%7CAtomic%20staging%2C%20retries%2C%20cancellation%2C%20and%20crash-resumable%20downloads%3BFull%20fidelity%7CMulti-audio%20MKV%2C%20ASS%20subtitles%2C%20chapters%2C%20fonts%2C%20plus%20strict%20MP4%3BTerminal%20first%7CHeadless%20commands%20and%20a%20polished%20Ratatui%20interface" width="880" />
</picture>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=dark&amp;label=Installation&amp;kicker=Get%20started" />
  <img alt="Installation" src="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=light&amp;label=Installation&amp;kicker=Get%20started" width="880" />
</picture>

### Download a release

Ready-to-run archives are attached to every [GitHub release](https://github.com/jokelbaf/crunchydl/releases/latest). Each archive includes `crunchydl`, this README, the GPL-3.0 license, and a matching SHA-256 checksum file.

| Platform | Architectures | Archive |
| --- | --- | --- |
| Linux | x86_64, ARM64 | `.tar.gz` |
| macOS | Intel, Apple silicon | `.tar.gz` |
| Windows | x86_64, ARM64 | `.zip` |

Release executables include both PlayReady and Widevine support. After extracting the archive, place `crunchydl` (or `crunchydl.exe`) somewhere on your `PATH`.

Artifacts are currently unsigned. Verify the adjacent SHA-256 checksum before running them; macOS or Windows may require you to approve the binary explicitly.

### Install from source

The workspace uses the nightly Rust toolchain pinned in `rust-toolchain.toml`.

```bash
git clone https://github.com/jokelbaf/crunchydl.git
cd crunchydl
cargo install --path cli --locked --all-features
```

Omit `--all-features` for a PlayReady-only build.

To install the latest source directly from GitHub:

```bash
cargo install --git https://github.com/jokelbaf/crunchydl --package cli --locked --all-features
```

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=dark&amp;label=Quick%20start&amp;kicker=CLI%20or%20TUI" />
  <img alt="Quick start" src="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=light&amp;label=Quick%20start&amp;kicker=CLI%20or%20TUI" width="880" />
</picture>

Sign in, configure your DRM device, then launch the terminal interface:

```bash
crunchydl login
crunchydl config set --drm-device /path/to/device.prd
crunchydl
```

Headless commands use the same configuration and download engine:

```bash
crunchydl search "Frieren"
crunchydl browse series GYYYYYYY
crunchydl browse season GZZZZZZZ
crunchydl download episode GXXXXXXX --audio ja-JP --subtitle en-US
crunchydl download series GYYYYYYY --all-audio --season 1 --exclude-specials
crunchydl queue add season GZZZZZZZ --subtitle en-US
crunchydl queue run
```

Add `--json` to search and browse commands for script-friendly output. Run `crunchydl help` or `crunchydl <command> --help` for the complete command reference.

The TUI keeps discovery, downloads, settings, account information, and help on `F1` through `F5`.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=dark&amp;label=Format%20support&amp;kicker=Explicit%20contracts" />
  <img alt="Format support" src="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=light&amp;label=Format%20support&amp;kicker=Explicit%20contracts" width="880" />
</picture>

| Area | Supported |
| --- | --- |
| Catalog | Episodes, movies, music videos, seasons, series, and movie listings |
| Input media | Crunchyroll DASH with fragmented MP4 AVC/H.264 video and AAC audio |
| DRM | PlayReady and optional Widevine with caller-supplied device material |
| Subtitles | Raw ASS plus focused WebVTT-to-ASS conversion, captions, signs, and referenced local fonts |
| Chapters | Normalized recap, intro, credits, preview, and uncovered episode ranges |
| Matroska | Multiple audio tracks, ASS subtitles, chapters, font attachments, cues, names, languages, and flags |
| MP4 | One AVC video with one or more AAC tracks; no subtitles, chapters, or silent feature loss |

Matroska is the full-fidelity default. MP4 is a strict native remux and requires the richer features to be disabled explicitly:

```bash
crunchydl download movie GXXXXXXX --format mp4 --no-subtitles --no-chapters
```

Unsupported media layouts return typed errors instead of guessed timestamps, dropped tracks, or subprocess fallbacks.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=dark&amp;label=Development&amp;kicker=Nightly%20Rust" />
  <img alt="Development" src="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=light&amp;label=Development&amp;kicker=Nightly%20Rust" width="880" />
</picture>

Any code pushed to the repository must pass formatting, linting, and tests. The workspace uses the nightly Rust toolchain pinned in `rust-toolchain.toml`.

Formatting:
```bash
cargo fmt --all -- --check
```

Linting:
```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Tests:
```bash
cargo test --workspace --all-features
```

Documentation:
```bash
cargo doc --workspace --all-features --no-deps
```

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=dark&amp;label=License%20%26%20responsible%20use&amp;kicker=GPL-3.0" />
  <img alt="License and responsible use" src="https://gita.jokelbaf.dev/api/widget/public/crunchydl-section-divider?w=880&amp;theme=light&amp;label=License%20%26%20responsible%20use&amp;kicker=GPL-3.0" width="880" />
</picture>

**Crunchydl** is licensed under the [GNU General Public License v3.0](LICENSE).

This project is not affiliated with, endorsed by, or sponsored by Crunchyroll, LLC or Sony Group Corporation. Crunchyroll names and marks belong to their respective owners. Use `crunchydl` only with an account and content you are authorized to access, and comply with the service terms and laws that apply to you. The project does not provide accounts, DRM devices, content keys, or downloaded media.
