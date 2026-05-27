use serde_json::json;

use headroom_ipc::{Event, Topic};

use crate::pw::command::PwCommand;
use crate::pw::filter::FilterControl;
use crate::state::DaemonState;

pub(super) struct ProfileEffects {
    event: Option<EffectEvent>,
    dsp: DspEffect,
    commands: &'static [PwCommand],
}

enum EffectEvent {
    ProfileUsed(String),
    ProfileReloaded(Vec<String>),
    RouteRuleChanged,
}

enum DspEffect {
    None,
    PushEffective,
}

pub(super) struct PostProfileEffects {
    dsp: Option<(FilterControl, DspSnapshot)>,
}

const ROUTE_REAPPLY: &[PwCommand] = &[PwCommand::ReevaluateAll];
const LAYER_A_REAPPLY: &[PwCommand] = &[PwCommand::ReevaluateLayerA];
const ROUTE_AND_LAYER_A_REAPPLY: &[PwCommand] =
    &[PwCommand::ReevaluateAll, PwCommand::ReevaluateLayerA];

impl ProfileEffects {
    pub(super) fn profile_used(name: &str) -> Self {
        Self {
            event: Some(EffectEvent::ProfileUsed(name.to_owned())),
            dsp: DspEffect::PushEffective,
            commands: ROUTE_AND_LAYER_A_REAPPLY,
        }
    }

    pub(super) fn profile_reloaded(loaded: Vec<String>) -> Self {
        Self {
            event: Some(EffectEvent::ProfileReloaded(loaded)),
            dsp: DspEffect::PushEffective,
            commands: ROUTE_AND_LAYER_A_REAPPLY,
        }
    }

    pub(super) fn route_rule_changed() -> Self {
        Self {
            event: Some(EffectEvent::RouteRuleChanged),
            dsp: DspEffect::None,
            commands: ROUTE_REAPPLY,
        }
    }

    pub(super) fn settings_changed() -> Self {
        Self {
            event: None,
            dsp: DspEffect::PushEffective,
            commands: ROUTE_AND_LAYER_A_REAPPLY,
        }
    }

    pub(super) fn bypass_changed() -> Self {
        Self {
            event: None,
            dsp: DspEffect::None,
            commands: ROUTE_REAPPLY,
        }
    }

    pub(super) fn per_app_changed() -> Self {
        Self {
            event: Some(EffectEvent::RouteRuleChanged),
            dsp: DspEffect::None,
            commands: LAYER_A_REAPPLY,
        }
    }

    pub(super) fn apply(self, state: &mut DaemonState) -> PostProfileEffects {
        publish(state, self.event);
        let dsp = match self.dsp {
            DspEffect::None => None,
            DspEffect::PushEffective => state
                .filter_control
                .clone()
                .map(|control| (control, DspSnapshot::from(state))),
        };
        for cmd in self.commands {
            post_command(state, cmd);
        }
        PostProfileEffects { dsp }
    }
}

impl PostProfileEffects {
    pub(super) fn finish(self) {
        if let Some((control, snap)) = self.dsp {
            control.set_compressor(snap.compressor);
            control.set_limiter(snap.limiter);
            control.set_agc_enabled(snap.agc_enabled);
        }
    }
}

fn publish(state: &mut DaemonState, event: Option<EffectEvent>) {
    let Some(event) = event else { return };
    let event = match event {
        EffectEvent::ProfileUsed(name) => {
            Event::new(Topic::Profile, "used", &json!({ "name": name }))
        }
        EffectEvent::ProfileReloaded(loaded) => {
            Event::new(Topic::Profile, "reloaded", &json!({ "loaded": loaded }))
        }
        EffectEvent::RouteRuleChanged => Event::new(Topic::Routing, "rule_changed", &json!({})),
    };
    if let Ok(event) = event {
        state.broadcaster.publish(event.topic, event);
    }
}

fn post_command(state: &DaemonState, cmd: &PwCommand) {
    let Some(tx) = state.pw_command_tx.as_ref() else {
        tracing::debug!(?cmd, "no PipeWire command channel; profile effect skipped");
        return;
    };
    if tx.send((*cmd).clone()).is_err() {
        tracing::warn!(?cmd, "PipeWire command channel closed; profile effect lost");
    }
}

struct DspSnapshot {
    compressor: headroom_dsp::CompressorConfig,
    limiter: headroom_dsp::LimiterConfig,
    agc_enabled: bool,
}

impl DspSnapshot {
    fn from(state: &DaemonState) -> Self {
        let effective = state.profiles.effective();
        Self {
            compressor: effective.build_compressor_config(),
            limiter: effective.build_limiter_config(),
            agc_enabled: effective.agc.enabled,
        }
    }
}
