# reedblog

**English** | [中文](README.md)

A lightweight blog system written in Rust, with plugin & theme extensibility.

## Features

**Core blogging**

- Web-based install wizard: initialize the site from the browser on first visit, with **SQLite** (zero-config) or **MySQL**
- Post management: Markdown editing (GFM tables, strikethrough, task lists), draft/published states, custom or auto-generated slugs, automatic plain-text excerpts (≤200 chars) when omitted
- Categories, tags, monthly archives; paginated post lists
- Comments: publish-first (visible immediately), with hide/restore/delete moderation in the admin panel
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
│   └── tests/                # Integration tests (integration, extensibility)
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
# Backend: unit tests + integration tests (full API flows and extensibility acceptance)
cd backend
cargo test
# Optionally generate example plugin/theme zips into tests/fixtures/
cargo test --test extensibility generate_fixtures -- --ignored

# Frontend: type checking and production build
cd frontend
npm run typecheck     # tsc --noEmit
npm run build         # tsc --noEmit && vite build
```

Port conventions: backend 3000 (configurable via `[server]` in config.toml);
frontend dev server 5173 with `/api` proxied to the backend.

## License

[GPL-3.0](LICENSE)
