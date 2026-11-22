use gpui_kit::component::sidebar::{
    Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem, SidebarToggleButton,
};
use gpui_kit::component::{ActiveTheme, IconName, h_flex, v_flex};
use gpui_kit::*;

mod components;
mod llm;
use components::chat::AgentChat;
use components::map::RoverMap;
use components::teleop::Teleop;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Map,
    Chat,
    Teleop,
}

struct HelloWorld {
    map: Entity<RoverMap>,
    chat: Entity<AgentChat>,
    teleop: Entity<Teleop>,
    page: Page,
    collapsed: bool,
}

impl HelloWorld {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let map = cx.new(|cx| RoverMap::new(cx));
        let chat = cx.new(|cx| AgentChat::new(window, cx));
        let teleop = cx.new(|cx| Teleop::new(cx));
        Self {
            map,
            chat,
            teleop,
            page: Page::Map,
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
                            SidebarMenuItem::new("Map")
                                .icon(IconName::LayoutDashboard)
                                .active(self.page == Page::Map)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.page = Page::Map;
                                    cx.notify();
                                })),
                        )
                        .child(
                            SidebarMenuItem::new("Chat")
                                .icon(IconName::Settings)
                                .active(self.page == Page::Chat)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.page = Page::Chat;
                                    cx.notify();
                                })),
                        )
                        .child(
                            SidebarMenuItem::new("Teleop")
                                .icon(IconName::Bot)
                                .active(self.page == Page::Teleop)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.page = Page::Teleop;
                                    cx.notify();
                                })),
                        ),
                ),
            );

        let active_view = match self.page {
            Page::Map => div().flex_1().min_h_0().child(self.map.clone()),
            Page::Chat => div().flex_1().min_h_0().child(self.chat.clone()),
            Page::Teleop => div().flex_1().min_h_0().child(self.teleop.clone()),
        };

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
            .child(active_view);

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
