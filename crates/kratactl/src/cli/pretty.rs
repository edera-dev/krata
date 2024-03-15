use krata::common::{GuestState, GuestStatus};

pub fn guest_status_text(status: GuestStatus) -> String {
    match status {
        GuestStatus::Destroy => "destroying",
        GuestStatus::Destroyed => "destroyed",
        GuestStatus::Start => "starting",
        GuestStatus::Exited => "exited",
        GuestStatus::Started => "started",
        _ => "unknown",
    }
    .to_string()
}

pub fn guest_state_text(state: GuestState) -> String {
    let mut text = guest_status_text(state.status());

    if let Some(exit) = state.exit_info {
        text.push_str(&format!(" (exit code: {})", exit.code));
    }

    if let Some(error) = state.error_info {
        text.push_str(&format!(" (error: {})", error.message));
    }
    text
}
