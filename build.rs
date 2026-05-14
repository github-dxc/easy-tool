// Build script for compiling Slint UI resources and embedding Windows metadata.

fn main() {
    // 在构建阶段生成 Slint 的 Rust 代码，并通过 `cargo:rustc-env`
    // 将 `SLINT_INCLUDE_GENERATED` 环境变量传递给编译器，
    // 以供 `slint::include_modules!()` 在编译期包含生成的文件。
    // slint_build::compile("ui/app.slint").unwrap();

    // slint打包文件到二进制中
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles),
    )
    .unwrap();

    // 仅在 Windows 平台编译时生效，增加windows应用图标
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/icon.ico"); // 你的 ico 路径
        res.compile().unwrap();
    }
}
