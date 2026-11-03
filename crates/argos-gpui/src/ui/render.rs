//! The window layout: console bar, map with its mouse and key handlers,
//! rover cards, inspector, and popups. Pieces are built in the sibling files.

use super::agent;
use super::cards::CARD_HEIGHT_REM;
use super::cards::CARD_WIDTH_REM;
use super::cards::card_group;
use super::cards::offscreen_tabs;
use super::console_bar::console_bar;
use super::drive::drive_key;
use super::inspector::Facts;
use super::inspector::rover_inspector;
use super::interaction::goal_geo;
use super::map_overlay::map_paint;
use super::popups::dump_popups;
use super::popups::goal_popup;
use crate::Dashboard;

use argos_core::{DragAnchor, DragSpace};
use argos_gpui::console;
use gpui_kit::base::v_flex;
use gpui_kit::component::{ActiveTheme, ThemeStyled};
use gpui_kit::{prelude::*, *};

impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.driving && !window.is_window_active() {
            self.stop_drive();
            self.note = "Drive stopped because the window lost focus".into();
        }
        if self.drag.is_some() && !window.is_window_active() {
            self.cancel_drag();
        }
        let selected = self.selected;
        let control_held = self.claim.held(self.now);
        let driving = self.driving;
        let limits = self.feed.limits();
        let mark = self.selected_mark().cloned();
        let drag_local = self.drag.as_ref().and_then(|active| {
            let goal = &active.goal;
            let local = match goal.space {
                DragSpace::Local => Some((goal.cursor.x, goal.cursor.y)),
                DragSpace::Geographic => self
                    .selected_mark()
                    .and_then(|mark| mark.overlay_frame)
                    .and_then(|frame| frame.local(goal.cursor.x, goal.cursor.y).ok()),
            }?;
            Some((
                goal.id,
                local.0,
                local.1,
                goal.distance_m(),
                goal.arrival_radius_m,
            ))
        });
        let status = console::status_line(&self.note, &self.command).to_owned();
        let unlocated = console::unlocated(&self.marks, &self.presence);
        let mut notices = console::reporting_notices(&self.marks);
        notices.extend(console::field_notices(
            &self.link,
            &self.detail,
            self.maps_notice.as_deref(),
        ));
        let reversing_key = self.driving && self.held.contains("s");
        let handles: Vec<(u64, DragAnchor, Point<Pixels>)> = {
            let map = self.view.read(cx);
            let mut handles = Vec::new();
            for mark in &self.marks {
                if let Some(at) = map.project_local(mark.latitude, mark.longitude) {
                    handles.push((mark.id, DragAnchor::Rover, at));
                }
                if let Some(goal) = goal_geo(mark)
                    && let Some(at) = map.project_local(goal.x, goal.y)
                {
                    handles.push((mark.id, DragAnchor::Goal, at));
                }
            }
            handles
        };
        // robot.glb bounds after the loader's Y-up to Z-up conversion.
        let model_handles: Vec<_> = self
            .marks
            .iter()
            .filter_map(|mark| {
                let object = *self.objects.get(&mark.id)?;
                self.view
                    .read(cx)
                    .object_bounds_local(object, [-1.225, -1.25, 0.], [1.225, 1.25, 2.25])
                    .map(|bounds| (mark.id, bounds))
            })
            .collect();
        let overlay = map_paint(self);
        let confirm = self.drag.as_ref().and_then(|active| {
            let at = self.proposal_at(active, cx)?;
            Some(goal_popup(&active.goal, at, cx).into_any_element())
        });
        let field: Vec<(u64, Option<Point<Pixels>>, console::Callout)> = {
            let map = self.view.read(cx);
            self.marks
                .iter()
                .map(|mark| {
                    let driving_here = driving && selected == Some(mark.id);
                    (
                        mark.id,
                        map.project_local(mark.latitude, mark.longitude),
                        console::callout(mark, self.stop_cause(mark).as_ref(), driving_here),
                    )
                })
                .collect()
        };
        // Measured last frame; until then the field has no known edge.
        let rem = f32::from(window.rem_size());
        if self.selected_mark().is_none() {
            *self.inspector_frame.lock().unwrap() = None;
        }
        let field_bounds = *self.field_frame.lock().unwrap();
        let gap = 0.5 * rem;
        let groups = console::place_cards(
            &field
                .iter()
                .filter_map(|(id, at, _)| at.map(|at| (*id, f32::from(at.x), f32::from(at.y))))
                .collect::<Vec<_>>(),
            console::CardSpace {
                // The card's 1rem offset from its anchor counts toward its size.
                card_width: (CARD_WIDTH_REM + 1.) * rem,
                card_height: CARD_HEIGHT_REM * rem,
                // Both frames are measured at the previous prepaint; until the
                // first one, the field has no known edge and nothing to avoid.
                field: console::FieldRect {
                    x: 0.,
                    y: 0.,
                    width: field_bounds.map_or(f32::INFINITY, |b| f32::from(b.size.width)),
                    height: field_bounds.map_or(f32::INFINITY, |b| f32::from(b.size.height)),
                },
                avoid: field_bounds.zip(*self.inspector_frame.lock().unwrap()).map(
                    |(field, inspector)| console::FieldRect {
                        x: f32::from(inspector.origin.x - field.origin.x) - gap,
                        y: f32::from(inspector.origin.y - field.origin.y) - gap,
                        width: f32::from(inspector.size.width) + 2. * gap,
                        height: f32::from(inspector.size.height) + 2. * gap,
                    },
                ),
            },
        );
        let inspector = mark.map(|mark| {
            rover_inspector(
                self,
                &mark,
                Facts {
                    drag: drag_local,
                    limits,
                    reversing_key,
                },
                cx,
            )
            .into_any_element()
        });
        div()
            .id("argos")
            .track_focus(&self.focus)
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, window, cx| {
                if this.typing(window, cx) {
                    return;
                }
                if let Some(key) = drive_key(&event.keystroke.key) {
                    this.hold(key, false);
                    cx.stop_propagation();
                }
            }))
            // Capture phase: Space stops even when a focused button would
            // otherwise take it as activation.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                // Space in a task text box is a space; Stop all stays on the console bar.
                if event.keystroke.key == "space" && !this.typing(window, cx) {
                    this.estop();
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel_drag();
                    cx.stop_propagation();
                    return;
                }
                if this.typing(window, cx) {
                    return;
                }
                if let Some(key) = drive_key(&event.keystroke.key) {
                    this.hold(key, true);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(".SystemUIFont")
            .child(console_bar(self, status, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    // Bubbles up from the map; cards, tabs, the inspector,
                    // and the popup stop it first.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.press = Some(event.position);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _, cx| {
                            let Some(press) = this.press.take() else {
                                return;
                            };
                            let moved = f32::from(event.position.x - press.x)
                                .hypot(f32::from(event.position.y - press.y));
                            if moved <= CLICK_SLOP_PX {
                                this.click_map(event.position, cx);
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.press = None),
                    )
                    .child(self.view.clone())
                    .child(overlay)
                    .children(model_handles.into_iter().map(|(id, bounds)| {
                        div()
                            .id(("model-drag", id))
                            .absolute()
                            .left(bounds.origin.x)
                            .top(bounds.origin.y)
                            .w(bounds.size.width)
                            .h(bounds.size.height)
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.focus.focus(window, cx);
                                    this.select(id);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                    }))
                    .children(handles.into_iter().map(|(id, anchor, at)| {
                        let slot = match anchor {
                            DragAnchor::Rover => 2,
                            DragAnchor::Goal => 3,
                        };
                        div()
                            .id(("map-drag", id.saturating_mul(4).saturating_add(slot)))
                            .absolute()
                            .left(at.x - px(9.))
                            .top(at.y - px(9.))
                            .w(px(18.))
                            .h(px(18.))
                            .rounded_md()
                            .cursor_pointer()
                            .bg(
                                if anchor == DragAnchor::Goal
                                    || (selected == Some(id) && control_held)
                                {
                                    cx.theme().success
                                } else {
                                    cx.theme().warning
                                },
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.focus.focus(window, cx);
                                    this.select(id);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                    }))
                    .child({
                        let frame = self.field_frame.clone();
                        canvas(
                            move |bounds, _, _| *frame.lock().unwrap() = Some(bounds),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                    })
                    .children(
                        groups.into_iter().map(|group| {
                            card_group(group, &field, selected, cx).into_any_element()
                        }),
                    )
                    .when(!notices.is_empty(), |field| {
                        field.child(
                            v_flex()
                                .absolute()
                                .top_3()
                                .left_3()
                                .max_w(px(420.))
                                .px_2()
                                .py_1()
                                .gap_1()
                                .popover_style(cx)
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .children(notices),
                        )
                    })
                    .child(offscreen_tabs(&field, &unlocated, selected, cx))
                    .child(agent::agent_panel(self, cx))
                    .children(confirm)
                    .children(inspector)
                    .child(dump_popups(&self.marks, cx)),
            )
    }
}

/// How far the pointer may move between press and release and still count
/// as a click rather than a camera orbit or pan.
const CLICK_SLOP_PX: f32 = 4.;
