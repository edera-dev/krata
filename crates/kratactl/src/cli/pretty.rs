use krata::common::{GuestState, GuestStatus};

pub fn guest_status_text(status: GuestStatus) -> String {
    match status {
        GuestStatus::Starting => "starting",
        GuestStatus::Started => "started",
        GuestStatus::Destroying => "destroying",
        GuestStatus::Destroyed => "destroyed",
        GuestStatus::Exited => "exited",
        GuestStatus::Failed => "failed",
        _ => "unknown",
    }
    .to_string()
}

pub fn guest_state_text(state: Option<&GuestState>) -> String {
    let state = state.cloned().unwrap_or_default();
    let mut text = guest_status_text(state.status());

    if let Some(exit) = state.exit_info {
        text.push_str(&format!(" (exit code: {})", exit.code));
    }

    if let Some(error) = state.error_info {
        text.push_str(&format!(" (error: {})", error.message));
    }
    text
}
