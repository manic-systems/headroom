use crossbeam_channel::unbounded;
use headroom_ipc::{RouteList, Sinks, Status};

use super::*;

fn stream(node_id: u32, app: &str, route: Route) -> StreamRoute {
    StreamRoute {
        node_id,
        app: app.into(),
        route,
    }
}

fn status_with(streams: Vec<StreamRoute>) -> Status {
    Status {
        version: "test".into(),
        protocol: 1,
        uptime_s: 0,
        profile: "default".into(),
        bypass: false,
        per_app: false,
        sinks: Sinks::default(),
        streams,
        layer_a: vec![],
        warnings: vec![],
        setting_overrides: Default::default(),
    }
}

fn app_from_snapshot(
    status_streams: Vec<StreamRoute>,
    route_streams: Vec<StreamRoute>,
) -> HeadroomApp {
    let (_tx, rx) = unbounded();
    let (cmd_tx, _cmd_rx) = unbounded();
    HeadroomApp::new(Bootstrap {
        snapshot: Snapshot {
            status: status_with(status_streams),
            route_list: RouteList {
                rules: vec![],
                current: route_streams,
                default_route: Route::Processed,
            },
            profiles: vec![],
        },
        rx,
        cmd_tx,
    })
}

#[test]
fn snapshot_merge_prefers_route_list_and_fills_missing_status_streams() {
    let s = app_from_snapshot(
        vec![
            stream(1, "status-only", Route::Processed),
            stream(2, "stale-status", Route::Processed),
        ],
        vec![stream(2, "route-list", Route::Bypass)],
    );

    assert_eq!(s.streams.get(&1).map(|r| r.route), Some(Route::Processed));
    let got = s.streams.get(&2).expect("route list stream");
    assert_eq!(got.app, "route-list");
    assert_eq!(got.route, Route::Bypass);
}
