This is a utility for managing stacks of Pull Requests in various git providers (GitHub, GitLab, etc), written in Rust.

Build, test and run static analysis checks before committing anything.

Build:
 * cargo build

Test:
 * cargo test

Static analysis:
 * cargo clippy --all-targets --all-features -- -D warnings
 * cargo fmt --all -- --check
