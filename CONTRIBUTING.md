# Contributing to pacm-v8

Thanks for your interest in contributing! This document outlines the process and expectations for contributors. If anything is unclear, please open an issue so we can clarify.

## Code of Conduct

Participation in this project is governed by the [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Please read it before joining discussions or contributing changes.

## Ways to Help

- Report bugs using the [issue tracker](https://github.com/pacmpkg/pacm-v8/issues)
- Suggest new features or improvements
- Improve documentation or examples
- Submit pull requests that fix issues or add new capabilities
- Review open pull requests and share feedback

## Development Environment

1. Install Rust 1.85 or newer (MSVC toolchain on Windows):
   ```ps1
   rustup default stable-x86_64-pc-windows-msvc
   ```
2. Clone the repository and install submodules if necessary:
   ```ps1
   git clone https://github.com/pacmpkg/pacm-v8.git
   cd pacm-v8
   git submodule update --init --recursive
   ```
3. (Optional) Install Python 3.11+ if you want to rebuild the V8 artifacts with `scripts/build_v8.py`.

## Workflow

1. Fork the repository and create a topic branch from `main`.
2. Keep your branch focused. Separate unrelated changes into independent pull requests.
3. Write tests for any new behavior or bug fixes when possible.
4. Run the quality gates before opening a pull request:
   ```ps1
   cargo fmt
   cargo clippy --all-targets --all-features
   ```
5. Update documentation if the change affects public APIs or behavior.
6. Link any relevant issues in your commit messages or pull request description.

## Building and Testing

- The crate uses prebuilt V8 binaries stored under the releases tab. If you need to refresh them, run:
  ```ps1
  python scripts/build_v8.py
  ```
  This script downloads and compiles V8 using Google's depot_tools. Expect the build to take time.
- Thin wrappers for the native shims live under `src/cpp/`. Changes here require rebuilding the native static library by running `cargo build`.
- Run the example application to confirm end-to-end behavior:
  ```ps1
  cd example
  cargo run
  ```

## Commit Guidelines

- Keep commits small and focused; rewrite history with `git rebase -i` if needed.
- Use present-tense, imperative mood commit messages (e.g., "Add host function registration helper").
- Include context in the body when the summary alone is insufficient.

## Pull Requests

When opening a pull request:

- Ensure the title clearly states the change
- Fill in the pull request template checklist
- Provide screenshots or logs for UX changes or build-script updates
- Mention maintainers if you need expedited review

## Release Process

Maintainers handle publishing to crates.io. If your change warrants a release:

1. Update the version in `Cargo.toml` following semver
2. Add a changelog entry (if applicable)
3. Open a pull request titled `Release x.y.z`
4. After approval, maintainers will tag the release and publish the crate

## Getting Help

If you get stuck:

- Open a draft pull request and describe where you need help
- Chat with maintainers via discussions or issues
- Reach out directly at pacm-maintainers@pacmpkg.org for sensitive matters

We appreciate your contributions and look forward to collaborating with you!
