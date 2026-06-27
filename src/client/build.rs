// 编译期把 ui/app.slint 编译为 Rust 代码（由 slint::include_modules!() 引入）。
fn main() {
    slint_build::compile("ui/app.slint").unwrap();
}
