use super::*;
use headroom_ipc::{RouteList, Sinks, Status};

fn stream(node_id: u32, app: &str, route: Route) -> StreamRoute {
    StreamRoute {
        node_id,
        app: app.into(),
        route,
    }
}

fn state_from(status_streams: Vec<StreamRoute>, route_streams: Vec<StreamRoute>) -> UiState {
    let status = Status {
        version: "test".into(),
        protocol: 1,
        uptime_s: 0,
        profile: "default".into(),
        bypass: false,
        per_app: false,
        sinks: Sinks::default(),
        streams: status_streams,
        layer_a: vec![],
        warnings: vec![],
        setting_overrides: Default::default(),
    };
    let route_list = RouteList {
        rules: vec![],
        current: route_streams,
        default_route: Route::Processed,
    };
    UiState::new(status, route_list)
}

#[test]
fn snapshot_merge_prefers_route_list_and_fills_missing_status_streams() {
    let state = state_from(
        vec![
            stream(1, "status-only", Route::Processed),
            stream(2, "stale-status", Route::Processed),
        ],
        vec![stream(2, "route-list", Route::Bypass)],
    );

    assert_eq!(
        state.streams.get(&1).map(|s| s.route),
        Some(Route::Processed)
    );
    let got = state.streams.get(&2).expect("route list stream");
    assert_eq!(got.app, "route-list");
    assert_eq!(got.route, Route::Bypass);
}

#[test]
fn profile_used_updates_active() {
    let mut state = state_from(vec![], vec![]);
    let ev = Event::new(
        Topic::Profile,
        "used",
        &serde_json::json!({ "name": "movie" }),
    )
    .unwrap();

    state.apply_event(ev);

    assert_eq!(state.profile, "movie");
}
