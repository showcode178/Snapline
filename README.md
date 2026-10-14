# Snapline · 晾图

原生 Windows 截图晾衣绳，使用 **Rust + Slint + Windows API**。灵感来自 [Tendedero](https://github.com/alejandrobujan/tendedero)。本项目使用独立名称和原创图标。

适用于 Windows 10 / 11 的 x64 电脑。界面默认中文，可在设置里切换英文。

## 使用

从 [GitHub Releases](https://github.com/showcode178/Snapline/releases/latest) 下载 `Snapline-<版本>-Setup.exe` 安装，或下载 `Snapline-<版本>-Windows-x64.zip`，解压后双击 `Snapline.exe` 使用便携版。安装到当前用户目录，无需管理员权限。

1. 按 **Win + Shift + S** 截图，截图进入剪贴板后会短暂闪光、发出快门声，再缩小挂到屏幕顶部并轻摆。也可以点「导入」添加本地图片。
2. 鼠标靠近屏幕顶部展开晾衣绳；绳子自动覆盖当前显示器的宽度，分辨率或缩放变化后同步调整。点「收起」或 **Ctrl + Alt + T** 切换显示。
3. 双击截图打开内置编辑器，右键复制图片。悬停时可编辑、保存副本或移除。
4. 从截图拖出，可作为文件放入资源管理器或支持文件拖入的软件。目标软件是否接收图片由其自身决定。
5. 在「设置」中调整语言、主题、图片尺寸、保留数量、截图目录监听与开机启动，也可关闭快门声；托盘菜单可打开设置或退出。动效遵循 Windows 的动画效果设置。

截图与设置保存在当前用户的本地应用数据目录，卸载应用保留这些数据。移除截图影响应用自己的副本，不会删除导入的原文件。

图片按时间从左到右排列，最新的在右侧。超过最大保留数量时，最早的一张先脱离衣夹、下落消失；掉落结束后，其余图片才向左平滑补位。连续挂图时，等所有正在掉落的图片结束，再统一补位。

编辑器支持画笔、矩形、箭头、裁剪和撤销。点击「保存」会替换绳上的当前图片并更新缩略图，不会新增一张；「保存副本」可另外导出 PNG。导入的来源文件保持不变。视频可导入、复制为文件、拖放和用默认播放器打开；当前版本不内置视频播放、视频缩略图生成或视频编辑。透明窗口使用 OpenGL 渲染，需要可用的显卡驱动。

## 开发

需要 Rust 的 `x86_64-pc-windows-msvc` 工具链，以及 Visual Studio Build Tools 的「使用 C++ 的桌面开发」组件和 Windows SDK。支持已有 Visual Studio 开发环境，也支持通过 `SNAPLINE_MSVC_ROOT` 指定便携 MSVC 目录。

```powershell
. .\scripts\windows-env.ps1
cargo run
```

本机工具和输出目录可通过 `.tools/msvc-path.txt`、`.tools/cargo-target.txt` 配置；这些文件不纳入版本控制。

发行构建使用静态 C 运行库，并生成便携 ZIP 与 NSIS 安装包：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1
```

需要 NSIS 3；可设置 `SNAPLINE_NSIS_EXE` 指向 `makensis.exe`。应用运行时不需要 WebView2、浏览器或 .NET。

## GitHub Actions 安装包

推送到 `main` 后，[Windows installer 工作流](https://github.com/showcode178/Snapline/actions/workflows/windows-installer.yml) 会在 Windows x64 环境构建 NSIS 安装包和便携 ZIP。也可在工作流页面点击 **Run workflow** 手动构建。

打开成功的运行记录，在 **Artifacts** 中下载 `Snapline-<版本>-Setup` 或 `Snapline-<版本>-Windows-x64`，解压 GitHub 的产物压缩包后即可得到安装程序或便携 ZIP。下载 Actions 产物需要登录 GitHub；产物保留 30 天。版本号自动读取 `Cargo.toml`，依赖使用 `Cargo.lock` 锁定。

正式发布时，先更新 `Cargo.toml` 和 `Cargo.lock` 中的版本并提交，然后创建、推送对应的 `v<版本>` 标签（例如 `git tag v0.1.0`、`git push origin v0.1.0`）。标签版本必须与 `Cargo.toml` 一致；标签构建成功后自动创建 GitHub Release，直接附上安装程序与便携 ZIP。Release 附件没有 Actions 的 30 天保留限制，公开仓库无需登录即可下载。普通分支推送只生成 Actions 产物，不创建 Release。

## 授权

项目代码采用 MIT 许可，见 [LICENSE](LICENSE)。保留原项目作者的署名及名称、图标条款；本项目不复用原版名称、图标或宣传图片，也不代表原作者认可。

Slint 使用 Royalty-free Desktop, Mobile, and Web Applications License 2.0，授权全文见 [SLINT_LICENSE.md](SLINT_LICENSE.md)。设置里的「关于」包含 Slint 官方署名控件。依赖信息见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
