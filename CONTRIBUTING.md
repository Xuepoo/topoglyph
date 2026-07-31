# Contributing to TopoGlyph

Thank you for your interest in contributing!

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable)
- [FFmpeg development headers](https://ffmpeg.org/) (`libavcodec`, `libavformat`, `libavutil`, `libswscale`, `libswresample`, `libavdevice`, `libavfilter` — required to build `topoglyph-video`/the CLI's default `video` feature)
- [pre-commit](https://pre-commit.com/) (optional)

## Development Setup

```bash
git clone https://github.com/Xuepoo/topoglyph.git
cd topoglyph
cargo build
cargo test
```

## Code Quality

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Commit Convention

We use [Conventional Commits](https://www.conventionalcommits.org/):

| Prefix      | Usage                     |
| ----------- | ------------------------- |
| `feat:`     | New feature               |
| `fix:`      | Bug fix                   |
| `docs:`     | Documentation only        |
| `refactor:` | Code change (no feat/fix) |
| `test:`     | Tests                     |
| `ci:`       | CI/CD changes             |
| `deps:`     | Dependency updates        |

## Release Process

Releases are automated via CI. When a version tag (`v*`) is pushed:

1. CI builds binaries for 5 platforms (Linux x86/arm64, macOS x86/arm64, Windows)
2. Packages (.deb, .rpm, .pkg.tar.zst) are created
3. Published to crates.io, AUR, Docker Hub, Homebrew, and Scoop
