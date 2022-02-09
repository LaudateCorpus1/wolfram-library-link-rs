# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] – 2022-02-08

### Fixed

* Update `wstp` dependency to fix <docs.rs> build failures caused by earlier versions of
  `wstp-sys`.  ([#16])
* Fix missing `"full"` feature needed by the `syn` dependency of
  `wolfram-library-link-macros`.  ([#16])

## [0.1.0] – 2022-02-08

Initial release. `wolfram-library-link-sys` was the only crate published in this release,
due to a [docs.rs build failure](https://docs.rs/crate/wolfram-library-link-sys/0.1.0)
caused by bugs present in early versions of `wolfram-app-discovery` and `wstp-sys`.

### Added

* [`Link`](https://docs.rs/wstp/0.1.3/wstp/struct.Link.html) struct that represents a
  WSTP link endpoint, and provides methods for reading and writing symbolic Wolfram
  Language expressions.

* [`LinkServer`](https://docs.rs/wstp/0.1.3/wstp/struct.LinkServer.html) struct that
  represents a WSTP TCPIP link server, which binds to a port, listens for incoming
  connections, and creates a new `Link` for each connection.




[#16]: https://github.com/WolframResearch/wolfram-library-link-rs/pull/16


<!-- This needs to be updated for each tagged release. -->
[Unreleased]: https://github.com/WolframResearch/wolfram-library-link-rs/compare/v0.1.1...HEAD

[0.1.1]: https://github.com/WolframResearch/wolfram-library-link-rs/releases/tag/v0.1.1
[0.1.0]: https://github.com/WolframResearch/wolfram-library-link-rs/releases/tag/v0.1.0