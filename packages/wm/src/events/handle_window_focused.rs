use anyhow::Context;
use tracing::info;
use wm_common::{DisplayState, WindowRuleEvent, WmEvent};
#[cfg(target_os = "windows")]
use wm_platform::DispatcherExtWindows;
use wm_platform::NativeWindow;

#[cfg(target_os = "windows")]
use crate::commands::general::is_shell_focus_target;
use crate::{
  commands::{
    container::set_focused_descendant, window::run_window_rules,
    workspace::focus_workspace,
  },
  models::WorkspaceTarget,
  traits::{CommonGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn handle_window_focused(
  native_window: &NativeWindow,
  state: &mut WmState,
  config: &mut UserConfig,
) -> anyhow::Result<()> {
  let found_window = state.window_from_native(native_window);
  let focused_container =
    state.focused_container().context("No focused container.")?;

  // Update the focus sync state. If the OS focused window is not same as
  // the WM's focused container, then the focus is not synced.
  state.is_focus_synced = match focused_container.as_window_container() {
    Ok(window) => *window.native() == *native_window,
    _ => native_window.is_desktop_window().unwrap_or(false),
  };

  // Handle overriding focus on close/minimize. After a window is closed
  // or minimized, the OS or the closed application might automatically
  // switch focus to a different window. To force focus to go to the WM's
  // target focus container, we reassign any focus events 100ms after
  // close/minimize. This will cause focus to briefly flicker to the OS
  // focus target and then to the WM's focus target.
  if should_override_focus(state) {
    state.pending_sync.queue_focus_change();
    return Ok(());
  }

  // Windows can occasionally leave keyboard focus on the desktop or
  // taskbar even though the WM's focused window is still visible. Restore
  // that window after a short grace period. The delayed check avoids
  // fighting taskbar app activation, Start, dialogs, and other real
  // windows that legitimately take focus.
  #[cfg(target_os = "windows")]
  if config.value.general.restore_focus_on_shell
    && should_restore_shell_focus(native_window, &focused_container, state)
  {
    schedule_shell_focus_restore(&focused_container, state);
    return Ok(());
  }

  // Ignore the focus event if window is being hidden by the WM.
  if let Some(window) = &found_window {
    if window.display_state() == DisplayState::Hiding {
      return Ok(());
    }
  }

  // Focus effect should be updated for any change in focus that shouldn't
  // be overwritten. The incoming focus event at this point is either:
  //  1. WM's focus container (window or workspace). This is the desktop
  //     window in the case of a workspace.
  //  2. An ignored window.
  //  3. A window that received manual focus.
  state.pending_sync.queue_focused_effect_update();

  if let Some(window) = found_window {
    let workspace = window.workspace().context("No workspace")?;

    // Native focus has been synced to the WM's focused container.
    if focused_container == window.clone().into() {
      state.is_focus_synced = true;
      state.pending_sync.queue_workspace_to_reorder(workspace);
      return Ok(());
    }

    info!("Window manually focused: {window}");

    // Handle focus events from windows on hidden workspaces. For example,
    // if Discord is forcefully shown by the OS when it's on a hidden
    // workspace, switch focus to Discord's workspace.
    if window.display_state() == DisplayState::Hidden {
      info!("Focusing off-screen window: {window}");

      focus_workspace(
        WorkspaceTarget::Name(workspace.config().name),
        state,
        config,
      )?;
    }

    // Update the WM's focus state.
    set_focused_descendant(&window.clone().into(), None);

    // Run window rules for focus events.
    run_window_rules(
      window.clone(),
      &WindowRuleEvent::Focus,
      state,
      config,
    )?;

    state.is_focus_synced = true;
    state.pending_sync.queue_workspace_to_reorder(workspace);

    // Broadcast the focus change event.
    state.emit_event(WmEvent::FocusChanged {
      focused_container: window.to_dto()?,
    });
  }

  Ok(())
}

/// Returns true if focus should be reassigned to the WM's focus container.
fn should_override_focus(state: &WmState) -> bool {
  let has_recent_unmanage = state
    .unmanaged_or_minimized_timestamp
    .is_some_and(|time| time.elapsed().as_millis() < 100);

  has_recent_unmanage && !state.is_focus_synced
}

#[cfg(target_os = "windows")]
const SHELL_FOCUS_RESTORE_DELAYS_MS: [u64; 4] = [150, 150, 300, 600];

#[cfg(target_os = "windows")]
fn should_restore_shell_focus(
  native_window: &NativeWindow,
  focused_container: &crate::models::Container,
  state: &WmState,
) -> bool {
  if state.is_paused || !is_shell_focus_target(native_window) {
    return false;
  }

  focused_container.as_window_container().is_ok_and(|window| {
    matches!(
      window.display_state(),
      DisplayState::Shown | DisplayState::Showing
    ) && window.native().is_visible().unwrap_or(false)
      && !window.native().is_minimized().unwrap_or(true)
  })
}

#[cfg(target_os = "windows")]
fn schedule_shell_focus_restore(
  focused_container: &crate::models::Container,
  state: &WmState,
) {
  let Ok(window) = focused_container.as_window_container() else {
    return;
  };

  let target = window.native().clone();
  let dispatcher = state.dispatcher.clone();

  tokio::task::spawn(async move {
    for delay_ms in SHELL_FOCUS_RESTORE_DELAYS_MS {
      tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

      let Ok(current) = dispatcher.focused_window() else {
        return;
      };

      if current == target {
        return;
      }

      // Only restore while focus remains stranded on the shell. If
      // another window, Start, or a dialog takes focus, leave it alone.
      if !is_shell_focus_target(&current)
        || !target.is_valid()
        || !target.is_visible().unwrap_or(false)
        || target.is_minimized().unwrap_or(true)
      {
        return;
      }

      // The taskbar and desktop context menus keep the foreground on the
      // window they belong to, so focus looks stranded on the shell for as
      // long as one is open. Restoring would dismiss the menu mid-use, and
      // the retries run soon enough after the click to catch a menu the
      // user has only just opened. Checked on each pass rather than once
      // up front, since the menu can open after the restore is scheduled.
      if dispatcher.is_menu_open().unwrap_or(false) {
        return;
      }

      info!("Restoring focus from Windows shell to WM-focused window.");
      if let Err(err) = target.focus() {
        tracing::warn!(
          "Failed to restore focus from Windows shell: {err}"
        );
      }
    }
  });
}
