# reedblog

**English** | [中文](README.md)

A lightweight blog system written in Rust, with plugin & theme extensibility.

## Features

**Core blogging**

- Web-based install wizard: initialize the site from the browser on first visit, with **SQLite** (zero-config) or **MySQL**
- Post management: Markdown editing (GFM tables, strikethrough, task lists), draft/published states, custom or auto-generated slugs, automatic plain-text excerpts (≤200 chars) when omitted
- Categories, tags, monthly archives; paginated post lists
- Comments: publish-first (visible immediately), with hide/restore/delete moderation in the admin panel
- Image uploads: toolbar button, paste, or drag-and-drop in the editor (PNG/JPEG/GIF/WebP, real type detected via magic bytes, SVG rejected, 10MB default limit, configurable), sha256 content-hash deduplication, served via `/api/uploads/*` with immutable caching
- RSS feed (`/api/feed.xml`, latest 20 posts) and sitemap (`/api/sitemap.xml`); absolute site URL from `[server] base_url` when set, otherwise derived from reverse-proxy headers (X-Forwarded-Proto/Host)
- Full-text search: `GET /api/search` built on LIKE (works on both SQLite and MySQL with zero migrations; whitespace-split AND terms, wildcard escaping, published only); the shareable `/search` page highlights hits with `<mark>` and shows plain-text context snippets
- Email notifications (SMTP): a notification email is sent to the admin mailbox after each new comment/reply (post/page title with absolute link, author, content, admin comments link); the admin "Email" page configures SMTP (STARTTLS/implicit TLS/plaintext) and sends test emails. **The password is read only from `config.toml` `[smtp] password` or the `REEDBLOG_SMTP_PASSWORD` env var — never stored in the database, never returned by any API**; sending is asynchronous (~10s timeout, failures are logged only) and never blocks comment publishing
- Dark mode toggle: tri-state light/dark/system preference (persisted in localStorage, defaults to system), applied before first paint by an inline `<head>` script to prevent FOUC, with a one-click sun/moon toggle in the header
- Frontend: code highlighting (highlight.js), responsive layout
- Admin panel: dashboard, posts, categories, tags, comments, plugins, themes

**Plugin system** ([development guide](docs/plugin-development.md), in Chinese)

- A plugin = a directory + a [Rhai](https://rhai.rs/) script + manifest.toml. No compilation, no Rust knowledge required; install by uploading a zip
- 4 backend hooks: `post.before_render` / `post.after_render` / `comment.before_create` (can block comments) / `post.after_publish`; multiple plugins chain in slug-lexicographic order
- Light frontend injection: raw HTML fragments at `head` / `body_end` (`<script>` is executed — fits analytics snippets and widgets)
- Safe sandbox: no file/network/process capabilities; call depth, operation count, string and collection sizes are limited; script errors are skipped and recorded in `last_error`
- Hot switching: enable/disable/delete take effect immediately, no restart needed

**Theme system** ([development guide](docs/theme-development.md), in Chinese)

- A theme = design tokens (theme.toml) + custom CSS + static assets; upload a zip and activate with one click
- 32 design tokens mapped onto the shadcn/Tailwind CSS variable system (plus an optional dark variant `[tokens_dark]`)
- Fonts/images served via `/api/themes/:slug/assets/*` (directory-traversal safe)
- The builtin `default` theme is auto-created and undeletable, so the site always has styling

**Engineering**

- Uniform REST API: standardized error shape, pagination, RFC3339 UTC timestamps
- JWT (HS256, 7-day) auth, argon2 password hashing
- Not-installed gate: before initialization every API outside the whitelist returns 503, and the frontend routes to the install wizard

## Tech stack

| Layer | Technologies |
|---|---|
| Backend | Rust · [Axum](https://github.com/tokio-rs/axum) 0.8 · tokio · [sqlx](https://github.com/launchbadge/sqlx) 0.8 (Any driver: SQLite/MySQL) · [rhai](https://rhai.rs/) 1.x (plugin sandbox) · pulldown-cmark (Markdown rendering) · zip · jsonwebtoken · argon2 |
| Frontend | React 19 · TypeScript 5.9 · Vite 8 · Tailwind CSS 4 · shadcn/ui-style components (Radix UI) · react-router-dom 7 · react-markdown + rehype-highlight · sonner |

## Quick start

Requirements:

- **Rust**: stable toolchain (with cargo)
- **Node.js**: `^20.19.0 || >=22.12.0` (required by Vite 8), with npm

```bash
# 1. Start the backend (listens on 127.0.0.1:3000 by default)
cd backend
cargo run

# 2. Start the frontend dev server (port 5173, /api proxied to 3000)
cd frontend
npm install
npm run dev
```

Open **http://localhost:5173** — you'll be routed to the install wizard:

1. Pick a database: SQLite (default, writes `backend/reedblog.db`, zero-config) or MySQL (enter connection details);
2. Set the admin username/password and the site title/subtitle;
3. Finish — the backend writes `backend/config.toml`, no restart required. Then visit
   the site or the admin panel (log in at `/admin/login`).

The config file defaults to `config.toml` in the working directory and can be
overridden via the `REEDBLOG_CONFIG` environment variable; listen address, CORS
origins, plugin/theme directories, etc. are all configured in config.toml.

## Production deployment

```bash
# Build the backend binary
cd backend
cargo build --release        # → backend/target/release/reedblog-backend(.exe)

# Build the frontend static bundle
cd frontend
npm ci
npm run build                # → frontend/dist/
```

Topology: serve `dist/` from Nginx (SPA fallback to `index.html`) and reverse-proxy
`/api` to the backend process running on `127.0.0.1:3000` (same-origin proxying
means no CORS concerns — leave `[cors] allowed_origins` as is).

Example Nginx configuration:

```nginx
server {
    listen 80;
    server_name blog.example.com;

    # Frontend static files + SPA fallback
    root /var/www/reedblog/dist;
    index index.html;
    location / {
        try_files $uri /index.html;
    }

    # Backend API reverse proxy
    location /api/ {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # The backend caps plugin/theme zip uploads at 32MB; keep Nginx aligned
    client_max_body_size 32m;
}
```

The full snippet including SEO crawler routing lives in
[`deploy/nginx.conf.example`](deploy/nginx.conf.example) (**not verified against a
real Nginx** — self-test before going live; see the header comments in that file).

Run the backend (keep a fixed working directory — `config.toml`, `plugins/`,
`themes/` and the SQLite database file are all resolved relative to it):

```bash
cd /opt/reedblog
REEDBLOG_CONFIG=/opt/reedblog/config.toml /usr/local/bin/reedblog-backend
```

Complete the install wizard at `https://blog.example.com/install` on first deploy.
The default bind address is `127.0.0.1`, suited to reverse-proxy setups; to expose
it directly, change `[server] host` in config.toml. In production, supervise the
backend with systemd or similar.

## Docker deployment

No Rust/Node/Nginx toolchain needed. Images are published to GHCR (`linux/amd64` + `linux/arm64`),
tags `vX.Y.Z` / `X.Y` / `latest` / `sha-<short-sha>`. Pick whichever of the three ways fits.

### 1. Easiest: the whole stack in one command (all-in-one image)

Nginx (frontend + `/api` reverse proxy + SEO crawler routing) and the backend share one container:

```bash
docker run -d --name reedblog -p 8080:80 -v reedblog-data:/data \
  --restart unless-stopped ghcr.io/langfordkuo/reedblog:latest
```

Open **http://localhost:8080** → your first visit goes straight into the `/install` wizard
(pick SQLite — zero dependencies). All data lives in the `reedblog-data` volume, so removing the
container loses nothing; to upgrade, `docker pull`, remove the old container and re-run the same
command with the same volume.

> ⚠️ Inside the all-in-one image **nginx runs as root** (it must bind port 80 and write its cache
> directories — that is the price of the one-command setup). For process isolation with a non-root
> backend (uid 10002), use the compose option below.

### 2. Two `docker run` commands (backend + web images)

```bash
docker network create reedblog                 # lets the two containers talk
docker volume create reedblog-data             # data volume (used by the backend)

docker run -d --name reedblog-backend --network reedblog --network-alias backend \
  -v reedblog-data:/data --restart unless-stopped \
  ghcr.io/langfordkuo/reedblog-backend:latest

docker run -d --name reedblog-web --network reedblog -p 8080:80 \
  -e BACKEND_UPSTREAM=backend:3000 --restart unless-stopped \
  ghcr.io/langfordkuo/reedblog-web:latest
```

Open **http://localhost:8080** → first visit goes through the `/install` wizard.
`BACKEND_UPSTREAM` is Nginx's proxy target and must be **backend container name:port**
(use `host.docker.internal:3000` to point at a backend running on the host) — a wrong value
here gives you 502s.

<details>
<summary>Backend API only (no frontend pages)</summary>

```bash
docker run -d --name reedblog-backend -p 3000:3000 -v reedblog-data:/data \
  --restart unless-stopped ghcr.io/langfordkuo/reedblog-backend:latest
```

Port 3000 returns API JSON only; there are no frontend pages and no SEO crawler routing
(OG cards need the web layer).
</details>

### 3. docker compose (recommended for the long run)

```bash
docker compose pull && docker compose up -d
```

Open **http://localhost:8080** → first visit goes through the `/install` wizard.
Two containers (the backend runs non-root, uid 10002) share a compose network; ports and volumes
live in `docker-compose.yml`, and all data lives in the named `/data` volume (config.toml, SQLite,
uploads, plugins, themes) — removing containers does not lose data. SQLite by default; MySQL via
`--profile mysql`.

Full guide (trade-offs between the three images, volumes and backups, switching to MySQL,
environment variables, upgrades, FAQ): [deploy/DOCKER.md](deploy/DOCKER.md).

## SEO / share metadata (crawler routing)

Crawlers of WeChat / Twitter / Facebook / Slack never execute JS, so SPA runtime
meta injection is invisible to them. Instead, Nginx routes by User-Agent
(see the "SEO / share metadata" section of the API contract):

- **Human traffic**: Nginx keeps serving the SPA bundle (unchanged); `frontend/src/lib/meta.ts`
  injects `document.title`, `meta[name=description]`, `og:title/og:description/og:url` and
  `link[rel=canonical]` at runtime (every managed tag carries `data-reedblog-meta="1"`).
- **Crawlers / social previews** (`bot|crawler|spider|facebookexternalhit|…|micromessenger|wechat`,
  case-insensitive): `/posts/*` and `/pages/*` are proxied to the backend, which returns a minimal
  HTML page with OG / Twitter / JSON-LD tags; `/robots.txt` is served by the backend too.
- Requests from unknown UAs that reach the backend directly get a **302 to the site root**.
- `og:image` fallback chain: first Markdown image in the post body → site setting `og_image`
  (new field in admin site settings) → tag omitted.
- Implementation: `backend/src/seo.rs` (rendering/escaping/UA whitelist) +
  `backend/src/handlers/seo.rs` (HTTP routes).

Self-test:

```bash
curl -sA "Twitterbot/1.0" http://127.0.0.1:3000/posts/<slug> | head -30   # OG HTML
curl -sA "Mozilla/5.0" -D - -o /dev/null http://127.0.0.1:3000/posts/<slug>  # expect 302
curl -s http://127.0.0.1:3000/robots.txt                                   # expect Sitemap:
```

## Project layout

```
reedblog/
├── backend/                  # Rust backend (Axum)
│   ├── src/
│   │   ├── handlers/         # API handlers (install/public/admin/plugins/themes)
│   │   ├── plugins.rs        # Plugin host: Rhai sandbox + hook chains + injections
│   │   ├── themes.rs         # Theme system: manifest, builtin default, static serving
│   │   ├── packages.rs       # zip extraction/validation, slug/semver helpers
│   │   ├── config.rs         # config.toml structure and I/O
│   │   ├── state.rs          # App state, DB connections & migrations
│   │   ├── auth.rs / error.rs / middleware.rs / models.rs
│   │   └── main.rs / lib.rs  # Entrypoint and router assembly
│   ├── migrations/           # sqlx migrations (sqlite/ and mysql/)
│   └── tests/                # Integration tests (integration, extensibility, uploads_feed)
├── frontend/                 # React frontend (Vite + Tailwind CSS 4)
│   └── src/
│       ├── pages/            # Public pages, /install wizard, /admin panel
│       ├── components/       # shadcn/ui and site components (incl. plugin injection)
│       └── lib/              # API client, theme application, type definitions
├── docs/                     # API contract and development docs
└── examples/                 # Example plugin / theme sources
```

## Documentation

| Document | Description |
|---|---|
| [docs/api-contract.md](docs/api-contract.md) | Core API contract v1 (endpoints, data shapes, error codes; in Chinese) |
| [docs/extensibility-contract.md](docs/extensibility-contract.md) | Extensibility contract v1 (plugin + theme spec shared by backend/frontend; in Chinese) |
| [docs/plugin-development.md](docs/plugin-development.md) | Plugin development guide (manifest, Rhai hooks, sandbox, injection, packaging; in Chinese) |
| [docs/theme-development.md](docs/theme-development.md) | Theme development guide (token list, theme.css, assets, packaging; in Chinese) |
| [examples/plugins/demo-suite/](examples/plugins/demo-suite/) | Example plugin: comment word filter + copyright footer + analytics injection |
| [examples/themes/midnight/](examples/themes/midnight/) | Example theme: dark theme covering all design tokens |

## Development

```bash
# Backend: format check + unit tests + integration tests (full API flows and extensibility acceptance)
cd backend
cargo fmt --check
cargo test --all-targets
# Optionally generate example plugin/theme zips into tests/fixtures/
cargo test --test extensibility generate_fixtures -- --ignored

# Frontend: type checking, production build and unit tests
cd frontend
npm ci
npm run typecheck     # tsc --noEmit
npm run build         # tsc --noEmit && vite build
npm test              # vitest run (pure-function and DOM unit tests under src/lib)
npm run test:watch    # vitest watch mode for local development
```

### CI and tests

On every push or pull request targeting `main`, GitHub Actions
([.github/workflows/ci.yml](.github/workflows/ci.yml)) runs two jobs in parallel:

- **Backend (Rust)**: `cargo fmt --check` gate + `cargo test --all-targets`;
  tests use temporary SQLite databases and need no external services or secrets.
- **Frontend (Node 24)**: `npm ci` → `npx tsc --noEmit` → `npm run build` → `npm test`
  (vitest + jsdom; cases live in `frontend/src/lib/*.test.ts` and cover the diff engine,
  math source preprocessing, TOC extraction, formatting helpers and API rate-limit error parsing).

No secrets are required. Workflow triggers are limited to `backend/**`, `frontend/**`
and `.github/workflows/**`, and duplicate runs on the same branch are cancelled automatically.

Port conventions: backend 3000 (configurable via `[server]` in config.toml);
frontend dev server 5173 with `/api` proxied to the backend.

## License

[GPL-3.0](LICENSE)
