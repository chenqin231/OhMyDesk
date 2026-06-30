// 编译期把 ui/app.slint 编译为 Rust 代码（由 slint::include_modules!() 引入）。
// Windows target 额外把 app.ico 嵌入 exe（任务栏/资源管理器图标）。
fn main() {
    slint_build::compile("ui/app.slint").unwrap();

    // build.rs 运行在 host（交叉编译时 host=Linux），必须按 target 而非 host 判断。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/app.ico");
        if let Err(e) = res.compile() {
            // warn-not-fail：缺 windres 等工具时不中断构建，仅告警（图标不嵌入）。
            println!("cargo:warning=winresource 图标嵌入失败: {e}");
        }
    }
}
