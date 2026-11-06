mod api;
mod config;
mod keys;
mod local_wallet;
mod p2qrh;
mod transaction_plan;
mod transaction_signing;
mod ui;
mod unsigned_transaction;
mod wallet_file;
mod wallet_secrets;

fn main() {
    let window = dioxus::desktop::WindowBuilder::new()
        .with_inner_size(dioxus::desktop::LogicalSize::new(1080.0, 640.0))
        .with_min_inner_size(dioxus::desktop::LogicalSize::new(1080.0, 640.0))
        .with_always_on_top(false);

    #[cfg(target_os = "macos")]
    let window = {
        use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;

        window
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
    };

    let desktop = dioxus::desktop::Config::new().with_window(window);

    dioxus::LaunchBuilder::desktop()
        .with_cfg(desktop)
        .launch(ui::App);
}
