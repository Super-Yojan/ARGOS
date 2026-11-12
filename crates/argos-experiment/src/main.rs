use gpui_kit::component::sidebar::{
    Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem, SidebarToggleButton,
};
use gpui_kit::component::{ActiveTheme, IconName, h_flex, v_flex};
use gpui_kit::*;

mod components;
mod llm;
use components::chat::AgentChat;

struct HelloWorld {
    chat: Entity<AgentChat>,
    collapsed: bool,
}

impl HelloWorld {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let chat = cx.new(|cx| AgentChat::new(window, cx));
        Self {
            chat,
            collapsed: false,
        }
    }
}

impl Render for HelloWorld {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(div().flex_1().min_h_0().child(self.chat.clone()));

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

        open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| HelloWorld::new(window, cx))
        })
        .expect("Failed to open window");
    });
}
