use dioxus::prelude::*;

use crate::ui::state::AppContext;

#[component]
pub(crate) fn ModalDialog(
    title: String,
    class: String,
    #[props(default = "dialog".to_owned())] role: String,
    on_cancel: EventHandler<()>,
    children: Element,
) -> Element {
    let mut context = use_context::<AppContext>();
    let scope = dioxus::core::current_scope_id().0;
    let dialog_id = format!("wallet-modal-{scope}");
    let title_id = format!("{dialog_id}-title");

    rsx! {
        dialog {
            id: "{dialog_id}",
            class: "modal-dialog {class}",
            role,
            aria_labelledby: "{title_id}",
            onmounted: move |_| async move {
                let script = format!(
                    r#"
                    const dialog = document.getElementById('wallet-modal-{scope}');
                    if ('closedBy' in dialog) {{
                        dialog.closedBy = 'any';
                    }} else {{
                        const onBackdrop = event => {{
                            const box = dialog.getBoundingClientRect();
                            return event.target === dialog && (
                                event.clientX < box.left || event.clientX >= box.right ||
                                event.clientY < box.top || event.clientY >= box.bottom
                            );
                        }};
                        let startedOutside = false;
                        dialog.addEventListener('pointerdown', event => {{
                            startedOutside = onBackdrop(event);
                        }});
                        dialog.addEventListener('click', event => {{
                            if (startedOutside && onBackdrop(event)) {{
                                dialog.dispatchEvent(new Event('cancel'));
                            }}
                            startedOutside = false;
                        }});
                    }}
                    dialog.showModal();
                    "#,
                );
                if let Err(error) = document::eval(&script).await {
                    context.push_header_message(format!("Could not open dialog: {error}"));
                    on_cancel.call(());
                }
            },
            oncancel: move |_| on_cancel.call(()),
            h2 { id: title_id, "{title}" }
            {children}
        }
    }
}
