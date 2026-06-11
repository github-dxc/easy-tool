# easy-tool

一个基于 Rust 与 Slint 构建的 Windows 桌面效率工具，提供时间戳转换、剪贴板历史、截图标注、文本翻译、图片预览、OCR 识别与 Base64 转换等能力，支持本地模型与腾讯云 API 两种后端。

## 功能特性

- 时间戳转换：复制时间戳后自动弹出转换窗口，支持多个常用时区。
- 剪贴板历史：记录文本剪贴板内容，便于快速回看与复用。
- 截图工具：支持快捷键唤起截图窗口，并提供基础标注能力（矩形、椭圆、箭头、画笔、马赛克、文字）。
- 图片预览：支持从主界面或系统右键菜单打开图片预览，侧边栏可展开翻译面板。
- 图片文字识别：支持本地 PaddleOCR-VL ONNX 模型推理，也可切换为腾讯云 OCR API。
- 文本翻译：支持本地 ONNX 翻译模型或腾讯云 TMT 机器翻译 API，复制即触发翻译。
- Base64 转换：支持文本 Base64 编码与解码，输入即时转换，一键切换模式。
- 系统托盘：通过托盘菜单快速打开主界面和切换功能状态。

## 技术栈

- Rust 2024
- Slint
- ONNX Runtime
- tokenizers
- rdev
- tray-icon
- windows-sys
- Noto Sans（Unicode 全字符覆盖字体）

## 环境要求

- Windows 10/11
- Rust stable toolchain
- 可用的 C/C++ 构建环境，例如 Visual Studio Build Tools

## 快速开始

克隆项目后，在项目根目录执行：

```powershell
cargo run
```

构建发布版本：

```powershell
cargo build --release
```

发布产物默认位于：

```text
target/release/easy-tool.exe
```

首次运行后，程序会在用户配置目录创建配置文件：

```text
%APPDATA%/easy-tool/config.toml
```

也可以在应用的设置窗口中修改相关功能开关和模型目录。

## 使用方法

启动程序后，应用会常驻系统托盘。

常用快捷键：

| 快捷键 | 功能 |
| --- | --- |
| `Ctrl + C` | 复制内容后触发时间戳转换；开启文本翻译时也会触发翻译 |
| `Ctrl + Shift + C` | 打开剪贴板历史 |
| `Alt + Shift + Z` | 打开截图工具 |
| `Esc` | 取消当前截图 |

图片预览与 OCR：

1. 打开主界面的图片预览功能，或通过系统右键菜单使用 easy-tool 打开图片。
2. 点击侧边栏翻译按钮展开 OCR/翻译面板。
3. OCR 后端可在设置中切换为本地模型或腾讯云 API。

## OCR 模型来源

本项目的图片文字识别功能支持两种后端：

**本地模型**：使用 PaddleOCR-VL ONNX 模型，翻译模型使用 Helsinki-NLP 模型。

**腾讯云 API**：在设置页启用腾讯云后端，配置 SecretId 和 SecretKey 即可使用腾讯云 OCR 和 TMT 机器翻译服务。

模型来源：

```text
https://www.modelscope.cn/models/onnx-community/PaddleOCR-VL-1.5-ONNX
https://www.modelscope.cn/models/Xenova/opus-mt-en-zh
https://www.modelscope.cn/models/Xenova/opus-mt-zh-en
```

推荐将模型下载到项目内的默认目录：

```text
resource/image-recognition
```

或者在应用设置中手动选择模型目录。

当前程序期望 OCR 模型目录结构如下：

```text
resource/image-recognition/
├── tokenizer.json
└── onnx/
    ├── vision_encoder.onnx
    ├── embedding.onnx
    └── decoder_q4.onnx
```

也可以使用 `decoder.onnx` 替代 `decoder_q4.onnx`。

如果下载后的模型文件名与上方结构不一致，请以程序实际校验的文件名为准进行调整。

## 文本翻译模型

文本翻译功能默认关闭。启用后，可在设置中选择后端：

**本地模型**：配置 ONNX 翻译模型目录，默认路径约定为：

```text
resource/Xenova/opus-mt-zh-en
resource/Xenova/opus-mt-en-zh
```

**腾讯云 TMT**：在设置页启用腾讯云后端，配置 SecretId 和 SecretKey 即可使用云端翻译服务。

每个翻译模型目录应包含：

```text
tokenizer.json
onnx/encoder_model.onnx
onnx/decoder_model.onnx
```

## 项目结构

```text
easy-tool/
├── assets/          # 图标、字体等静态资源
├── src/             # Rust 源码
├── ui/              # Slint UI 文件
├── tests/           # 测试代码
├── build.rs         # Slint 编译与 Windows 资源配置
├── Cargo.toml       # Rust 项目配置
└── README.md
```

## 开发命令

运行项目：

```powershell
cargo run
```

运行测试：

```powershell
cargo test
```

格式化代码：

```powershell
cargo fmt
```

检查代码：

```powershell
cargo clippy
```

## 注意事项

- OCR 与文本翻译支持本地模型和腾讯云 API 两种后端，可在设置页切换。
- 使用腾讯云 API 需在设置页配置 SecretId 和 SecretKey。
- 本地模型文件不会提交到仓库，建议自行下载并放入 `resource/` 目录。
- 首次启动会尝试注册 Windows 文件右键菜单。
- 全局快捷键依赖系统输入监听，部分安全软件或权限策略可能会影响监听效果。

## License

本项目使用 MIT License 开源，详情请查看 [LICENSE](LICENSE)。
