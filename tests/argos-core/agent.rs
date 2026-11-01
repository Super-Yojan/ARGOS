use argos_core::*;
use serde_json::json;

fn line(value: serde_json::Value) -> AgentEnvelope {
    parse_agent_line(value.to_string().as_bytes())
}

fn rec(value: serde_json::Value) -> Recommendation {
    decode_recommendation(&value, "test").unwrap()
}

#[test]
fn parses_reads_and_commands() {
    let envelope = line(json!({"id": 7, "token": "t", "op": "state"}));
    assert_eq!(envelope.id, json!(7));
    assert_eq!(envelope.token, "t");
    assert_eq!(envelope.request.unwrap(), AgentRequest::State);

    let explore = line(json!({"op": "set_level", "rover": 2, "level": "explore", "budget_min": 5}));
    assert_eq!(
        explore.request.unwrap(),
        AgentRequest::Command(AgentCommand::SetLevel {
            rover: 2,
            level: "explore".into(),
            budget_min: Some(5)
        })
    );
    let gps = line(json!({"op": "go_to", "rover": 1, "latitude": 38.83, "longitude": -77.32}));
    assert!(matches!(
        gps.request.unwrap(),
        AgentRequest::Command(AgentCommand::GoTo {
            goal: AgentGoal::Geographic { .. },
            ..
        })
    ));
    let task = line(
        json!({"op": "start_task", "rover": 3, "instruction": "find the orange bucket", "budget_min": 5, "agent": "mcp"}),
    );
    assert!(matches!(
        task.request.unwrap(),
        AgentRequest::Command(AgentCommand::StartTask {
            agent: Some(TaskAgent::Mcp),
            budget_min: 5,
            ..
        })
    ));
}

#[test]
fn refuses_what_the_buttons_cannot_send() {
    let error = |value| line(value).request.unwrap_err();
    assert!(
        error(json!({"op": "set_level", "rover": 1, "level": "explore"})).contains("budget_min")
    );
    assert!(
        error(json!({"op": "set_level", "rover": 1, "level": "explore", "budget_min": 7}))
            .contains("[3, 5, 10]")
    );
    assert!(error(json!({"op": "set_level", "rover": 1, "level": "task"})).contains("start_task"));
    assert!(
        error(json!({"op": "set_level", "rover": 1, "level": "fly"})).contains("Unknown level")
    );
    assert!(
        error(json!({"op": "go_to", "rover": 1, "x": 1.0, "latitude": 2.0})).contains("either")
    );
    assert!(
        error(json!({"op": "drive", "rover": 1, "linear": 0.0, "angular": 0.0, "seconds": 1.0}))
            .contains("zero")
    );
    assert!(
        error(json!({"op": "drive", "rover": 1, "linear": 1.0, "angular": 0.0, "seconds": 5.0}))
            .contains("at most 2")
    );
    assert!(
        error(json!({"op": "request_drive", "rover": 1, "seconds": 120.0, "reason": "x"}))
            .contains("at most 60")
    );
    assert!(error(json!({"op": "dump", "rover": 1})).contains("Unknown op"));
    assert!(parse_agent_line(b"not json").request.is_err());
}

#[test]
fn modes_gate_requests() {
    let command = AgentRequest::Command(AgentCommand::StopAll);
    let hello = AgentRequest::Hello { engine: "x".into() };
    assert!(agent_permitted(AgentMode::Off, &hello).is_ok());
    assert!(agent_permitted(AgentMode::Off, &AgentRequest::State).is_err());
    assert!(agent_permitted(AgentMode::Advise, &AgentRequest::State).is_ok());
    assert!(agent_permitted(AgentMode::Advise, &AgentRequest::Propose(vec![])).is_ok());
    assert!(
        agent_permitted(AgentMode::Advise, &command)
            .unwrap_err()
            .contains("Advise")
    );
    assert!(agent_permitted(AgentMode::Operator, &command).is_ok());
    assert_eq!(AgentMode::default(), AgentMode::Advise);
}

#[test]
fn tokens_and_sources() {
    assert!(token_matches("abc", "abc"));
    assert!(!token_matches("abd", "abc"));
    assert!(!token_matches("", ""));
    let agent = command_token(CommandSource::Agent, 4, 123);
    assert_eq!(agent, "gpui-agent-4-123");
    assert!(valid_token(&agent));
    assert_eq!(token_source(&agent), "agent");
    assert_eq!(
        token_source(&command_token(CommandSource::Recommendation, 4, 1)),
        "recommendation"
    );
    assert_eq!(
        token_source(&command_token(CommandSource::Operator, 4, 1)),
        "operator"
    );
    // The window's receipt check still sees agent tokens as its own.
    let receipt = vehicle_request_receipt(
        &json!({"token": agent, "result": "accepted", "run_id": "r", "revision": 1}),
        0,
    );
    assert!(receipt.is_some());
}

#[test]
fn drive_keys_follow_direction_only() {
    assert_eq!(drive_keys(0.5, 0.0).unwrap(), vec!["w"]);
    assert_eq!(drive_keys(-0.5, -0.4).unwrap(), vec!["s", "d"]);
    assert_eq!(drive_keys(0.05, 0.9).unwrap(), vec!["a"]);
    assert!(drive_keys(2.0, 0.0).is_err());
}

#[test]
fn drive_grant_waits_for_the_operator() {
    let mut grant = DriveGrant::request(2, 20., "nudge into the bucket".into(), 10.);
    assert!(grant.pending());
    assert!(!grant.active(11.));
    assert!(!grant.lapsed(69.));
    assert!(grant.lapsed(70.));
    grant.approve(12.);
    assert!(grant.active(20.));
    assert!((grant.remaining(22.) - 10.).abs() < 1e-9);
    assert!(grant.lapsed(32.));
    assert_eq!(grant.to_json(13.)["state"], "granted");
}

#[test]
fn approach_stops_short_of_the_bucket() {
    let (x, y) = approach_point((0., 0.), (10., 0.), 1.5).unwrap();
    assert!((x - 8.5).abs() < 1e-9 && y.abs() < 1e-9);
    assert!(approach_point((0., 0.), (1.0, 0.), 1.5).is_err());
}

#[test]
fn decodes_recommendations() {
    let r = rec(
        json!({"rover": 1, "kind": "handoff_bucket", "action": {"object_id": 7},
        "urgency": 3, "reason": "5 balls on board, bucket 6 m away", "evidence": ["objects"], "confidence": 0.7}),
    );
    assert_eq!(
        r.action,
        RecAction::HandoffBucket {
            object_id: 7,
            approach_m: 1.5
        }
    );
    assert_eq!(r.ttl_s, DEFAULT_RECOMMENDATION_TTL_S);
    let error = |value| decode_recommendation(&value, "t").unwrap_err();
    assert!(error(json!({"rover": 1, "kind": "dump", "reason": "x"})).contains("Unknown kind"));
    assert!(error(json!({"rover": 1, "kind": "attention"})).contains("reason"));
    assert!(
        error(json!({"rover": 1, "kind": "attention", "reason": "x", "urgency": 5}))
            .contains("urgency")
    );
    assert!(
        error(json!({"rover": 1, "kind": "set_level", "action": {"level": "task"}, "reason": "x"}))
            .contains("kind `task`")
    );
    let long = rec(json!({"rover": 1, "kind": "attention", "reason": "x", "expires_s": 9999}));
    assert_eq!(long.ttl_s, MAX_RECOMMENDATION_TTL_S);
}

#[test]
fn book_caps_dedupes_and_honours_rejections() {
    let mut book = RecBook::default();
    let explore = |rover, urgency| {
        rec(
            json!({"rover": rover, "kind": "set_level", "action": {"level": "explore", "budget_min": 5},
            "urgency": urgency, "reason": "idle"}),
        )
    };
    let (a, _) = book.propose(explore(1, 1), 0.).unwrap();
    // The same idea refreshes the open card.
    let (again, _) = book.propose(explore(1, 2), 5.).unwrap();
    assert_eq!(a, again);
    assert_eq!(book.open().len(), 1);
    assert_eq!(book.open()[0].rec.urgency, 2);
    let (b, _) = book.propose(explore(2, 1), 6.).unwrap();
    let (c, _) = book.propose(explore(3, 1), 7.).unwrap();
    // Full, and the newcomer is no more urgent than the weakest.
    assert!(book.propose(explore(4, 1), 8.).is_err());
    // A more urgent one replaces the oldest least urgent card.
    let (d, superseded) = book.propose(explore(4, 3), 9.).unwrap();
    assert_eq!(superseded.as_deref(), Some(b.as_str()));
    assert_eq!(book.open()[0].rec.rec_id, d);
    assert_eq!(book.get(&b).unwrap().state, RecState::Superseded);

    book.decide(&c, RecState::Rejected, Some("bad_timing".into()), 10.)
        .unwrap();
    assert!(book.decide(&c, RecState::Accepted, None, 11.).is_err());
    let cooled = book.propose(explore(3, 3), 20.).unwrap_err();
    assert!(cooled.contains("rejected"));
    assert!(
        book.propose(explore(3, 3), 10. + REJECT_COOLDOWN_S + 1.)
            .is_ok()
    );

    let recent = book.recent(5);
    assert_eq!(recent[0].state, RecState::Rejected);
    let json = book.get(&c).unwrap().to_json(12.);
    assert_eq!(json["decision_reason"], "bad_timing");
    assert_eq!(json["decision_latency_s"], 3.0);
}

#[test]
fn book_expires_and_supersedes() {
    let mut book = RecBook::default();
    let (id, _) = book
        .propose(rec(json!({"rover": 1, "kind": "set_level", "action": {"level": "waypoint"}, "reason": "goal ready", "expires_s": 20})), 0.)
        .unwrap();
    assert!(book.expire(19.).is_empty());
    assert_eq!(book.expire(20.), vec![id.clone()]);
    assert_eq!(book.get(&id).unwrap().state, RecState::Expired);

    let (id, _) = book
        .propose(rec(json!({"rover": 1, "kind": "set_level", "action": {"level": "waypoint"}, "reason": "goal ready"})), 30.)
        .unwrap();
    let closed = book.supersede_where(31., |r| recommendation_stale(r, true, Some("waypoint")));
    assert_eq!(closed, vec![id]);
    assert!(recommendation_stale(&book.get(&closed[0]).unwrap().rec, false, None).is_some());
}

#[test]
fn status_tells_the_operator_when_no_agent_connects() {
    let note = "Agent bridge on 127.0.0.1:7471";
    let link = AgentLink {
        bridge: Some(note),
        since_open: 5.,
        ..AgentLink::default()
    };
    let waiting = agent_status(&link);
    assert!(!waiting.warn);
    assert!(waiting.text.contains("waiting for an agent"));

    let missing = agent_status(&AgentLink {
        since_open: AGENT_CONNECT_GRACE_S,
        ..link.clone()
    });
    assert!(missing.warn, "bridge up, no agent after the grace period");
    assert!(missing.text.contains("Agent not connected"));
    assert!(missing.text.contains("recommend_loop.py"));

    let launched = agent_status(&AgentLink {
        since_open: 60.,
        command: Some("python3 recommend_loop.py"),
        ..link.clone()
    });
    assert!(launched.warn);
    assert!(launched.text.contains("python3 recommend_loop.py"));

    let stopped = agent_status(&AgentLink {
        command: Some("python3 recommend_loop.py"),
        command_exit: Some("exit status: 1 · ModuleNotFoundError: anthropic"),
        ..link.clone()
    });
    assert!(stopped.warn);
    assert!(stopped.text.contains("ModuleNotFoundError"));

    let stale = agent_status(&AgentLink {
        since_open: 60.,
        bad_tokens: 2,
        ..link.clone()
    });
    assert!(stale.warn);
    assert!(stale.text.contains("stale token"));
}

#[test]
fn status_shows_a_connected_agent_and_flags_a_silent_one() {
    let link = AgentLink {
        bridge: Some("Agent bridge on 127.0.0.1:7471"),
        since_open: 600.,
        engine: Some("loop:claude"),
        since_call: Some(4.),
        ..AgentLink::default()
    };
    let live = agent_status(&link);
    assert!(!live.warn);
    assert_eq!(live.text, "loop:claude · last call 4 s ago");

    let silent = agent_status(&AgentLink {
        since_call: Some(AGENT_SILENT_S + 1.),
        ..link
    });
    assert!(silent.warn);
    assert!(silent.text.contains("silent"));

    let off = agent_status(&AgentLink {
        bridge_error: "Agent bridge off (ARGOS_AGENT=0)",
        ..AgentLink::default()
    });
    assert_eq!(off.text, "Agent bridge off (ARGOS_AGENT=0)");
}
