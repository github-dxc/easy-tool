fn main() {
    // 在构建阶段生成 Slint 的 Rust 代码，并通过 `cargo:rustc-env`
    // 将 `SLINT_INCLUDE_GENERATED` 环境变量传递给编译器，
    // 以供 `slint::include_modules!()` 在编译期包含生成的文件。
    slint_build::compile("ui/app.slint").unwrap();
}
