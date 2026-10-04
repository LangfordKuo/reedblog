# reedblog

**English** | [中文](README.md)

A lightweight blog system written in Rust, with plugin & theme extensibility (Rust/Axum backend + React frontend, SQLite or MySQL).

## Quick start (Docker)

Images are published to GHCR (`linux/amd64` + `linux/arm64`). **One command brings up the whole stack** (Nginx and the backend share a container):

```bash
docker run -d --name reedblog -p 8080:80 -v reedblog-data:/data \
  --restart unless-stopped ghcr.io/langfordkuo/reedblog:latest
```

Open **http://localhost:8080** → your first visit goes into the `/install` wizard (pick SQLite — zero configuration).
All data lives in the `reedblog-data` volume (`config.toml`, SQLite, `uploads/`, `plugins/`, `themes/`), so removing the
container loses nothing; to upgrade, `docker pull`, remove the old container and re-run the same command with the same volume.

> Inside this image Nginx runs as root (it must bind port 80). For process isolation with a non-root backend (uid 10002), use the options below.

Other deployment options:

| Option | How |
|---|---|
| Two images, separate containers | `reedblog-backend` + `reedblog-web`; create a network and point `BACKEND_UPSTREAM` at the backend container — see [DOCKER.md](deploy/DOCKER.md) |
| docker compose (recommended for the long run) | `docker compose pull && docker compose up -d`; MySQL via `--profile mysql` |
| Run from source | see [Run from source](#run-from-source-local-development) |
| Self-hosted without Docker | see [Production deployment](#production-deployment-nginx) |

Image tags: `vX.Y.Z` / `X.Y` / `latest` / `sha-<short-sha>`. Full guide (three-image trade-offs, volumes and backups, switching to MySQL, environment variables, upgrades, FAQ): [deploy/DOCKER.md](deploy/DOCKER.md).

## Features

**Content**: Markdown editing (GFM tables/strikethrough/task lists), draft & published states, custom or auto-generated slugs, automatic excerpts; categories, tags, monthly archives, pagination; full-text search (LIKE-based, works on SQLite and MySQL with zero migrations)

**Comments**: threaded replies; **switchable moderation** (publish-first / approve-first); anti-abuse (per IP+target rate limiting, honeypot, keyword blocklist, login back-off); SMTP notifications (password read only from config or env var — never stored in the database, never returned by any API)

**Media**: toolbar/paste/drag-and-drop uploads (real type via magic bytes, sha256 deduplication, SVG rejected); admin media library (grid browsing, copy URL, delete)

**Presentation**: code highlighting, KaTeX math, dark mode (light/dark/system, applied before first paint), related posts, previous/next post, reading progress, likes and view counts, RSS, sitemap, SEO crawler routing (crawlers and social previews get OG/Twitter/JSON-LD)

**Admin**: dashboard, posts and pages, revision history, trash, comments, categories & tags, media library, themes, plugins, site settings, backup export/import

**Extensibility**: a plugin = directory + [Rhai](https://rhai.rs/) script + manifest, install by uploading a zip (4 backend hooks + frontend `head`/`body_end` injection, sandboxed with no file/network/process access); a theme = design tokens + CSS + assets, upload and activate with one click (32 tokens covering the shadcn/Tailwind variables, builtin `default` theme cannot be deleted). See the [plugin](docs/plugin-development.md) and [theme](docs/theme-development.md) guides (in Chinese)

**Engineering**: uniform REST API (standardized error shape, pagination, RFC3339 UTC); JWT (HS256, 7 days) + argon2; install wizard; not-installed gate (every API outside the whitelist returns 503 before initialization)

## Tech stack

| Layer | Technologies |
|---|---|
| Backend | Rust · [Axum](https://github.com/tokio-rs/axum) 0.8 · tokio · [sqlx](https://github.com/launchbadge/sqlx) 0.8 (Any driver: SQLite/MySQL) · pulldown-cmark · [Rhai](https://rhai.rs/) (plugin sandbox) · lettre (SMTP) |
| Frontend | React 19 · TypeScript · Vite 8 · Tailwind CSS 4 · shadcn/ui-style components (Radix UI) · react-router 7 · KaTeX · highlight.js |

## Run from source (local development)

Requirements: Rust stable (with cargo) and Node.js `^20.19.0 || >=22.12.0`.

```bash
cd backend  && cargo run                      # backend on 127.0.0.1:3000
cd frontend && npm install && npm run dev     # frontend on 5173, /api proxied to 3000
```

Open **http://localhost:5173** → if the site is not installed yet you land in the install wizard: pick a database
(SQLite is zero-config, or MySQL) → set the admin account and site info → the backend writes `backend/config.toml`
(no restart needed). Admin panel: `/admin/login`.

## Production deployment (Nginx)

```bash
cd backend  && cargo build --release     # → backend/target/release/reedblog-backend
cd frontend && npm ci && npm run build   # → frontend/dist/
```

Serve `dist/` from Nginx (SPA fallback to `index.html`) and reverse-proxy `/api` to the backend, which keeps listening on
`127.0.0.1:3000` (same-origin proxying means no CORS changes). The full snippet including SEO crawler routing lives in
[`deploy/nginx.conf.example`](deploy/nginx.conf.example); a three-step deployment guide for the release packages is in
[`deploy/QUICKSTART.md`](deploy/QUICKSTART.md). Supervise the backend with systemd and keep a fixed working directory
(`config.toml`, `plugins/`, `themes/` and the SQLite file are all resolved relative to it); first deploy goes through `/install`.

## Configuration

`config.toml` defaults to the working directory and can be relocated via the `REEDBLOG_CONFIG` environment variable
(the install wizard writes back to that file).

| Key | Description |
|---|---|
| `[server] host` / `port` / `base_url` | Bind address and port; when `base_url` is empty the absolute URL is derived from proxy headers (X-Forwarded-Proto/Host) |
| `[database] db_type` / `sqlite_path` / `[database.mysql]` | SQLite file path, or MySQL connection details |
| `[auth] jwt_secret` | JWT secret (generated during install); empty means not installed |
| `[cors] allowed_origins` | Only needed when the frontend is served from another origin; leave as is behind a same-origin proxy |
| `[plugins]` / `[themes]` / `[uploads]` | Plugin, theme and upload directories; `[uploads] max_size_mb` caps a single file |
| `[smtp] password` | Read only from here or from `REEDBLOG_SMTP_PASSWORD` (takes precedence); never stored in the database |

## Project layout

```
backend/   Rust backend: handlers/, plugins.rs (plugin host), themes.rs (theme system), seo.rs (crawler routing),
           migrations/ (sqlite + mysql), tests/ (integration tests)
frontend/  React frontend: src/pages (public + /install + /admin), components/, lib/ (API client, theme application)
docs/      API contract and development docs      deploy/  deployment snippets and Docker docs      examples/  example plugin & theme
```

## Documentation

| Document | Description |
|---|---|
| [docs/api-contract.md](docs/api-contract.md) | Core API contract v1 (endpoints, data shapes, error codes; in Chinese) |
| [docs/extensibility-contract.md](docs/extensibility-contract.md) | Extensibility contract v1 (plugin + theme spec; in Chinese) |
| [docs/plugin-development.md](docs/plugin-development.md) | Plugin development guide (in Chinese) |
| [docs/theme-development.md](docs/theme-development.md) | Theme development guide (in Chinese) |
| [deploy/DOCKER.md](deploy/DOCKER.md) | Full Docker deployment guide |
| [deploy/QUICKSTART.md](deploy/QUICKSTART.md) | Three-step deployment for release packages |
| [examples/](examples/) | Example plugin demo-suite, example theme midnight |

## Development and CI

```bash
cd backend  && cargo fmt --check && cargo test --all-targets
cd frontend && npm ci && npm run typecheck && npm run build && npm test
```

On every push to `main` or pull request, GitHub Actions runs the backend (format gate + full test suite) and the frontend
(type check + build + vitest unit tests) in parallel — see [ci.yml](.github/workflows/ci.yml). Tagging adds two release
pipelines: [release.yml](.github/workflows/release.yml) builds the three-platform binaries and
[docker.yml](.github/workflows/docker.yml) builds and pushes the three images (amd64 + arm64). No secrets are required.

## License

[GPL-3.0](LICENSE)
