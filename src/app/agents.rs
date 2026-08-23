use std::time::{Duration, Instant};

use bytes::Bytes;

use super::{terminal_targets::TerminalTargetError, App};
use crate::api::schema::{AgentStartMode, AgentStartParams};

const DEFAULT_AGENT_START_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MAX_AGENT_START_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const AGENT_START_SETTLE_DELAY: Duration = Duration::from_secs(3);
const INVALID_AGENT_TIMEOUT_MESSAGE: &str =
    "agent start timeout must be greater than 3000ms and at most 300000ms";
const INVALID_AGENT_NAME_MESSAGE: &str = "agent name must start with a lowercase letter and contain only lowercase letters, digits, '-' or '_' (1-32 characters)";

fn valid_agent_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && name.len() <= 32
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
}

impl App {
    pub(super) fn collect_agent_infos(&self) -> Vec<crate::api::schema::AgentInfo> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(move |pane_id| self.agent_info(ws_idx, pane_id))
                })
            })
            .collect()
    }

    pub(super) fn reconcile_managed_agent_target(&mut self, target: &str) {
        let Ok(resolved) = self.resolve_agent_target(target) else {
            return;
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(resolved.ws_idx)
            .and_then(|workspace| workspace.terminal_id(resolved.pane_id))
            .cloned()
        else {
            return;
        };
        let changed = self
            .state
            .terminals
            .get_mut(&terminal_id)
            .is_some_and(|terminal| terminal.reconcile_managed_agent_at(Instant::now(), None));
        if changed {
            self.state.mark_session_dirty();
            self.schedule_session_save();
            self.emit_pane_updated(resolved.ws_idx, resolved.pane_id);
        }
    }

    pub(super) fn agent_info_for_target(
        &self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_agent_target(target)?;
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn focus_agent_target(
        &mut self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_agent_target(target)?;
        self.state
            .focus_pane_in_workspace(resolved.ws_idx, resolved.pane_id);
        self.state.mark_active_tab_seen();
        self.state.settle_terminal_mode_after_focus();
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn rename_agent_target(
        &mut self,
        target: &str,
        name: Option<String>,
    ) -> Result<crate::api::schema::AgentInfo, AgentRenameError> {
        let resolved = self
            .resolve_agent_target(target)
            .map_err(AgentRenameError::Target)?;
        let normalized_name = match name {
            Some(name) if valid_agent_name(&name) => Some(name),
            Some(_) => return Err(AgentRenameError::InvalidName),
            None => None,
        };

        if let Some(name) = normalized_name.as_deref() {
            let conflicts = self.agent_name_conflicts(name, &resolved.terminal_id);
            if !conflicts.is_empty() {
                return Err(AgentRenameError::DuplicateName {
                    name: name.to_string(),
                    candidates: conflicts,
                });
            }
        }

        let Some(terminal) = self
            .state
            .terminals
            .values_mut()
            .find(|terminal| terminal.id.to_string() == resolved.terminal_id)
        else {
            return Err(AgentRenameError::Target(TerminalTargetError::NotFound {
                target: target.to_string(),
            }));
        };
        if terminal.managed_agent_launch_pending() {
            return Err(AgentRenameError::PendingLaunch);
        }
        if terminal.effective_agent_label().is_none() {
            return Err(AgentRenameError::NotAgent);
        }
        match normalized_name {
            Some(name) => terminal.set_agent_name(name),
            None => terminal.clear_agent_name(),
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();
        self.emit_pane_updated(resolved.ws_idx, resolved.pane_id);
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| {
                AgentRenameError::Target(TerminalTargetError::NotFound {
                    target: target.to_string(),
                })
            })
    }

    pub(super) fn start_agent(
        &mut self,
        params: AgentStartParams,
    ) -> Result<(crate::api::schema::AgentInfo, Vec<String>), AgentStartError> {
        if params.mode == AgentStartMode::Resume {
            return self.resume_agent(params);
        }
        let name = params.name;
        let task_provenance = params.task_provenance;
        if !valid_agent_name(&name) {
            return Err(AgentStartError::InvalidName);
        }
        let Some(kind) = crate::detect::parse_agent_label(&params.kind) else {
            return Err(AgentStartError::UnsupportedKind(params.kind));
        };
        if params
            .args
            .iter()
            .any(|arg| arg.chars().any(char::is_control))
        {
            return Err(AgentStartError::InvalidArgument);
        }
        let conflicts = self.agent_name_conflicts(&name, "");
        if !conflicts.is_empty() {
            return Err(AgentStartError::DuplicateName {
                name,
                candidates: conflicts,
            });
        }
        let Some((ws_idx, pane_id)) = self.parse_current_public_pane_id(&params.pane_id) else {
            return Err(AgentStartError::TargetNotFound(params.pane_id));
        };
        let terminal_id = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|workspace| workspace.terminal_id(pane_id))
            .cloned()
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        let terminal = self
            .state
            .terminals
            .get(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        if terminal.is_agent_terminal() || terminal.managed_agent_kind().is_some() {
            return Err(AgentStartError::TargetBusy(params.pane_id));
        }
        let runtime = self
            .terminal_runtimes
            .get(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetUnavailable(params.pane_id.clone()))?;
        let shell_name = available_shell_name(runtime)
            .ok_or_else(|| AgentStartError::TargetBusy(params.pane_id.clone()))?;

        let mut argv = vec![crate::detect::interactive_agent_executable(kind).to_string()];
        argv.extend(params.args);
        let command = crate::platform::interactive_shell_command(&argv, &shell_name)
            .ok_or(AgentStartError::InvalidArgument)?;
        let bytes = crate::app::api_helpers::encode_api_submission(runtime, &command);
        let timeout = Duration::from_millis(
            params
                .timeout_ms
                .unwrap_or(DEFAULT_AGENT_START_TIMEOUT.as_millis() as u64),
        );
        if timeout <= AGENT_START_SETTLE_DELAY || timeout > MAX_AGENT_START_TIMEOUT {
            return Err(AgentStartError::InvalidTimeout);
        }

        let now = Instant::now();
        let terminal = self
            .state
            .terminals
            .get_mut(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetUnavailable(params.pane_id.clone()))?;
        let Some(generation) = terminal.begin_managed_agent_with_argv(
            name.clone(),
            kind,
            argv.clone(),
            now,
            AGENT_START_SETTLE_DELAY,
            timeout,
        ) else {
            return Err(AgentStartError::TargetUnavailable(params.pane_id));
        };
        terminal.set_task_provenance(task_provenance);
        runtime.set_managed_agent_generation(generation);
        if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
            terminal.clear_agent_name();
            terminal.clear_task_provenance();
            return Err(AgentStartError::InputFailed(err.to_string()));
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();

        let agent = self
            .agent_info(ws_idx, pane_id)
            .ok_or(AgentStartError::TargetUnavailable(params.pane_id))?;
        self.emit_pane_updated(ws_idx, pane_id);
        Ok((agent, argv))
    }

    fn resume_agent(
        &mut self,
        params: AgentStartParams,
    ) -> Result<(crate::api::schema::AgentInfo, Vec<String>), AgentStartError> {
        if !valid_agent_name(&params.name) {
            return Err(AgentStartError::InvalidName);
        }
        let Some(kind) = crate::detect::parse_agent_label(&params.kind) else {
            return Err(AgentStartError::UnsupportedKind(params.kind));
        };
        if !params.args.is_empty() {
            return Err(AgentStartError::InvalidArgument);
        }
        let Some((ws_idx, pane_id)) = self.parse_current_public_pane_id(&params.pane_id) else {
            return Err(AgentStartError::TargetNotFound(params.pane_id));
        };
        let terminal_id = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|workspace| workspace.terminal_id(pane_id))
            .cloned()
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        let terminal = self
            .state
            .terminals
            .get(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        if terminal.managed_agent_name() != Some(params.name.as_str())
            || terminal.managed_agent_kind() != Some(kind)
        {
            return Err(AgentStartError::DefinitionMismatch);
        }
        match terminal.managed_agent_lifecycle() {
            Some(
                crate::terminal::ManagedAgentLifecycle::Stopped
                | crate::terminal::ManagedAgentLifecycle::StartFailed,
            ) => {}
            Some(
                crate::terminal::ManagedAgentLifecycle::Starting
                | crate::terminal::ManagedAgentLifecycle::Running,
            ) => return Err(AgentStartError::TargetBusy(params.pane_id)),
            None => return Err(AgentStartError::DefinitionMismatch),
        }
        let session = terminal
            .persisted_agent_session
            .clone()
            .ok_or(AgentStartError::MissingSession)?;
        if crate::detect::parse_agent_label(&session.agent) != Some(kind) {
            return Err(AgentStartError::InvalidSession);
        }
        let plan = crate::agent_resume::plan(&session.source, &session.agent, &session.session_ref)
            .ok_or(AgentStartError::InvalidSession)?;
        if !crate::agent_resume::is_available(&session.session_ref) {
            return Err(AgentStartError::MissingSession);
        }
        if self.state.terminals.iter().any(|(other_id, other)| {
            other_id != &terminal_id
                && other
                    .persisted_agent_session
                    .as_ref()
                    .is_some_and(|other_session| {
                        crate::agent_resume::dedupe_key(
                            &other_session.source,
                            &other_session.agent,
                            &other_session.session_ref,
                        ) == plan.dedupe_key
                    })
                && (self.terminal_runtimes.get(other_id).is_some()
                    || matches!(
                        other.managed_agent_lifecycle(),
                        Some(
                            crate::terminal::ManagedAgentLifecycle::Starting
                                | crate::terminal::ManagedAgentLifecycle::Running
                        )
                    ))
        }) {
            return Err(AgentStartError::DuplicateSession);
        }
        let timeout = Duration::from_millis(
            params
                .timeout_ms
                .unwrap_or(DEFAULT_AGENT_START_TIMEOUT.as_millis() as u64),
        );
        if timeout <= AGENT_START_SETTLE_DELAY || timeout > MAX_AGENT_START_TIMEOUT {
            return Err(AgentStartError::InvalidTimeout);
        }
        let live_shell_submission = self
            .terminal_runtimes
            .get(&terminal_id)
            .map(|runtime| {
                let shell_name = available_shell_name(runtime)
                    .ok_or_else(|| AgentStartError::TargetBusy(params.pane_id.clone()))?;
                let command = crate::platform::interactive_shell_command(&plan.argv, &shell_name)
                    .ok_or(AgentStartError::InvalidArgument)?;
                Ok(crate::app::api_helpers::encode_api_submission(
                    runtime, &command,
                ))
            })
            .transpose()?;
        let (rows, cols) = self
            .state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == pane_id)
            .map(|info| (info.inner_rect.height, info.inner_rect.width))
            .unwrap_or_else(|| self.state.estimate_pane_size());
        let cwd = self.state.terminals[&terminal_id].cwd.clone();
        let Some(generation) = self
            .state
            .terminals
            .get_mut(&terminal_id)
            .expect("resolved terminal should remain available")
            .begin_managed_agent_with_argv(
                params.name,
                kind,
                plan.argv.clone(),
                Instant::now(),
                AGENT_START_SETTLE_DELAY,
                timeout,
            )
        else {
            return Err(AgentStartError::ResumeFailed);
        };
        let launched = if let Some(bytes) = live_shell_submission {
            let runtime = self
                .terminal_runtimes
                .get(&terminal_id)
                .expect("live shell runtime should remain available");
            runtime.set_managed_agent_generation(generation);
            runtime.try_send_bytes(Bytes::from(bytes)).is_ok()
        } else {
            self.start_pending_agent_resume(
                pane_id,
                terminal_id.clone(),
                cwd,
                plan.clone(),
                rows,
                cols,
                true,
                Some(generation),
            )
        };
        if !launched {
            if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
                terminal.report_managed_agent_lifecycle(
                    generation,
                    crate::terminal::ManagedAgentLifecycle::StartFailed,
                );
            }
            return Err(AgentStartError::ResumeFailed);
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();
        let agent = self
            .agent_info(ws_idx, pane_id)
            .ok_or(AgentStartError::TargetUnavailable(params.pane_id))?;
        Ok((agent, plan.argv))
    }

    pub(super) fn agent_start_error_body(
        &self,
        err: AgentStartError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentStartError::InvalidName => crate::api::schema::ErrorBody {
                code: "invalid_agent_name".into(),
                message: INVALID_AGENT_NAME_MESSAGE.into(),
            },
            AgentStartError::UnsupportedKind(kind) => crate::api::schema::ErrorBody {
                code: "unsupported_agent_kind".into(),
                message: format!("unsupported interactive agent kind {kind}"),
            },
            AgentStartError::InvalidArgument => crate::api::schema::ErrorBody {
                code: "invalid_agent_argument".into(),
                message: "agent arguments cannot be encoded safely for the target shell".into(),
            },
            AgentStartError::InvalidTimeout => crate::api::schema::ErrorBody {
                code: "invalid_agent_timeout".into(),
                message: INVALID_AGENT_TIMEOUT_MESSAGE.into(),
            },
            AgentStartError::TargetNotFound(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_not_found".into(),
                message: format!("agent target pane {target} not found"),
            },
            AgentStartError::TargetBusy(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_busy".into(),
                message: format!("agent target pane {target} is not an available shell"),
            },
            AgentStartError::TargetUnavailable(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_unavailable".into(),
                message: format!("agent target pane {target} has no live terminal"),
            },
            AgentStartError::InputFailed(message) => crate::api::schema::ErrorBody {
                code: "agent_start_input_failed".into(),
                message,
            },
            AgentStartError::DefinitionMismatch => crate::api::schema::ErrorBody {
                code: "agent_resume_definition_mismatch".into(),
                message: "resume name and kind must match the stopped managed agent".into(),
            },
            AgentStartError::MissingSession => crate::api::schema::ErrorBody {
                code: "agent_resume_session_missing".into(),
                message: "managed agent session reference is missing or unreadable".into(),
            },
            AgentStartError::InvalidSession => crate::api::schema::ErrorBody {
                code: "agent_resume_session_invalid".into(),
                message: "managed agent session reference is invalid".into(),
            },
            AgentStartError::DuplicateSession => crate::api::schema::ErrorBody {
                code: "agent_session_active".into(),
                message: "managed agent session is already active in another pane".into(),
            },
            AgentStartError::ResumeFailed => crate::api::schema::ErrorBody {
                code: "agent_resume_failed".into(),
                message: "failed to launch the stopped managed agent".into(),
            },
            AgentStartError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    pub(super) fn agent_target_error_body(
        &self,
        err: TerminalTargetError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            TerminalTargetError::NotFound { target } => crate::api::schema::ErrorBody {
                code: "agent_not_found".into(),
                message: format!("agent target {target} not found"),
            },
            TerminalTargetError::Ambiguous { target, candidates } => {
                crate::api::schema::ErrorBody {
                    code: "agent_target_ambiguous".into(),
                    message: format!(
                        "agent target {target} is ambiguous; candidates: {}",
                        candidates
                            .into_iter()
                            .map(|candidate| format!(
                                "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                                candidate.terminal_id,
                                candidate.pane_id,
                                candidate.workspace_id,
                                candidate.tab_id,
                                candidate.cwd.unwrap_or_else(|| "unknown".into()),
                                candidate.agent_status,
                            ))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                }
            }
        }
    }

    pub(super) fn agent_rename_error_body(
        &self,
        err: AgentRenameError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentRenameError::Target(err) => self.agent_target_error_body(err),
            AgentRenameError::InvalidName => crate::api::schema::ErrorBody {
                code: "invalid_agent_name".into(),
                message: INVALID_AGENT_NAME_MESSAGE.into(),
            },
            AgentRenameError::NotAgent => crate::api::schema::ErrorBody {
                code: "agent_not_found".into(),
                message: "agent target does not currently host an agent".into(),
            },
            AgentRenameError::PendingLaunch => crate::api::schema::ErrorBody {
                code: "agent_launch_pending".into(),
                message: "agent name cannot change while startup is pending".into(),
            },
            AgentRenameError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    pub(super) fn agent_info(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<crate::api::schema::AgentInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane_state = ws.pane_state(pane_id)?;
        let terminal = self.state.terminals.get(&pane_state.attached_terminal_id)?;
        if !terminal.is_agent_terminal() {
            return None;
        }
        let pane = self.pane_info(ws_idx, pane_id)?;
        Some(crate::api::schema::AgentInfo {
            terminal_id: pane.terminal_id,
            name: terminal.agent_name.clone(),
            agent: pane.agent,
            title: pane.title,
            terminal_title: pane.terminal_title,
            terminal_title_stripped: pane.terminal_title_stripped,
            display_agent: pane.display_agent,
            agent_status: pane.agent_status,
            screen_detection_skipped: terminal.full_lifecycle_hook_authority_active(),
            state_labels: pane.state_labels,
            tokens: pane.tokens,
            agent_session: pane.agent_session,
            task_provenance: pane.task_provenance,
            workspace_id: pane.workspace_id,
            tab_id: pane.tab_id,
            pane_id: pane.pane_id,
            focused: pane.focused,
            launch_pending: terminal.managed_agent_launch_pending(),
            interactive_ready: terminal.managed_agent_interactive_ready(),
            process_lifecycle: terminal.managed_agent_lifecycle(),
            launch_generation: terminal.managed_agent_generation(),
            state_change_seq: terminal.last_agent_state_change_seq.unwrap_or(0),
            cwd: pane.cwd,
            foreground_cwd: pane.foreground_cwd,
            revision: pane.revision,
        })
    }

    fn agent_name_conflicts(
        &self,
        name: &str,
        except_terminal_id: &str,
    ) -> Vec<crate::api::schema::AgentInfo> {
        self.collect_agent_infos()
            .into_iter()
            .filter(|agent| {
                agent.name.as_deref() == Some(name) && agent.terminal_id != except_terminal_id
            })
            .collect()
    }
}

fn available_shell_name(runtime: &crate::terminal::TerminalRuntime) -> Option<String> {
    #[cfg(test)]
    if runtime.child_pid().is_none() {
        return Some("sh".into());
    }
    crate::platform::available_pane_shell(runtime.child_pid()?)
}

pub(super) fn runtime_hosts_agent(
    runtime: &crate::terminal::TerminalRuntime,
    expected: crate::detect::Agent,
) -> bool {
    #[cfg(test)]
    if runtime.child_pid().is_none() {
        return true;
    }
    live_runtime_agent(runtime) == Some(expected)
}

fn live_runtime_agent(runtime: &crate::terminal::TerminalRuntime) -> Option<crate::detect::Agent> {
    let job = crate::detect::foreground_job(runtime.child_pid()?)?;
    crate::detect::identify_agent_in_job(&job)
        .map(|(agent, _)| agent)
        .or_else(|| {
            job.processes
                .iter()
                .find_map(|process| crate::platform::process_agent_hint(process.pid))
        })
}

pub(super) enum AgentStartError {
    InvalidName,
    UnsupportedKind(String),
    InvalidArgument,
    InvalidTimeout,
    TargetNotFound(String),
    TargetBusy(String),
    TargetUnavailable(String),
    InputFailed(String),
    DefinitionMismatch,
    MissingSession,
    InvalidSession,
    DuplicateSession,
    ResumeFailed,
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

pub(super) enum AgentRenameError {
    Target(TerminalTargetError),
    InvalidName,
    NotAgent,
    PendingLaunch,
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

#[cfg(test)]
mod tests {
    use super::valid_agent_name;

    #[test]
    fn agent_names_use_a_small_cli_safe_grammar() {
        for name in ["a", "reviewer-one", "reviewer_2", &"a".repeat(32)] {
            assert!(valid_agent_name(name), "expected {name:?} to be valid");
        }
        for name in [
            "",
            " reviewer",
            "reviewer ",
            "reviewer one",
            "Reviewer",
            "1reviewer",
            "reviewer.one",
            &"a".repeat(33),
        ] {
            assert!(!valid_agent_name(name), "expected {name:?} to be invalid");
        }
    }
}
