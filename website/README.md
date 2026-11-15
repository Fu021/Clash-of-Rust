# 产品网站

首页、图文功能介绍和下载页，沿用客户端的暖灰深色、陶土强调色与几何字标。网站独立于客户端，不加载 WebView、不连接本机控制器，也不影响代理或 TUN。

## 本地构建与预览

在仓库根目录使用 Python 3.11+，无需安装依赖：

```bash
python3 -B website/build.py
python3 -m http.server 8080 --bind 127.0.0.1 --directory website/dist
```

浏览器打开 `http://127.0.0.1:8080`。`website/dist/` 是忽略的静态输出；不要直接编辑。浏览器截图工具在仓库外维护，输出整页长图，不提交预览产物。

```bash
python3 -B website/test_build.py
node --check website/static/site.js
```

构建回归覆盖发行身份、九组安装包与校验文件、错误元数据、源目录保护及离线构建；浏览器检查另覆盖导航、图片放大、目录高亮、下载通道、平台筛选、360px 布局及接口失败回退。截图检查使用公开发行快照，不下载或运行安装包。

## 维护位置

| 文件 | 职责 |
| --- | --- |
| `layout.html`、`pages/` | 共享导航、页脚、三页内容 |
| `static/site.css`、`static/site.js` | 响应式深色界面与浏览器交互 |
| `static/screenshots/` | 正式组件的演示配图，注明演示数据 |
| `build.py`、`test_build.py` | 标准库构建及离线回归 |
| `releases.json` | 正式版与预发布版的公开发行快照 |
| `_headers` | 静态托管的安全响应头 |

几何字标、图标与 GPL 许可证由构建器从仓库复制；中文使用系统字体，无远程字体或追踪脚本。功能说明应链接到 [docs](../docs/README.md) 的权威主题；新功能标明正式版或预发布版范围，不把 `main` 上的版本视为已经发布。

## 下载信息

```bash
python3 -B website/build.py --refresh-releases
```

只读取公开 GitHub API，验证仓库、标签、通道、资产状态和九包／九 SHA256 链接后更新快照；失败不覆盖原文件。发布新版本后应刷新快照并重新构建。

下载页还会按需读取 GitHub 的当前公开发行信息。接口失败、限流或内容不完整时保留已保存下载；不展示残缺的新版本。禁用 JavaScript 时仍提供正式版九包及校验链接，隐藏需要脚本的筛选和切换按钮。下载直接前往 GitHub，网站不代理或存储安装包。

## 部署候选

当前尚未部署，也没有新增 Actions 或自动发布入口。Cloudflare Pages 作为候选可采用：

| 设置 | 值 |
| --- | --- |
| 项目根目录 | 仓库根目录 |
| 构建命令 | `python3 -B website/build.py` |
| 输出目录 | `website/dist` |

保留仓库根目录是为了复制品牌与许可证资源。`_headers` 随输出复制；正式上线时另验域名、响应头、重定向和真实 GitHub API 的浏览器请求。配置依据：[静态 HTML](https://developers.cloudflare.com/pages/framework-guides/deploy-anything/)、[构建配置](https://developers.cloudflare.com/pages/configuration/build-configuration/)和[响应头](https://developers.cloudflare.com/pages/configuration/headers/)。
