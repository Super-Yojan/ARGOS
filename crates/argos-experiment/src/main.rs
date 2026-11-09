use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem, SidebarToggleButton,
};
use gpui_kit::component::{ActiveTheme, IconName, h_flex, v_flex};
use gpui_kit::*;

struct MessageStore {
    count: usize,
}

struct HelloWorld {
    store: Entity<MessageStore>,
    collapsed: bool,
    _subscription: Subscription,
}

impl HelloWorld {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = cx.new(|_| MessageStore { count: 0 });
        let _subscription = cx.observe(&store, |_, _, cx| cx.notify());

        Self {
            store,
            collapsed: false,
            _subscription,
        }
    }
}

impl Render for HelloWorld {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.store.read(cx).count;

        let sidebar = Sidebar::new("sidebar")
            .collapsed(self.collapsed)
            .header(SidebarHeader::new().child("ARGOS"))
            .child(
                SidebarGroup::new("Menu").child(
                    SidebarMenu::new()
                        .child(
                            SidebarMenuItem::new("Home")
                                .icon(IconName::LayoutDashboard)
                                .active(true),
                        )
                        .child(SidebarMenuItem::new("Settings").icon(IconName::Settings)),
                ),
            );

        let content = v_flex()
            .flex_1()
            .h_full()
            .child(
                // Top bar: toggle button that collapses/expands the sidebar.
                h_flex()
                    .p_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        SidebarToggleButton::new()
                            .collapsed(self.collapsed)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.collapsed = !this.collapsed;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .p_4()
                    .gap_2()
                    .items_center()
                    .child(format!("Message: {count}"))
                    .child(
                        Button::new("add")
                            .primary()
                            .label("Add")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.count += 1;
                                    cx.notify();
                                })
                            })),
                    ),
            );

        // Sidebar on the left, content filling the rest of the window.
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(sidebar)
            .child(content)
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);

        open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(HelloWorld::new)
        })
        .expect("Failed to open window");
    });
}
