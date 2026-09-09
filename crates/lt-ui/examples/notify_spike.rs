//! Windows 原生通知实机验证（D-33/H-2）。
//!
//! 用法：`cargo run -p lt-ui --example notify_spike`
//! 触发完整链路：RoInitialize(STA) + SetCurrentProcessExplicitAppUserModelID、
//! 开始菜单快捷方式注册（首次）、Toast 发送。退出码 0 = 链路全链无错
//! （通知是否目视可达需人工确认）；1 = 链路任一步失败。
//! 副作用与产品行为一致：首次运行会在用户开始菜单创建 LiveTranslate 快捷方式。

fn main() {
    lt_i18n::set_lang("zh").expect("zh 表解析");
    let (title, body) = lt_ui::notifications::hidden_hint_texts();
    match lt_ui::notifications::show(&title, &body) {
        Ok(()) => {
            println!("通知已发送：{title} / {body}");
            println!("请查看 Windows 通知（右下角 toast / 操作中心）。如需清理测试产生的约束，删除开始菜单 LiveTranslate.lnk 与配置目录 app.ico 即可。");
        }
        Err(e) => {
            eprintln!("通知链路失败: {e}");
            std::process::exit(1);
        }
    }
}
