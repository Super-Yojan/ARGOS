use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use rig::Agent;
use rig::completion::Message;

use crate::llm::{self, LlmConfig};

#[derive(Clone, Copy, PartialEq)]
enum Sender {
    User,
    Agent,
    Error,
}

struct ChatMessage {
    sender: Sender,
    text: SharedString,
}

pub struct AgentChat {
    input: Entity<InputState>,
    /// The scroller only tracks row count and scroll position;
    /// the messages themselves live here.
    scroller: Entity<MessageScrollerState>,
    messages: Vec<ChatMessage>,
    // Might need to make agents a vector to support different types of agent
    // and different system prompts.
    agent: Agent,
    /// The conversation as rig sees it, sent with every request.
    history: Vec<Message>,
    base_url: String,
    /// The in-flight request. Dropping it cancels the UI update.
    pending: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AgentChat {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Message the agent…"));
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));

        let config = LlmConfig::from_env();
        let agent = llm::build_agent(&config);

        let _subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, state, event, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        if this.pending.is_some() {
                            return; // Wait for the current reply.
                        }
                        let text = state.read(cx).value().trim().to_string();
                        if text.is_empty() {
                            return;
                        }
                        state.update(cx, |state, cx| state.set_value("", window, cx));
                        this.send(text, cx);
                    }
                }),
            ];

        Self {
            input,
            scroller,
            messages: Vec::new(),
            agent,
            history: Vec::new(),
            base_url: config.base_url,
            pending: None,
            _subscriptions,
        }
    }

    /// Show the user's message, then ask the model in the background.
    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        self.push_message(Sender::User, text.clone().into(), cx);

        // The request runs on the tokio runtime (reqwest needs it);
        // the GPUI task waits for it and updates the view.
        let agent = self.agent.clone();
        let mut history = self.history.clone();
        let request = llm::runtime().spawn(async move {
            let reply = agent
                .chat(text, &mut history)
                .await
                .map(|response| response.output());
            (reply, history)
        });

        self.pending = Some(cx.spawn(async move |this, cx| {
            let outcome = request.await;
            let _ = this.update(cx, |this, cx| {
                this.pending = None;
                match outcome {
                    Ok((Ok(reply), history)) => {
                        this.history = history;
                        this.push_message(Sender::Agent, reply.trim().to_string().into(), cx);
                    }
                    Ok((Err(err), _)) => {
                        let text = format!("Couldn't get a reply from {}: {err}", this.base_url);
                        this.push_message(Sender::Error, text.into(), cx);
                    }
                    Err(err) => {
                        let text = format!("The request task failed: {err}");
                        this.push_message(Sender::Error, text.into(), cx);
                    }
                }
            });
        }));
        cx.notify();
    }

    /// Add a message and tell the scroller there is one more row.
    fn push_message(&mut self, sender: Sender, text: SharedString, cx: &mut Context<Self>) {
        self.messages.push(ChatMessage { sender, text });
        self.scroller.update(cx, |scroller, cx| {
            scroller.append(1, cx);
        });
        cx.notify();
    }
}

impl Render for AgentChat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The row renderer must be 'static, so it can't borrow `self`.
        // It holds a weak handle and reads the message by index when a row is drawn.
        let this = cx.entity().downgrade();

        let messages = MessageScroller::new(
            "agent-chat-messages",
            self.scroller.clone(),
            move |ix, _window, cx| {
                let Some(this) = this.upgrade() else {
                    return div().into_any_element();
                };
                let Some(message) = this.read(cx).messages.get(ix) else {
                    return div().into_any_element();
                };
                let sender = message.sender;
                let is_user = sender == Sender::User;
                let text = message.text.clone();
                let theme = cx.theme();

                h_flex()
                    .w_full()
                    .px_4()
                    .py_1()
                    .when(is_user, |row| row.justify_end())
                    .child(
                        div()
                            .max_w(relative(0.75))
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .when(is_user, |bubble| {
                                bubble
                                    .bg(theme.primary)
                                    .text_color(theme.primary_foreground)
                            })
                            .when(sender == Sender::Agent, |bubble| {
                                bubble.bg(theme.muted).text_color(theme.foreground)
                            })
                            .when(sender == Sender::Error, |bubble| {
                                bubble
                                    .border_1()
                                    .border_color(theme.danger)
                                    .text_color(theme.danger)
                            })
                            .child(text),
                    )
                    .into_any_element()
            },
        )
        .flex_1()
        .with_bottom_fade(cx.theme().background);

        v_flex().size_full().child(messages).child(
            h_flex()
                .p_4()
                .gap_2()
                .items_center()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(Input::new(&self.input))
                .when(self.pending.is_some(), |row| {
                    row.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Thinking…"),
                    )
                }),
        )
    }
}
