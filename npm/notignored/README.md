# notignored-cli

Find every lint and type-check suppression comment in a codebase — natively, and
fast.

```console
npm install -g notignored-cli
notignored src/
```

Or without installing:

```console
npx notignored-cli src/
```

This package ships the **prebuilt** `notignored` binary: installing it needs no
Rust toolchain and compiles nothing. The binary for your platform arrives through
an optional dependency (`notignored-cli-<platform>-<arch>`), so npm downloads
exactly one of them.

Prebuilt binaries exist for Linux (x64, arm64), macOS (x64, arm64), and Windows
(x64). On any other platform, install with `cargo install --git
https://github.com/nickderobertis/notignored --locked`.

Full documentation: <https://github.com/nickderobertis/notignored>
