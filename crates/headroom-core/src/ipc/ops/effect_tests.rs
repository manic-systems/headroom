use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use rtrb::Consumer;
use serde_json::{json, Value};

use super::*;
use crate::profile_store::{ProfileStore, StorePaths};
use crate::pw::command::PwCommand;
use crate::pw::filter::{AudioCmd, FilterControl};
use crate::state::{self, SharedState};
use headroom_ipc::{Op, Request, ResponsePayload, Route, ServerFrame, Topic};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_paths() -> (StorePaths, TempDir) {
    let base = std::env::temp_dir().join(format!(
        "headroom-ipc-effects-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(base.join("config/profiles")).unwrap();
    fs::create_dir_all(base.join("state")).unwrap();
    (
        StorePaths {
            config_dir: base.join("config"),
            state_dir: base.join("state"),
            share_dirs: vec![],
            extra_profile_dirs: vec![],
        },
        TempDir(base),
    )
}

fn shared_with_default_profile() -> SharedState {
    state::shared(crate::state::DaemonState::new(ProfileStore::builtin()))
}

fn shared_with_night_profile() -> (SharedState, TempDir) {
    let (paths, guard) = temp_paths();
    fs::write(
        paths.config_dir.join("profiles/night.toml"),
        r#"
name = "night"
description = "night profile"
[limiter]
ceiling_dbtp = -2.0
"#,
    )
    .unwrap();
    let store = ProfileStore::load(&paths).unwrap();
    (state::shared(crate::state::DaemonState::new(store)), guard)
}

fn ok(resp: Response) -> Value {
    match resp.payload {
        ResponsePayload::Ok { result } => result,
        ResponsePayload::Err { error } => panic!("expected ok, got {error}"),
    }
}

fn attach_filter(state: &SharedState) -> Consumer<AudioCmd> {
    let (control, consumer) = FilterControl::for_testing(16);
    state.lock().filter_control = Some(control);
    consumer
}

fn attach_pw_commands(state: &SharedState) -> Receiver<PwCommand> {
    let (tx, rx) = crossbeam_channel::unbounded();
    state.lock().pw_command_tx = Some(tx);
    rx
}

fn attach_events(state: &SharedState, topics: &[Topic]) -> Receiver<ServerFrame> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut state = state.lock();
    let id = state.broadcaster.register(tx);
    state.broadcaster.subscribe(id, topics);
    rx
}

fn drain_audio(consumer: &mut Consumer<AudioCmd>) -> Vec<AudioCmd> {
    let mut out = Vec::new();
    while let Ok(cmd) = consumer.pop() {
        out.push(cmd);
    }
    out
}

fn drain_pw(rx: &Receiver<PwCommand>) -> Vec<PwCommand> {
    rx.try_iter().collect()
}

fn drain_events(rx: &Receiver<ServerFrame>) -> Vec<headroom_ipc::Event> {
    rx.try_iter()
        .filter_map(|frame| match frame {
            ServerFrame::Event(event) => Some(event),
            ServerFrame::Response(_) => None,
        })
        .collect()
}

fn setting_set(key: &str, value: Value, state: &SharedState) {
    let _ = ok(dispatch(
        &Request::new(
            1,
            Op::SettingSet {
                key: key.into(),
                value,
            },
        ),
        state,
    ));
}

fn count_reevaluate_all(cmds: &[PwCommand]) -> usize {
    cmds.iter()
        .filter(|cmd| matches!(cmd, PwCommand::ReevaluateAll))
        .count()
}

fn count_reevaluate_layer_a(cmds: &[PwCommand]) -> usize {
    cmds.iter()
        .filter(|cmd| matches!(cmd, PwCommand::ReevaluateLayerA))
        .count()
}

fn assert_reevaluate_counts(cmds: &[PwCommand], route: usize, layer_a: usize) {
    assert_eq!(
        count_reevaluate_all(cmds),
        route,
        "expected {route} route reapply command(s), got {cmds:?}"
    );
    assert_eq!(
        count_reevaluate_layer_a(cmds),
        layer_a,
        "expected {layer_a} layer a reapply command(s), got {cmds:?}"
    );
}

#[test]
fn setting_set_agc_enabled_pushes_filter_enable() {
    let state = shared_with_default_profile();
    let mut audio = attach_filter(&state);

    setting_set("agc.enabled", json!(false), &state);

    let cmds = drain_audio(&mut audio);
    assert!(
        cmds.iter()
            .any(|cmd| matches!(cmd, AudioCmd::SetAgcEnabled(false))),
        "expected agc enable update, got {cmds:?}"
    );
}

#[test]
fn setting_set_compressor_enabled_pushes_compressor_config() {
    let state = shared_with_default_profile();
    let mut audio = attach_filter(&state);

    setting_set("compressor.enabled", json!(false), &state);

    let cmds = drain_audio(&mut audio);
    assert!(
        cmds.iter()
            .any(|cmd| { matches!(cmd, AudioCmd::SetCompressor(cfg) if !cfg.enabled) }),
        "expected compressor update, got {cmds:?}"
    );
}

#[test]
fn setting_set_limiter_ceiling_pushes_limiter_config() {
    let state = shared_with_default_profile();
    let mut audio = attach_filter(&state);

    setting_set("limiter.ceiling_dbtp", json!(-1.5), &state);

    let cmds = drain_audio(&mut audio);
    assert!(
        cmds.iter().any(|cmd| {
            matches!(cmd, AudioCmd::SetLimiter(cfg) if (cfg.ceiling_dbtp - -1.5).abs() < 1e-6)
        }),
        "expected limiter update, got {cmds:?}"
    );
}

#[test]
fn setting_set_default_route_posts_route_reapply() {
    let state = shared_with_default_profile();
    let rx = attach_pw_commands(&state);

    setting_set("default_route.route", json!("bypass"), &state);

    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 1, 1);
}

#[test]
fn setting_set_per_app_enabled_posts_layer_a_reapply() {
    let state = shared_with_default_profile();
    let rx = attach_pw_commands(&state);

    setting_set("per_app.enabled", json!(false), &state);

    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 1, 1);
}

#[test]
fn setting_clear_posts_route_and_layer_a_reapply() {
    let state = shared_with_default_profile();
    setting_set("limiter.ceiling_dbtp", json!(-2.0), &state);
    let rx = attach_pw_commands(&state);

    let _ = ok(dispatch(
        &Request::new(
            2,
            Op::SettingClear {
                key: "limiter.ceiling_dbtp".into(),
            },
        ),
        &state,
    ));

    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 1, 1);
}

#[test]
fn setting_reset_posts_route_and_layer_a_reapply() {
    let state = shared_with_default_profile();
    setting_set("limiter.ceiling_dbtp", json!(-2.0), &state);
    let rx = attach_pw_commands(&state);

    let _ = ok(dispatch(&Request::new(2, Op::SettingReset), &state));

    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 1, 1);
}

#[test]
fn profile_use_posts_dsp_and_reapply() {
    let (state, _guard) = shared_with_night_profile();
    let mut audio = attach_filter(&state);
    let rx = attach_pw_commands(&state);

    let _ = ok(dispatch(
        &Request::new(
            1,
            Op::ProfileUse {
                name: "night".into(),
            },
        ),
        &state,
    ));

    let audio_cmds = drain_audio(&mut audio);
    let pw_cmds = drain_pw(&rx);
    assert!(
        audio_cmds
            .iter()
            .any(|cmd| matches!(cmd, AudioCmd::SetLimiter(_))),
        "expected dsp update, got {audio_cmds:?}"
    );
    assert_reevaluate_counts(&pw_cmds, 1, 1);
}

#[test]
fn profile_reload_posts_dsp_and_reapply() {
    let (state, _guard) = shared_with_night_profile();
    let mut audio = attach_filter(&state);
    let rx = attach_pw_commands(&state);

    let _ = ok(dispatch(&Request::new(1, Op::ProfileReload), &state));

    let audio_cmds = drain_audio(&mut audio);
    let pw_cmds = drain_pw(&rx);
    assert!(
        audio_cmds
            .iter()
            .any(|cmd| matches!(cmd, AudioCmd::SetLimiter(_))),
        "expected dsp update, got {audio_cmds:?}"
    );
    assert_reevaluate_counts(&pw_cmds, 1, 1);
}

#[test]
fn bypass_set_posts_route_reapply() {
    let state = shared_with_default_profile();
    let rx = attach_pw_commands(&state);

    let _ = ok(dispatch(
        &Request::new(1, Op::BypassSet { enabled: true }),
        &state,
    ));

    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 1, 0);
}

#[test]
fn route_set_and_unset_post_route_reapply() {
    let state = shared_with_default_profile();
    let rx = attach_pw_commands(&state);

    let _ = ok(dispatch(
        &Request::new(
            1,
            Op::RouteSet {
                app: "obs".into(),
                to: Route::Bypass,
            },
        ),
        &state,
    ));
    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 1, 0);

    let _ = ok(dispatch(
        &Request::new(2, Op::RouteUnset { app: "obs".into() }),
        &state,
    ));
    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 1, 0);
}

#[test]
fn per_app_set_and_master_post_layer_a_reapply() {
    let state = shared_with_default_profile();
    let rx = attach_pw_commands(&state);

    let _ = ok(dispatch(
        &Request::new(
            1,
            Op::PerAppSet {
                app: "discord".into(),
                enabled: true,
            },
        ),
        &state,
    ));
    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 0, 1);

    let _ = ok(dispatch(
        &Request::new(2, Op::PerAppMaster { enabled: true }),
        &state,
    ));
    let cmds = drain_pw(&rx);
    assert_reevaluate_counts(&cmds, 0, 1);
}

#[test]
fn profile_use_publishes_used_event() {
    let (state, _guard) = shared_with_night_profile();
    let rx = attach_events(&state, &[Topic::Profile]);

    let _ = ok(dispatch(
        &Request::new(
            1,
            Op::ProfileUse {
                name: "night".into(),
            },
        ),
        &state,
    ));

    let events = drain_events(&rx);
    assert!(
        events.iter().any(|event| {
            event.topic == Topic::Profile
                && event.event == "used"
                && event.data["name"] == json!("night")
        }),
        "expected profile used event, got {events:?}"
    );
}

#[test]
fn profile_reload_publishes_reloaded_event() {
    let (state, _guard) = shared_with_night_profile();
    let rx = attach_events(&state, &[Topic::Profile]);

    let _ = ok(dispatch(&Request::new(1, Op::ProfileReload), &state));

    let events = drain_events(&rx);
    assert!(
        events.iter().any(|event| {
            event.topic == Topic::Profile
                && event.event == "reloaded"
                && event.data["loaded"]
                    .as_array()
                    .is_some_and(|loaded| loaded.iter().any(|name| name == "night"))
        }),
        "expected profile reloaded event, got {events:?}"
    );
}

#[test]
fn route_rule_mutations_publish_rule_changed() {
    let state = shared_with_default_profile();
    let rx = attach_events(&state, &[Topic::Routing]);

    let _ = ok(dispatch(
        &Request::new(
            1,
            Op::RouteSet {
                app: "obs".into(),
                to: Route::Bypass,
            },
        ),
        &state,
    ));

    let events = drain_events(&rx);
    assert!(
        events
            .iter()
            .any(|event| event.topic == Topic::Routing && event.event == "rule_changed"),
        "expected route rule change event, got {events:?}"
    );
}
