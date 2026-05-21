#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use easy_tool::*;

// Keep the binary entry point minimal; all startup work lives in `app::run`.
fn main() {
    run();
}
 