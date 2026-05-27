use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tmp_paths() -> (StorePaths, TempDir) {
    let base = std::env::temp_dir().join(format!(
        "headroom-profile-overlay-{}-{}",
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

#[test]
fn invalid_setting_override_is_preserved_but_skipped() {
    let (paths, _g) = tmp_paths();
    fs::write(
        paths.state_dir.join(OVERLAY_FILE),
        r#"
bypass_global = false
[route_overrides]
[setting_overrides]
"limiter.no_such_field" = 1
"limiter.ceiling_dbtp" = -3.0
"#,
    )
    .unwrap();

    let mut s = ProfileStore::load(&paths).unwrap();
    assert!((s.effective().limiter.ceiling_dbtp - -3.0).abs() < 1e-6);
    assert!(s.setting_overrides().contains_key("limiter.no_such_field"));
    let warnings = s.take_warnings();
    assert!(
        warnings.iter().any(|w| w.contains("limiter.no_such_field")),
        "expected warning for invalid override, got {warnings:?}"
    );
}

#[test]
fn setting_overrides_reports_json_values() {
    let (paths, _g) = tmp_paths();
    let mut s = ProfileStore::load(&paths).unwrap();
    s.set_setting("limiter.ceiling_dbtp", serde_json::json!(-2.5))
        .unwrap();
    s.set_setting("agc.enabled", serde_json::json!(false))
        .unwrap();

    let overrides = s.setting_overrides();
    assert_eq!(overrides["limiter.ceiling_dbtp"], serde_json::json!(-2.5));
    assert_eq!(overrides["agc.enabled"], serde_json::json!(false));
}
