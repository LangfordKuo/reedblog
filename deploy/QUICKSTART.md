# 快速部署（release 包内文件说明）

本包由 GitHub Actions 在打 tag 时自动构建，含三部分：后端可执行文件、前端静态站点、Nginx 参考配置。

```
reedblog-backend[.exe]      后端（默认监听 127.0.0.1:3000）
frontend-dist/              前端静态站点（构建产物，交给 Nginx/任意静态服务器托管）
deploy/nginx.conf.example   Nginx 参考片段（含 SPA fallback、/api 反代、SEO 爬虫分流）
VERSION.txt                 版本 / 目标平台 / 构建时间 / Git SHA
README.md、README.en.md     项目文档
```

## 三步上线（Linux）

```bash
# 1) 放置文件
sudo mkdir -p /opt/reedblog
sudo tar -xzf reedblog-*-linux-*.tar.gz -C /opt/reedblog --strip-components=1

# 2) 启动后端（首次启动会自动建 config.toml 与数据库目录要求，见下）
cd /opt/reedblog
./reedblog-backend                      # 默认读同目录 config.toml；可用 REEDBLOG_CONFIG 指定路径

# 3) 托管前端 + 反代（Nginx）
sudo mkdir -p /var/www/reedblog
sudo cp -r frontend-dist /var/www/reedblog/dist
# 把 deploy/nginx.conf.example 的 map/server 段并入站点配置，root 指向 /var/www/reedblog/dist
sudo nginx -t && sudo systemctl reload nginx
```

浏览器打开站点，首次访问 `/install` 完成安装向导（填数据库路径、管理员账号、站点信息），
向导会写出 `config.toml`（含自动生成的 `jwt_secret`）。

## Windows

```powershell
Expand-Archive reedblog-*-windows-*.zip -DestinationPath C:\reedblog
cd C:\reedblog
.\reedblog-backend.exe        # 默认读同目录 config.toml
```
前端 `frontend-dist\` 可用 Nginx for Windows、Caddy 或任意静态服务器托管；
开发/临时预览也可用 `npx serve frontend-dist --single`。

## 注意

- 数据库：SQLite（默认，文件路径在 `config.toml`）或 MySQL（安装向导里选）。SQLite 为**内置编译**，无需系统安装 sqlite。
- 反向代理必须把 `/api` 转发到后端；后端默认只监听 `127.0.0.1`，如需直接暴露改 `config.toml [server] host`。
- SMTP 密码只从 `config.toml [smtp] password` 或环境变量 `REEDBLOG_SMTP_PASSWORD` 读取，不入库、不经 API 返回。
- 升级：替换可执行文件后重启进程即可；数据库迁移在后端启动时自动执行。
- 备份：后台「备份」页可导出 zip（数据 + 媒体）。
