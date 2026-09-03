//! Recovery for keyboard focus stranded on the Windows shell.
//!
//! `handle_window_focused` restores focus when a foreground event tells us
//! the shell took it. Some transitions never produce that event: when a
//! taskbar flyout (Widgets, Search, the wifi/audio/battery quick settings)
//! is dismissed, Windows either drops the foreground entirely or leaves it
//! on the flyout's now-cloaked host window. `EVENT_SYSTEM_FOREGROUND` does
//! not fire for either, so focus sits nowhere until the user clicks a real
//! window. This polls for that state instead of waiting for an event that
//! never arrives.

#[cfg(target_os = "windows")]
use tracing::info;
#[cfg(target_os = "windows")]
use wm_common::DisplayState;
#[cfg(target_os = "windows")]
use wm_platform::{
  DispatcherExtWindows, NativeWindow, NativeWindowWindowsExt,
};

#[cfg(target_os = "windows")]
use crate::traits::{CommonGetters, WindowGetters};
use crate::{user_config::UserConfig, wm_state::WmState};

/// Number of consecutive polls that must observe stranded focus before it
/// is restored. Debouncing avoids racing the brief foreground gaps that
/// occur during normal window activation.
#[cfg(target_os = "windows")]
const STRANDED_FOCUS_TICKS_BEFORE_RESTORE: u32 = 2;

/// Restores attempted for one stranded stretch before giving up.
///
/// Windows grants foreground rights to the process that last received
/// input. When the shell refuses `SetForegroundWindow` it keeps refusing
/// until the user interacts, so retrying forever cannot recover focus and
/// sends an input event every couple of seconds to no purpose.
#[cfg(target_os = "windows")]
const STRANDED_FOCUS_MAX_ATTEMPTS: u32 = 3;

/// Restores focus to the WM's focused window when the OS has left keyboard
/// focus stranded on the shell.
///
/// A failed restore is logged rather than propagated, but the fallible
/// signature is kept so this matches the other arms of the main event
/// loop.
#[cfg(target_os = "windows")]
#[allow(clippy::unnecessary_wraps)]
pub fn reconcile_stranded_focus(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  if state.is_paused || !config.value.general.restore_focus_on_shell {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  // A locked session has no user to hand focus back to, and the lock
  // screen legitimately holds the foreground, so every poll would read
  // as stranded. Restoring focus calls `SendInput` to defeat the
  // foreground lock, which Windows counts as user input and uses to
  // wake the display -- polling that against a lock screen turns the
  // display back on within seconds of it going off, indefinitely.
  //
  // A failed query is treated as unlocked so that a broken lock check
  // degrades to the previous behaviour rather than disabling recovery.
  if state.dispatcher.is_session_locked().unwrap_or(false) {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  // An open menu holds the foreground on the window that owns it, and that
  // owner is typically an invisible message-only window -- a tray icon's
  // window, or the hidden owner behind the Win+X menu. That reads as
  // stranded focus by every test below, so without this the poll restores
  // focus out from under a menu the user is working their way through,
  // roughly a second and a half after it opens. Restoring focus also sends
  // input to defeat the foreground lock, which dismisses the menu
  // outright.
  //
  // A failed query is treated as no menu so that a broken check degrades
  // to the previous behaviour rather than disabling recovery.
  if state.dispatcher.is_menu_open().unwrap_or(false) {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  // Only restore to a window the user can actually see. A hidden or
  // minimized target means there is nothing to return focus to.
  let target = state
    .focused_container()
    .and_then(|container| container.as_window_container().ok())
    .filter(|window| {
      matches!(
        window.display_state(),
        DisplayState::Shown | DisplayState::Showing
      )
    })
    .map(|window| window.native().clone())
    .filter(|native| {
      native.is_visible().unwrap_or(false)
        && !native.is_minimized().unwrap_or(true)
    });

  let (Some(target), Ok(current)) =
    (target, state.dispatcher.focused_window())
  else {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  };

  if current == target || !is_stranded_focus_target(&current) {
    state.stranded_focus_ticks = 0;
    state.stranded_focus_attempts = 0;
    return Ok(());
  }

  state.stranded_focus_ticks += 1;
  if state.stranded_focus_ticks < STRANDED_FOCUS_TICKS_BEFORE_RESTORE {
    return Ok(());
  }

  state.stranded_focus_ticks = 0;

  // Stop once the shell has refused often enough to show it will keep
  // refusing. The counter is cleared as soon as focus lands anywhere
  // real, so a later stranding gets a fresh set of attempts.
  if state.stranded_focus_attempts >= STRANDED_FOCUS_MAX_ATTEMPTS {
    return Ok(());
  }

  state.stranded_focus_attempts += 1;
  info!("Restoring focus stranded on the Windows shell.");

  match target.focus() {
    Ok(()) => state.stranded_focus_attempts = 0,
    Err(err) => {
      tracing::warn!("Failed to restore stranded focus: {err}");

      if state.stranded_focus_attempts >= STRANDED_FOCUS_MAX_ATTEMPTS {
        tracing::warn!(
          "Giving up on stranded focus after {STRANDED_FOCUS_MAX_ATTEMPTS} attempts; the shell holds the foreground until there is user input."
        );
      }
    }
  }

  Ok(())
}

#[cfg(not(target_os = "windows"))]
#[allow(clippy::unnecessary_wraps)]
pub fn reconcile_stranded_focus(
  _state: &mut WmState,
  _config: &UserConfig,
) -> anyhow::Result<()> {
  Ok(())
}

/// Whether keyboard focus currently sits somewhere the user cannot type.
///
/// This is deliberately wider than [`is_shell_focus_target`]: a dismissed
/// taskbar flyout can remain the foreground window while cloaked.
/// `is_visible` already accounts for cloaking, so an *open* flyout is
/// visible and uncloaked and never matches here — it keeps its focus and
/// stays usable.
#[cfg(target_os = "windows")]
pub(crate) fn is_stranded_focus_target(
  native_window: &NativeWindow,
) -> bool {
  is_shell_focus_target(native_window)
    || !native_window.is_visible().unwrap_or(true)
}

/// Whether a focus event target is the desktop or the taskbar frame.
#[cfg(target_os = "windows")]
pub(crate) fn is_shell_focus_target(native_window: &NativeWindow) -> bool {
  native_window.hwnd().0 == 0
    || native_window.is_desktop_window().unwrap_or(false)
    || native_window
      .class_name()
      .is_ok_and(|class_name| is_taskbar_window_class(&class_name))
}

#[cfg(any(test, target_os = "windows"))]
pub(crate) fn is_taskbar_window_class(class_name: &str) -> bool {
  matches!(class_name, "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

#[cfg(test)]
mod tests {
  use super::is_taskbar_window_class;

  #[test]
  fn recognizes_only_taskbar_window_classes() {
    assert!(is_taskbar_window_class("Shell_TrayWnd"));
    assert!(is_taskbar_window_class("Shell_SecondaryTrayWnd"));
    assert!(!is_taskbar_window_class("Progman"));

    // The flyout hosts for Widgets, Search, and quick settings share this
    // class with the Start menu. Matching it would steal focus from a
    // flyout the user is actively typing into; the stranded-focus poll
    // recovers them after dismissal instead.
    assert!(!is_taskbar_window_class("Windows.UI.Core.CoreWindow"));
  }
}
