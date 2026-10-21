# Removed browser dashboard

The React dashboard, SwiftUI apps and HTTP application server have been removed.
The supported operator interface is `cargo run -p argos-gpui`.

Use [Getting started](getting-started.md) for the native app and
[Architecture](DESIGN.md) for the direct Zenoh command path.
The [GCP router guide](https://github.com/Super-Yojan/ARGOS/blob/main/packaging/gcp/README.md)
covers authenticated TLS hosting and migration from the retired service.

The former OAuth login, viewer/operator HTTP roles, API endpoints, server-side
control claims, generated TypeScript contracts and SQLite audit service are no
longer available. Docker Compose now starts a local development router only.
Historical screenshots and plans describe the former interface, not current behavior.
