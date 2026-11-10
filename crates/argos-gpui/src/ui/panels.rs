//! Dock panels. The map fills the center and the Rover panel (the basics)
//! sits in the right dock. Everything else is an optional panel the operator
//! opens from the console bar's Panels menu, then drags, tabs, or closes.
//!
//! A panel holds no state of its own: it asks the `Dashboard` to draw its
//! body, so buttons inside still act on the dashboard.

use crate::Dashboard;

use gpui_kit::component::dock::{
    BasePanel, DockArea, DockLayout, DockPlacement, DockSkin, Panel, PanelEvent, PanelId,
    panel_handle,
};
use gpui_kit::{prelude::*, *};

/// Every panel the window can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelKind {
    /// The 3D map with rover cards. Always open.
    Map,
    /// The selected rover's basics: Stop, state, Take over, level, goal.
    Rover,
    /// Trace, hardware, and navigation detail.
    Details,
    /// Study condition, L4 explore budget, and the L5 task box.
    Autonomy,
    /// Goal entry by coordinates and 3D capture.
    Goal,
    /// Planned missions.
    Mission,
    /// FPV video.
    Video,
    /// Agent bridge and recommendation cards.
    Agent,
    /// Per-rover neglect, waiting and interaction over time.
    Timeline,
}

impl PanelKind {
    /// Optional panels, in the order the Panels menu lists them.
    pub(crate) const OPTIONAL: [PanelKind; 7] = [
        PanelKind::Details,
        PanelKind::Autonomy,
        PanelKind::Goal,
        PanelKind::Mission,
        PanelKind::Video,
        PanelKind::Agent,
        PanelKind::Timeline,
    ];

    /// Stable name for saved layouts. Never change one.
    fn name(self) -> &'static str {
        match self {
            PanelKind::Map => "argos.map",
            PanelKind::Rover => "argos.rover",
            PanelKind::Details => "argos.details",
            PanelKind::Autonomy => "argos.autonomy",
            PanelKind::Goal => "argos.goal",
            PanelKind::Mission => "argos.mission",
            PanelKind::Video => "argos.video",
            PanelKind::Agent => "argos.agent",
            PanelKind::Timeline => "argos.timeline",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            PanelKind::Map => "Map",
            PanelKind::Rover => "Rover",
            PanelKind::Details => "Details",
            PanelKind::Autonomy => "Study & tasks",
            PanelKind::Goal => "Goal & capture",
            PanelKind::Mission => "Mission",
            PanelKind::Video => "Video",
            PanelKind::Agent => "Agent",
            PanelKind::Timeline => "Timeline",
        }
    }

    /// Where a panel opens from the Panels menu.
    fn home(self) -> DockPlacement {
        match self {
            PanelKind::Map => DockPlacement::Center,
            PanelKind::Agent => DockPlacement::Left,
            PanelKind::Video | PanelKind::Timeline => DockPlacement::Bottom,
            _ => DockPlacement::Right,
        }
    }
}

pub(crate) struct ArgosPanel {
    kind: PanelKind,
    dashboard: WeakEntity<Dashboard>,
    focus: FocusHandle,
}

impl EventEmitter<PanelEvent> for ArgosPanel {}

impl Focusable for ArgosPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BasePanel for ArgosPanel {
    fn panel_name(&self) -> &'static str {
        self.kind.name()
    }
    /// Map and Rover always stay: the Stop button lives in Rover.
    fn closable(&self, _: &App) -> bool {
        !matches!(self.kind, PanelKind::Map | PanelKind::Rover)
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ArgosPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.kind.title().into())
    }
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.kind.title()
    }
    fn zoom_control(&self, _: &App) -> Option<gpui_kit::component::dock::PanelControl> {
        None
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn title_bar(&self, _: &App) -> bool {
        // Map and Rover draw their own headers.
        !matches!(self.kind, PanelKind::Map | PanelKind::Rover)
    }
}

impl Render for ArgosPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        self.dashboard
            .update(cx, |dashboard, cx| dashboard.panel_body(kind, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// The dock area and one panel entity per kind, made once and reused when a
/// closed panel is opened again.
pub(crate) struct Docks {
    pub(crate) area: Entity<DockArea>,
    panels: Vec<(PanelKind, Entity<ArgosPanel>)>,
}

impl Docks {
    /// Map in the center, Rover in the right dock; optional panels closed.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let dashboard = cx.weak_entity();
        let (area, skin) = DockSkin::dock_area("argos", Some(1), window, cx);
        skin.set_close_button_visible(true, cx);
        let panels: Vec<_> = std::iter::once(PanelKind::Map)
            .chain(std::iter::once(PanelKind::Rover))
            .chain(PanelKind::OPTIONAL)
            .map(|kind| {
                let dashboard = dashboard.clone();
                (
                    kind,
                    cx.new(|cx| ArgosPanel {
                        kind,
                        dashboard,
                        focus: cx.focus_handle(),
                    }),
                )
            })
            .collect();
        let docks = Self { area, panels };
        let map = docks.entity(PanelKind::Map);
        let rover = docks.entity(PanelKind::Rover);
        docks.area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::tabs().panel_view(panel_handle(map), cx),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Right,
                DockLayout::tabs().panel_view(panel_handle(rover), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Right, px(380.), window, cx);
        });
        docks
    }

    fn entity(&self, kind: PanelKind) -> Entity<ArgosPanel> {
        self.panels
            .iter()
            .find(|(each, _)| *each == kind)
            .map(|(_, entity)| entity.clone())
            .expect("every panel kind is built in Docks::new")
    }

    /// Redraw every panel. The dock caches each panel view, and a panel body
    /// reads Dashboard state, so `cx.notify()` on the Dashboard alone does not
    /// repaint it. A cached Video panel kept painting a frame whose atlas
    /// texture was already dropped, so the picture blanked between redraws.
    pub(crate) fn notify_all(&self, cx: &mut App) {
        for (_, entity) in &self.panels {
            entity.update(cx, |_, cx| cx.notify());
        }
    }

    pub(crate) fn is_open(&self, kind: PanelKind, cx: &App) -> bool {
        let id = PanelId::from(self.entity(kind).entity_id());
        self.area.read(cx).panel(id).is_some()
    }

    /// Open a closed panel in its home dock, or close an open one.
    pub(crate) fn toggle(&self, kind: PanelKind, window: &mut Window, cx: &mut App) {
        let entity = self.entity(kind);
        let open = self.is_open(kind, cx);
        self.area.update(cx, |area, cx| {
            if open {
                area.remove_panel(entity, window, cx);
            } else {
                let placement = kind.home();
                let size = match placement {
                    DockPlacement::Bottom => Some(px(280.)),
                    _ => Some(px(360.)),
                };
                area.add_panel_view(panel_handle(entity), placement, size, window, cx);
                if !area.is_dock_open(placement) {
                    area.toggle_dock(placement, window, cx);
                }
            }
        });
    }
}
