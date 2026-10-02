# assets/ —— 主题静态资源目录

把字体、图片等静态文件放在这里，后端按相对路径托管：

```
主题目录内路径                        →  访问 URL
assets/inter-var.woff2               →  /api/themes/midnight/assets/inter-var.woff2
assets/img/hero.png                  →  /api/themes/midnight/assets/img/hero.png
```

在 `theme.css` 中用**绝对路径**引用（相对路径不会指向这里）：

```css
@font-face {
  font-family: "Midnight Sans";
  src: url("/api/themes/midnight/assets/inter-var.woff2") format("woff2");
}
body::before {
  content: "";
  background: url("/api/themes/midnight/assets/img/hero.png") center/cover;
}
```

注意事项：

- MIME 类型按扩展名识别（css/js/json/txt/html/png/jpg/gif/svg/webp/ico/avif/
  woff/woff2/ttf/otf/eot），未知扩展名按 `application/octet-stream` 下发；
- 路径经过防目录穿越校验（拒绝 `..`、绝对路径、盘符与符号链接），只能读取
  `assets/` 内的文件；
- 该端点在未安装门禁白名单内（GET），未安装站点也可访问；
- 本 README 仅为说明文件，打包进 zip 也不影响主题运行。
