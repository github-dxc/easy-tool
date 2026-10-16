# easy-tool

一个基于 Rust 与 Slint 构建的 Windows 桌面效率工具，提供时间戳转换、剪贴板历史、腾讯云文本翻译与系统托盘功能。

## 功能特性

- 时间戳转换：复制时间戳或日期后自动显示转换窗口，支持秒、毫秒与多个常用时区。从主页打开时默认显示当前时间并每秒更新，手动编辑后停止自动刷新。
- 剪贴板历史：记录文本、图片与文件列表，保留最近 20 条；支持查看、删除、粘贴和本地持久化，图片可在历史窗口内预览。
- 文本翻译：使用腾讯云 TMT API，自动判断中译英或英译中；支持复制触发、手动输入和翻译防抖。
- 系统托盘：左键打开主页，右键切换功能开关或退出应用。
- 设置：管理功能开关、腾讯云访问密钥和翻译防抖时间，修改后自动保存。

## 技术栈

Rust 2024、Slint、rdev、clipboard-master、arboard、tray-icon、windows-sys 与腾讯云 TMT API。

## 环境要求

- Windows 10/11
- Rust stable toolchain
- 可用的 C/C++ 构建环境，例如 Visual Studio Build Tools

## 快速开始

运行项目：

```powershell
cargo run
```

构建发布版本：

```powershell
cargo build --release
```

发布产物位于 `target/release/easy-tool.exe`。

首次运行后，程序会在 `%APPDATA%/easy-tool/config.toml` 创建配置文件。

## 使用方法

启动程序后，应用常驻系统托盘；左键点击托盘图标打开主页。

| 快捷键 | 功能 |
| --- | --- |
| `Ctrl + C` | 复制后触发时间戳转换；开启文本翻译时也会触发翻译 |
| `Ctrl + Shift + C` | 打开剪贴板历史 |

时间戳转换和剪贴板历史默认开启，复制触发的文本翻译默认关闭。也可以从主页手动打开各工具。

## 腾讯云文本翻译配置

文本翻译调用腾讯云 TMT `TextTranslate` 接口，使用时需要联网并开通对应服务。

1. 打开设置，在“腾讯云翻译 API 配置”中填写 `SecretId` 和 `SecretKey`。
2. 开启“文本翻译 - Ctrl+C”后，复制文本即可触发翻译。
3. 也可从主页打开文本翻译窗口手动输入文本，在设置中调整翻译防抖时间。

旧版本配置文件仍可读取，已废弃的功能和模型配置会被忽略，并在下一次保存设置时清理。已有剪贴板历史沿用可执行文件旁的 `temp/screenshot` 缓存目录，以兼容原有图片记录。

## 项目结构

```text
easy-tool/
├── assets/          # 图标、字体等静态资源
├── src/             # Rust 源码
│   ├── app/         # 启动、控制器、窗口管理与快捷键
│   ├── features/    # 时间戳、剪贴板、翻译、主页与设置
│   ├── infrastructure/ # 托盘、监听、腾讯云 API 与日志
│   └── platform/    # Windows 窗口与消息框适配
├── ui/              # Slint UI 文件
├── tests/           # 测试代码
├── build.rs         # Slint 编译与 Windows 资源配置
├── Cargo.toml       # Rust 项目配置
└── README.md
```

## 开发命令

```powershell
cargo check --all-targets
cargo test --lib
cargo fmt
cargo clippy
```

## 注意事项

- 文本翻译需要在设置中配置腾讯云 `SecretId` 和 `SecretKey`。
- 全局快捷键依赖系统输入监听，可能受到安全软件或权限策略影响。
- 应用常驻模式限制为单实例。

## License

本项目使用 MIT License 开源，详情请查看 [LICENSE](LICENSE)。

## 架构约定

- `app/` 负责服务组装和功能协调；后台事件通过消息通道送到 UI 线程。
- `features/` 按工具划分，窗口代码负责界面绑定，服务负责业务操作。
- `infrastructure/` 封装系统监听和外部 API，不直接持有功能窗口。
- 设置窗口和托盘统一通过应用控制器提交配置，保存成功后才更新运行状态。
- 复制触发使用非阻塞定时器，后台读取一次文本后供时间转换和翻译共用。
- Rust 函数使用中文注释；涉及线程切换、请求取消、文件清理边界时补充原因说明。
- `tests/window_test.rs` 是需要真实 Windows 桌面的手动托盘测试，默认忽略。
