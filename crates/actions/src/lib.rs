//! Native (built-in) actions for RiverDeck-Redux.
//!
//! This crate defines:
//! - The serializable action model (`ActionBinding`, `BuiltinAction`)
//! - A lightweight executor that expands bindings (e.g., Macro) into a linear
//!   sequence of `ActionStep`s that the UI/runtime can execute.

use serde::{Deserialize, Serialize};

/// A plugin action binding (OpenAction-style), matching existing on-disk profiles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginActionBinding {
    pub plugin_id: String,
    pub action_id: String,
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// A native action binding (built-in), stored inside profiles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "builtin", rename_all = "snake_case")]
pub enum BuiltinAction {
    /// Runs multiple actions in order.
    Macro { steps: Vec<MacroStep> },
    /// Runs a shell command.
    IssueCommand {
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Sends keyboard input.
    ///
    /// Linux MVP: uses an external tool configured by the host app.
    /// - If `text` is set, it will be typed.
    /// - If `keys` is set, a chord/sequence will be sent (tool-dependent).
    KeyboardInput {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        keys: Vec<String>,
    },
    /// Play an audio file.
    PlaySound { path: String },
    /// Switch to a specific profile or cycle.
    SwitchProfile { mode: SwitchProfileMode },
    /// Adjust device brightness.
    DeviceBrightness { mode: BrightnessMode },
    /// Live system monitoring (display in UI first).
    SystemMonitoring {
        kind: MonitorKind,
        #[serde(default)]
        refresh_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroStep {
    pub action: Box<ActionBinding>,
    #[serde(default)]
    pub delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SwitchProfileMode {
    /// Switch to a specific profile ID (as u64).
    ///
    /// Stored as u64 to keep this crate independent of storage/app-core types.
    To { profile_id: u64 },
    Next,
    Prev,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BrightnessMode {
    Set { percent: u8 },
    Increase { delta: u8 },
    Decrease { delta: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MonitorKind {
    Cpu,
    Memory,
    LoadAverage,
}

/// A key binding can point to either a plugin action or a builtin action.
///
/// Backwards compatible with existing plugin-only profiles because the plugin
/// variant has the same shape as the previous `storage::profiles::ActionBinding`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ActionBinding {
    Plugin(PluginActionBinding),
    Builtin(BuiltinAction),
}

#[derive(Debug, Clone)]
pub enum ActionStep {
    DelayMs(u64),
    Builtin(BuiltinAction),
    Plugin(PluginActionBinding),
}

#[derive(Debug, thiserror::Error)]
pub enum ExpandError {
    #[error("macro step count exceeded limit ({0})")]
    MacroTooLarge(usize),
}

/// Expands an `ActionBinding` into a linear sequence of executable steps.
///
/// - Macro steps are expanded depth-first.
/// - A hard limit prevents runaway recursion.
pub fn expand(binding: &ActionBinding) -> Result<Vec<ActionStep>, ExpandError> {
    const MAX_STEPS: usize = 128;

    fn push_binding(
        out: &mut Vec<ActionStep>,
        b: &ActionBinding,
        steps: &mut usize,
    ) -> Result<(), ExpandError> {
        if *steps >= MAX_STEPS {
            return Err(ExpandError::MacroTooLarge(MAX_STEPS));
        }

        match b {
            ActionBinding::Plugin(p) => {
                out.push(ActionStep::Plugin(p.clone()));
                *steps += 1;
            }
            ActionBinding::Builtin(BuiltinAction::Macro { steps: macro_steps }) => {
                for s in macro_steps {
                    if let Some(d) = s.delay_ms {
                        out.push(ActionStep::DelayMs(d));
                        *steps += 1;
                        if *steps >= MAX_STEPS {
                            return Err(ExpandError::MacroTooLarge(MAX_STEPS));
                        }
                    }
                    push_binding(out, &s.action, steps)?;
                }
            }
            ActionBinding::Builtin(bi) => {
                out.push(ActionStep::Builtin(bi.clone()));
                *steps += 1;
            }
        }

        Ok(())
    }

    let mut out = Vec::new();
    let mut steps = 0;
    push_binding(&mut out, binding, &mut steps)?;
    Ok(out)
}


