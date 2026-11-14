use dioxus::desktop::muda::{AboutMetadata, Menu, PredefinedMenuItem, Submenu};

fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let menu = create_menu_bar()?;
    let desktop = dioxus::desktop::Config::new()
        .with_window(window)
        .with_menu(menu);

    dioxus::LaunchBuilder::desktop()
        .with_cfg(desktop)
        .launch(wallet::App);

    Ok(())
}

fn create_menu_bar() -> Result<Menu, Box<dyn std::error::Error>> {
    let icon = dioxus::desktop::icon_from_memory(include_bytes!("../icons/icon.png"))?;
    let about = PredefinedMenuItem::about(
        Some("About Northbound"),
        Some(AboutMetadata {
            name: Some("Northbound".to_string()),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            icon: Some(icon),
            ..Default::default()
        }),
    );
    let application_menu = Submenu::with_items(
        "Northbound",
        true,
        &[
            &about,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )?;
    let edit_menu = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::select_all(None),
        ],
    )?;
    let window_menu = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::close_window(None),
        ],
    )?;
    let menu = Menu::with_items(&[&application_menu, &edit_menu, &window_menu])?;

    #[cfg(debug_assertions)]
    {
        use dioxus::desktop::muda::MenuItem;

        let help_menu = Submenu::with_items(
            "Help",
            true,
            &[
                &MenuItem::with_id(
                    "dioxus-toggle-dev-tools",
                    "Toggle Developer Tools",
                    true,
                    None,
                ),
                &MenuItem::with_id(
                    "dioxus-float-top",
                    "Float on Top (dev mode only)",
                    true,
                    None,
                ),
            ],
        )?;
        menu.append(&help_menu)?;

        #[cfg(target_os = "macos")]
        help_menu.set_as_help_menu_for_nsapp();
    }

    #[cfg(target_os = "macos")]
    window_menu.set_as_windows_menu_for_nsapp();

    Ok(menu)
}
