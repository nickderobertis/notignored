# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Releases are cut automatically from Conventional Commits by
[release-plz](https://release-plz.dev/): it writes the sections below, bumps the
version, and tags. Do not edit released sections by hand.

## [Unreleased]

## [0.1.5](https://github.com/nickderobertis/notignored/compare/v0.1.4...v0.1.5) - 2026-08-07

### Fixed

- *(sdk)* hold the SDK to the approved contract exactly ([#23](https://github.com/nickderobertis/notignored/pull/23))

## [0.1.4](https://github.com/nickderobertis/notignored/compare/v0.1.3...v0.1.4) - 2026-08-07

### Added

- *(sdk)* implement the typed Python SDK ([#20](https://github.com/nickderobertis/notignored/pull/20))

## [0.1.3](https://github.com/nickderobertis/notignored/compare/v0.1.2...v0.1.3) - 2026-08-07

### Changed

- recompose notignored as an Nx monorepo ([#16](https://github.com/nickderobertis/notignored/pull/16))

## [0.1.2](https://github.com/nickderobertis/notignored/compare/v0.1.1...v0.1.2) - 2026-08-07

### Added

- ship notignored-cli on PyPI and npm ([#9](https://github.com/nickderobertis/notignored/pull/9))

## [0.1.1](https://github.com/nickderobertis/notignored/compare/v0.1.0...v0.1.1) - 2026-08-07

### Fixed

- *(action)* unbreak the scan step on macOS, and three follow-ups ([#6](https://github.com/nickderobertis/notignored/pull/6))

## [0.1.0](https://github.com/nickderobertis/notignored/releases/tag/v0.1.0) - 2026-08-07

### Added

- PR-comment GitHub Action, dogfooded on this repo
- parse eslint, biome and typescript suppressions
- report mypy, pyright, and ty suppressions
- report only the suppressions a change added with --diff
- parse ruff suppressions natively behind a fixed record contract

### Changed

- tighten input validation, error guidance, and pin sources
- address the llmlint judge tier's findings

### Documentation

- drop comments that restate AGENTS.md and the code below them
- say that exit 2 covers a bad argument too

### Fixed

- treat a closed stdout pipe as normal, not a write failure
- declare the real MSRV and prove it in CI
